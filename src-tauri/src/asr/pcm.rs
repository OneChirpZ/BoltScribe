use crate::recorder::AudioChunk;
use anyhow::{bail, Result};
use std::path::Path;

pub(super) const TARGET_SAMPLE_RATE: u32 = 16_000;

#[derive(Default)]
pub(super) struct StreamingPcmConverter {
    sample_rate: Option<u32>,
    channels: Option<u16>,
    pending_mono: Vec<i16>,
    next_src_pos: f64,
}

impl StreamingPcmConverter {
    pub(super) fn push_chunk(&mut self, chunk: &AudioChunk) -> Result<Vec<i16>> {
        if chunk.samples.is_empty() {
            return Ok(Vec::new());
        }
        if chunk.sample_rate == 0 || chunk.channels == 0 {
            bail!("Invalid live audio format");
        }

        match (self.sample_rate, self.channels) {
            (None, None) => {
                self.sample_rate = Some(chunk.sample_rate);
                self.channels = Some(chunk.channels);
            }
            (Some(rate), Some(channels))
                if rate == chunk.sample_rate && channels == chunk.channels => {}
            _ => bail!("Live audio format changed during recording"),
        }

        self.pending_mono
            .extend(downmix_to_mono(&chunk.samples, chunk.channels));
        Ok(self.drain_resampled(false))
    }

    pub(super) fn finish(&mut self) -> Vec<i16> {
        self.drain_resampled(true)
    }

    fn drain_resampled(&mut self, final_chunk: bool) -> Vec<i16> {
        let Some(source_rate) = self.sample_rate else {
            return Vec::new();
        };
        if self.pending_mono.is_empty() {
            return Vec::new();
        }

        let step = source_rate as f64 / TARGET_SAMPLE_RATE as f64;
        let mut out = Vec::new();
        while self.next_src_pos + 1.0 < self.pending_mono.len() as f64 {
            out.push(interpolate_sample(&self.pending_mono, self.next_src_pos));
            self.next_src_pos += step;
        }

        if final_chunk {
            while self.next_src_pos < self.pending_mono.len() as f64 {
                out.push(interpolate_sample(&self.pending_mono, self.next_src_pos));
                self.next_src_pos += step;
            }
            self.pending_mono.clear();
            self.next_src_pos = 0.0;
        } else {
            let consumed =
                (self.next_src_pos.floor() as usize).min(self.pending_mono.len().saturating_sub(1));
            if consumed > 0 {
                self.pending_mono.drain(0..consumed);
                self.next_src_pos -= consumed as f64;
            }
        }

        out
    }
}

fn downmix_to_mono(samples: &[i16], channels: u16) -> Vec<i16> {
    let channels = channels.max(1) as usize;
    samples
        .chunks(channels)
        .map(|frame| {
            let sum: i32 = frame.iter().map(|sample| *sample as i32).sum();
            (sum / frame.len() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
        })
        .collect()
}

fn interpolate_sample(samples: &[i16], position: f64) -> i16 {
    let left = position.floor() as usize;
    let right = (left + 1).min(samples.len().saturating_sub(1));
    let frac = position - left as f64;
    let value = samples[left] as f64 * (1.0 - frac) + samples[right] as f64 * frac;
    value.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
}

pub(super) fn pcm_bytes(samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

pub(super) fn normalized_wav_bytes_with_cancel(
    path: &Path,
    is_cancelled: impl Fn() -> bool,
) -> Result<Vec<u8>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int {
        bail!("Only 16-bit PCM WAV recordings are supported");
    }

    let mut samples = Vec::new();
    for (index, sample) in reader.samples::<i16>().enumerate() {
        if index % 8_192 == 0 && is_cancelled() {
            bail!("ASR operation cancelled");
        }
        samples.push(sample?);
    }
    if is_cancelled() {
        bail!("ASR operation cancelled");
    }
    let channels = spec.channels.max(1) as usize;
    let mut mono = Vec::with_capacity(samples.len().div_ceil(channels));
    for (index, frame) in samples.chunks(channels).enumerate() {
        if index % 8_192 == 0 && is_cancelled() {
            bail!("ASR operation cancelled");
        }
        let sum: i32 = frame.iter().map(|sample| *sample as i32).sum();
        mono.push((sum / frame.len() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16);
    }

    let resampled = if spec.sample_rate == TARGET_SAMPLE_RATE {
        mono
    } else {
        resample_linear_with_cancel(&mono, spec.sample_rate, TARGET_SAMPLE_RATE, &is_cancelled)?
    };
    wav_bytes_from_mono_with_cancel(&resampled, TARGET_SAMPLE_RATE, &is_cancelled)
}

fn resample_linear_with_cancel(
    samples: &[i16],
    from_rate: u32,
    to_rate: u32,
    is_cancelled: &impl Fn() -> bool,
) -> Result<Vec<i16>> {
    if samples.is_empty() || from_rate == 0 {
        return Ok(Vec::new());
    }
    let out_len = ((samples.len() as u64 * to_rate as u64) / from_rate as u64).max(1) as usize;
    let ratio = from_rate as f64 / to_rate as f64;
    let mut output = Vec::with_capacity(out_len);
    for index in 0..out_len {
        if index % 8_192 == 0 && is_cancelled() {
            bail!("ASR operation cancelled");
        }
        let src = index as f64 * ratio;
        let left = src.floor() as usize;
        let right = (left + 1).min(samples.len() - 1);
        let frac = src - left as f64;
        let value = samples[left] as f64 * (1.0 - frac) + samples[right] as f64 * frac;
        output.push(value.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16);
    }
    Ok(output)
}

#[cfg(test)]
pub(super) fn wav_bytes_from_mono(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

fn wav_bytes_from_mono_with_cancel(
    samples: &[i16],
    sample_rate: u32,
    is_cancelled: &impl Fn() -> bool,
) -> Result<Vec<u8>> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for (index, sample) in samples.iter().enumerate() {
        if index % 8_192 == 0 && is_cancelled() {
            bail!("ASR operation cancelled");
        }
        out.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(out)
}
