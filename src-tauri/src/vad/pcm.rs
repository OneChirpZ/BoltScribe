use crate::recorder::AudioChunk;
use anyhow::{anyhow, Result};

const TARGET_SAMPLE_RATE: u32 = 16_000;

#[derive(Default)]
pub(super) struct PcmConverter {
    sample_rate: Option<u32>,
    channels: Option<u16>,
    pending_mono: Vec<i16>,
    next_src_pos: f64,
}

impl PcmConverter {
    pub(super) fn push_chunk(&mut self, chunk: &AudioChunk) -> Result<Vec<i16>> {
        if chunk.samples.is_empty() {
            return Ok(Vec::new());
        }
        if chunk.sample_rate == 0 || chunk.channels == 0 {
            return Err(anyhow!("Invalid VAD audio format"));
        }
        match (self.sample_rate, self.channels) {
            (None, None) => {
                self.sample_rate = Some(chunk.sample_rate);
                self.channels = Some(chunk.channels);
            }
            (Some(rate), Some(channels))
                if rate == chunk.sample_rate && channels == chunk.channels => {}
            _ => return Err(anyhow!("VAD audio format changed during recording")),
        }
        self.pending_mono
            .extend(downmix(&chunk.samples, chunk.channels));
        Ok(self.drain(false))
    }

    fn drain(&mut self, final_chunk: bool) -> Vec<i16> {
        let Some(rate) = self.sample_rate else {
            return Vec::new();
        };
        if self.pending_mono.is_empty() {
            return Vec::new();
        }
        let step = rate as f64 / TARGET_SAMPLE_RATE as f64;
        let mut output = Vec::new();
        while self.next_src_pos + 1.0 < self.pending_mono.len() as f64 {
            output.push(interpolate(&self.pending_mono, self.next_src_pos));
            self.next_src_pos += step;
        }
        if final_chunk {
            while self.next_src_pos < self.pending_mono.len() as f64 {
                output.push(interpolate(&self.pending_mono, self.next_src_pos));
                self.next_src_pos += step;
            }
            self.pending_mono.clear();
            self.next_src_pos = 0.0;
        } else {
            let consumed =
                (self.next_src_pos.floor() as usize).min(self.pending_mono.len().saturating_sub(1));
            if consumed > 0 {
                self.pending_mono.drain(..consumed);
                self.next_src_pos -= consumed as f64;
            }
        }
        output
    }
}

fn downmix(samples: &[i16], channels: u16) -> Vec<i16> {
    if channels <= 1 {
        return samples.to_vec();
    }
    samples
        .chunks(channels as usize)
        .map(|frame| {
            let sum = frame.iter().map(|sample| *sample as i32).sum::<i32>();
            (sum / frame.len().max(1) as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
        })
        .collect()
}

fn interpolate(samples: &[i16], position: f64) -> i16 {
    let index = position.floor() as usize;
    let fraction = position - index as f64;
    let first = samples[index] as f64;
    let second = samples.get(index + 1).copied().unwrap_or(samples[index]) as f64;
    (first + (second - first) * fraction)
        .round()
        .clamp(i16::MIN as f64, i16::MAX as f64) as i16
}
