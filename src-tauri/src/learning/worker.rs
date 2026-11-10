use super::{
    activity::{ActivityHandle, ActivityState},
    model, store, validation, LearnedEntry, LearningConfig, LearningSample, LearningStatus,
};
use crate::config::ConfigStore;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const QUEUE_MAX_AGE: Duration = Duration::from_secs(60);
static WORKER: OnceLock<Mutex<WorkerState>> = OnceLock::new();

struct Job {
    app: AppHandle,
    sample: LearningSample,
    config: LearningConfig,
    path: PathBuf,
    generation: u64,
    queued_at: Instant,
}

#[derive(Default)]
struct WorkerState {
    queue: WorkQueue<Job>,
    status: LearningStatus,
}

/// One owner executes a job; at most its newest successor is retained.
struct WorkQueue<T> {
    generation: u64,
    active: bool,
    pending: Option<T>,
}

impl<T> Default for WorkQueue<T> {
    fn default() -> Self {
        Self {
            generation: 0,
            active: false,
            pending: None,
        }
    }
}

impl<T> WorkQueue<T> {
    fn enqueue(&mut self, job: T) -> (Option<T>, Option<T>) {
        if self.active {
            (None, self.pending.replace(job))
        } else {
            self.active = true;
            (Some(job), None)
        }
    }

    fn finish(&mut self) -> Option<T> {
        let next = self.pending.take();
        self.active = next.is_some();
        next
    }

    fn cancel(&mut self) -> Option<T> {
        self.generation = self.generation.wrapping_add(1);
        self.pending.take()
    }
}

fn state() -> &'static Mutex<WorkerState> {
    WORKER.get_or_init(|| Mutex::new(WorkerState::default()))
}

pub fn status() -> LearningStatus {
    let mut status = state()
        .lock()
        .map(|state| state.status.clone())
        .unwrap_or_default();
    status.observation = super::observation::stats();
    status
}

pub fn cancel() {
    if let Ok(mut state) = state().lock() {
        if let Some(pending) = state.queue.cancel() {
            update_activity(
                &pending.sample.activity,
                ActivityState::Skipped,
                Some("cancelled"),
            );
            state.status.skipped += 1;
        }
        state.status.pending = false;
    }
}

pub fn submit(app: AppHandle, sample: LearningSample) {
    // Both sample validation and configuration loading may race cancellation.
    let generation = match state().lock() {
        Ok(state) => state.queue.generation,
        Err(_) => return,
    };
    let rejection = validation::sample_rejection(&sample);
    if matches!(rejection, None | Some("unrelated_edit")) {
        if let (Some(activity), Ok(state)) = (&sample.activity, state().lock()) {
            if generation == state.queue.generation {
                activity.set_details(Some(&sample.current), None);
            }
        }
    }
    if let Some(reason) = rejection {
        update_activity(&sample.activity, ActivityState::Skipped, Some(reason));
        if let Ok(mut state) = state().lock() {
            state.status.skipped += 1;
        }
        return;
    }
    let prepared = (|| -> Result<_> {
        let config = ConfigStore::load()?.learning_model();
        config.validate()?;
        Ok((config, store::path()?))
    })();
    let (config, path) = match prepared {
        Ok((config, path)) if config.can_observe() => (config, path),
        Ok(_) => {
            update_activity(
                &sample.activity,
                ActivityState::Skipped,
                Some("config_changed"),
            );
            return;
        }
        Err(error) => {
            if let Some(activity) = &sample.activity {
                activity.update(
                    ActivityState::Failed,
                    Some("config_changed"),
                    vec![],
                    Some("Learning configuration is unavailable".into()),
                );
            }
            if let Ok(mut state) = state().lock() {
                state.status.last_error = Some(error.to_string());
            }
            let _ = app.emit("learning://updated", ());
            return;
        }
    };
    let Ok(mut state) = state().lock() else {
        return;
    };
    if generation != state.queue.generation
        || sample
            .activity
            .as_ref()
            .is_some_and(|activity| !activity.is_current_active())
    {
        state.status.skipped += 1;
        update_activity(&sample.activity, ActivityState::Skipped, Some("cancelled"));
        return;
    }
    state.status.submitted += 1;
    let job = Job {
        app: app.clone(),
        sample,
        config,
        path,
        generation,
        queued_at: Instant::now(),
    };
    update_activity(&job.sample.activity, ActivityState::Queued, None);
    let (start, replaced) = state.queue.enqueue(job);
    if let Some(replaced) = replaced {
        update_activity(
            &replaced.sample.activity,
            ActivityState::Skipped,
            Some("superseded"),
        );
        state.status.skipped += 1;
    }
    state.status.pending = state.queue.pending.is_some();
    state.status.processing = state.queue.active;
    if let Some(job) = start {
        std::thread::spawn(move || run(job));
    }
    drop(state);
    let _ = app.emit("learning://updated", ());
}

fn update_activity(activity: &Option<ActivityHandle>, state: ActivityState, reason: Option<&str>) {
    if let Some(activity) = activity {
        activity.update(state, reason, vec![], None);
    }
}

enum JobResult {
    Skipped(&'static str),
    Analyzed {
        entries: Vec<LearnedEntry>,
        rejected: usize,
        reason: Option<&'static str>,
    },
}

fn run(mut job: Job) {
    loop {
        let result = process(&job);
        let Ok(mut state) = state().lock() else {
            return;
        };
        match result {
            Ok(JobResult::Skipped(reason)) => {
                state.status.skipped += 1;
                update_activity(&job.sample.activity, ActivityState::Skipped, Some(reason));
            }
            Ok(JobResult::Analyzed {
                entries,
                rejected,
                reason,
            }) => {
                let accepted = entries.len();
                if let Some(activity) = &job.sample.activity {
                    activity.update(
                        if accepted > 0 {
                            ActivityState::Learned
                        } else {
                            ActivityState::NoTerms
                        },
                        reason,
                        entries,
                        None,
                    );
                }
                state.status.analyzed += 1;
                state.status.accepted += accepted as u64;
                state.status.rejected += rejected as u64;
                state.status.last_error = None;
            }
            Err(error) => {
                // Requests never expose the response body, credentials or input text here.
                let message = error.to_string();
                let safe_message = if message.starts_with("Learning model")
                    || message.starts_with("Learning response")
                    || message.starts_with("Cannot read learning model")
                {
                    message.clone()
                } else {
                    "Learning analysis failed".into()
                };
                if let Some(activity) = &job.sample.activity {
                    activity.update(
                        ActivityState::Failed,
                        Some("model_error"),
                        vec![],
                        Some(safe_message),
                    );
                }
                state.status.last_error = Some(message);
            }
        }
        let next = state.queue.finish();
        state.status.pending = false;
        state.status.processing = state.queue.active;
        drop(state);
        let _ = job.app.emit("learning://updated", ());
        let Some(next) = next else {
            return;
        };
        job = next;
    }
}

fn is_current(job: &Job, state: &WorkerState) -> bool {
    job.generation == state.queue.generation
}

fn process(job: &Job) -> Result<JobResult> {
    {
        let state = state()
            .lock()
            .map_err(|_| anyhow::anyhow!("Learning worker is unavailable"))?;
        if !is_current(job, &state) {
            return Ok(JobResult::Skipped("cancelled"));
        }
        if job.queued_at.elapsed() > QUEUE_MAX_AGE {
            return Ok(JobResult::Skipped("queue_expired"));
        }
    }
    let current = ConfigStore::load()?;
    if !current.learning.enabled || current.learning_model() != job.config {
        return Ok(JobResult::Skipped("config_changed"));
    }
    update_activity(&job.sample.activity, ActivityState::Analyzing, None);
    let response = model::analyze(&job.config, &job.sample);
    {
        let state = state()
            .lock()
            .map_err(|_| anyhow::anyhow!("Learning worker is unavailable"))?;
        if !is_current(job, &state) {
            return Ok(JobResult::Skipped("cancelled"));
        }
    }
    let text = response?;
    let (candidates, rejected) = validation::parse_candidates(&text, &job.sample)?;
    let latest = ConfigStore::load()?;
    // Serialize cancellation with the final write. No state/store lock is held during HTTP.
    let state = state()
        .lock()
        .map_err(|_| anyhow::anyhow!("Learning worker is unavailable"))?;
    if !is_current(job, &state) || !latest.learning.enabled || latest.learning_model() != job.config
    {
        return Ok(JobResult::Skipped("config_changed"));
    }
    if let Some(activity) = &job.sample.activity {
        if !activity.is_current_active() {
            return Ok(JobResult::Skipped("cancelled"));
        }
        activity.set_details(None, Some(&text));
    }
    let entries = store::add_candidates(
        &job.path,
        &candidates,
        &latest.correction,
        &job.sample.record_id,
    )?;
    let reason = if !entries.is_empty() {
        None
    } else if !candidates.is_empty() {
        Some("already_known_or_conflict")
    } else if rejected > 0 {
        Some("candidates_rejected")
    } else {
        Some("no_terms")
    };
    Ok(JobResult::Analyzed {
        rejected: rejected + candidates.len() - entries.len(),
        entries,
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bursts_keep_one_running_and_only_the_latest_pending_job() {
        let mut queue = WorkQueue::default();
        assert_eq!(queue.enqueue(1), (Some(1), None));
        assert_eq!(queue.enqueue(2), (None, None));
        assert_eq!(queue.enqueue(3), (None, Some(2)));
        assert_eq!(queue.finish(), Some(3));
        assert!(queue.active);
        assert_eq!(queue.finish(), None);
        assert!(!queue.active);
    }

    #[test]
    fn cancel_discards_pending_and_new_generation_waits_for_inflight_owner() {
        let mut queue = WorkQueue::default();
        let old_generation = queue.generation;
        assert_eq!(queue.enqueue(1), (Some(1), None));
        queue.enqueue(2);
        assert_eq!(queue.cancel(), Some(2));
        assert_ne!(old_generation, queue.generation);
        assert_eq!(queue.enqueue(3), (None, None));
        assert_eq!(queue.finish(), Some(3));
        assert_eq!(queue.finish(), None);
    }
}
