use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};

const EMPTY_AUDIO_LEVEL: f32 = f32::NEG_INFINITY;
pub type AudioSink = mpsc::Sender<AudioChunk>;
#[derive(Clone)]
pub struct AudioLevelMeter {
    level_bits: Arc<AtomicU32>,
    active: Arc<AtomicBool>,
}

impl AudioLevelMeter {
    pub fn new() -> Self {
        Self {
            level_bits: Arc::new(AtomicU32::new(EMPTY_AUDIO_LEVEL.to_bits())),
            active: Arc::new(AtomicBool::new(true)),
        }
    }

    pub(super) fn publish(&self, level: f32) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let level = level.clamp(-96.0, 0.0);
        let mut current = self.level_bits.load(Ordering::Relaxed);
        while self.active.load(Ordering::Acquire) && f32::from_bits(current) < level {
            match self.level_bits.compare_exchange_weak(
                current,
                level.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(next) => current = next,
            }
        }
        if !self.active.load(Ordering::Acquire) {
            self.level_bits
                .store(EMPTY_AUDIO_LEVEL.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn take_level(&self) -> Option<f32> {
        let level = f32::from_bits(
            self.level_bits
                .swap(EMPTY_AUDIO_LEVEL.to_bits(), Ordering::Relaxed),
        );
        level.is_finite().then_some(level)
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub fn stop(&self) {
        self.active.store(false, Ordering::Release);
        self.level_bits
            .store(EMPTY_AUDIO_LEVEL.to_bits(), Ordering::Relaxed);
    }
}

impl Default for AudioLevelMeter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone)]
pub struct RecordedAudio {
    pub id: String,
    pub path: PathBuf,
    pub sample_rate: u32,
    pub channels: u16,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub sample_count: usize,
}

impl RecordedAudio {
    pub(crate) fn discard(self) -> Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => {
                Err(err).with_context(|| format!("Failed to discard {}", self.path.display()))
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AudioTrim {
    pub last_activity_ms: u64,
}
