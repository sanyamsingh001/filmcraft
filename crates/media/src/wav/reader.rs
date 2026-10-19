//! Range-based RIFF/RF64 WAV source: scan chunk headers, then read only requested PCM samples.

use std::sync::Arc;

use filmcraft_frame::{AudioBuffer, VideoFrame};
use filmcraft_time::Tick;

use crate::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, Result, SharedReader, SharedSource};

const MAX_CHUNKS: usize = 65_536;
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const AUDIO_READ_BYTES: usize = 64 * 1024;

pub(crate) fn opener(name: &str, head: &[u8], reader: &SharedReader) -> Option<Result<SharedSource>> {
    super::sniff(head).then(|| WavReaderSource::open(name, reader.clone()).map(|s| Arc::new(s) as SharedSource))
}

/// A WAV backed by random-access bytes. Opening skips audio and unrelated chunk bodies, so its
/// memory and disk-read costs do not grow with the recording's duration.
pub struct WavReaderSource {
    info: MediaInfo,
    reader: SharedReader,
    data_off: u64,
    frames: u64,
    channels: usize,
    bits: u16,
    float: bool,
    rate: u32,
    time_reference: Option<u64>,
}

fn decode_error(name: &str, message: &str) -> MediaError {
    MediaError::Decode(format!("{name}: {message}"))
}

fn read<const N: usize>(reader: &SharedReader, offset: u64, name: &str) -> Result<[u8; N]> {
    if offset.checked_add(N as u64).is_none_or(|end| end > reader.len()) {
        return Err(decode_error(name, "truncated WAV chunk"));
    }
    let mut bytes = [0; N];
    reader.read_at(offset, &mut bytes).map_err(|e| MediaError::Io(format!("{name}: {e}")))?;
    Ok(bytes)
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    bytes.get(offset..offset.saturating_add(2)).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes).unwrap_or(0)
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    bytes.get(offset..offset.saturating_add(4)).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).unwrap_or(0)
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    bytes.get(offset..offset.saturating_add(8)).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(0)
}

impl WavReaderSource {
    pub fn open(name: &str, reader: SharedReader) -> Result<Self> {
        let header = read::<12>(&reader, 0, name)?;
        if !super::sniff(&header) {
            return Err(decode_error(name, "not a RIFF/RF64 WAV"));
        }
        let mut pos = 12u64;
        let mut fmt = None;
        let mut data = None;
        let mut rf64_data = None;
        let mut bext_tr = None;
        let mut ixml_tr = None;
        let mut chunks = 0;
        while pos.checked_add(8).is_some_and(|end| end <= reader.len()) {
            chunks += 1;
            if chunks > MAX_CHUNKS {
                return Err(decode_error(name, "too many WAV chunks"));
            }
            let chunk = read::<8>(&reader, pos, name)?;
            let id = chunk.get(..4).unwrap_or_default();
            let declared = u32_at(&chunk, 4) as u64;
            let body = pos + 8;
            let remaining = reader.len() - body;
            let len = if id == b"data" {
                let len = if declared == u32::MAX as u64 { rf64_data.unwrap_or(remaining) } else { declared };
                len.min(remaining)
            } else {
                declared
            };
            if len > remaining {
                break;
            }
            match id {
                b"fmt " if len >= 16 => {
                    let mut bytes = [0; 40];
                    let n = len.min(bytes.len() as u64) as usize;
                    reader.read_at(body, &mut bytes[..n]).map_err(|e| MediaError::Io(format!("{name}: {e}")))?;
                    let tag = u16_at(&bytes, 0);
                    let tag = if tag == 0xfffe && len >= 40 { u16_at(&bytes, 24) } else { tag };
                    fmt = Some((tag, u16_at(&bytes, 2), u32_at(&bytes, 4), u16_at(&bytes, 14)));
                }
                b"data" => data = Some((body, len)),
                b"ds64" if len >= 28 => {
                    let bytes = read::<28>(&reader, body, name)?;
                    rf64_data = Some(u64_at(&bytes, 8));
                }
                b"bext" if len >= 346 => {
                    let bytes = read::<8>(&reader, body + 338, name)?;
                    bext_tr = Some(u64_at(&bytes, 0));
                }
                // iXML is optional. Skip oversized metadata without reading its body; it must
                // not prevent opening the audio (or the independent bext time reference).
                b"iXML" if len <= MAX_METADATA_BYTES => {
                    let mut bytes = vec![0; len as usize];
                    reader.read_at(body, &mut bytes).map_err(|e| MediaError::Io(format!("{name}: {e}")))?;
                    ixml_tr = super::ixml_time_reference(&bytes);
                }
                _ => {}
            }
            let Some(next) = body.checked_add(len).and_then(|end| end.checked_add(len & 1)) else {
                return Err(decode_error(name, "WAV chunk offset overflow"));
            };
            pos = next;
        }
        let (tag, ch, rate, bits) = fmt.ok_or_else(|| decode_error(name, "missing fmt chunk"))?;
        let (data_off, data_len) = data.ok_or_else(|| decode_error(name, "missing data chunk"))?;
        let float = tag == 3;
        if !(tag == 1 || float) || ch == 0 || rate == 0 || !matches!(bits, 8 | 16 | 24 | 32 | 64) {
            return Err(MediaError::Unsupported(format!("{name}: WAV format tag {tag}, {ch} channels, {rate} Hz, {bits} bits")));
        }
        let channels = ch as usize;
        let frame_bytes = channels * (bits as usize / 8);
        let frames = data_len / frame_bytes as u64;
        let duration = frames as i128 * filmcraft_time::TICKS_PER_SECOND as i128 / rate as i128;
        let duration = Tick(i64::try_from(duration).map_err(|_| decode_error(name, "WAV duration is too large"))?);
        let time_reference = bext_tr.or(ixml_tr);
        let mut info = MediaInfo {
            name: name.into(),
            kind: MediaKind::AudioOnly,
            duration,
            video: None,
            audio: Some(AudioStreamInfo {
                sample_rate: rate,
                channels: ch as u32,
                codec: if float { "PCM float".into() } else { "PCM".into() },
                bits_per_sample: Some(bits as u32),
            }),
            container: if time_reference.is_some() { "Broadcast WAV".into() } else { "WAV".into() },
            start_timecode: None,
            file_size: Some(reader.len()),
        };
        if let Some(tr) = time_reference {
            let r = info.frame_rate();
            let n = tr as i128 * r.num as i128;
            let d = rate as i128 * r.den as i128;
            info.start_timecode = Some(i64::try_from((n + d / 2) / d).map_err(|_| decode_error(name, "WAV time reference is too large"))?);
        }
        Ok(Self { info, reader, data_off, frames, channels, bits, float, rate, time_reference })
    }

    /// Broadcast Wave start time: samples since midnight.
    pub fn time_reference(&self) -> Option<u64> {
        self.time_reference
    }

    fn read_frames(&self, first: u64, count: usize) -> Result<Vec<u8>> {
        let frame_bytes = self.channels * (self.bits as usize / 8);
        let len = count.checked_mul(frame_bytes).ok_or_else(|| decode_error(&self.info.name, "WAV audio range is too large"))?;
        let offset = first
            .checked_mul(frame_bytes as u64)
            .and_then(|n| self.data_off.checked_add(n))
            .ok_or_else(|| decode_error(&self.info.name, "WAV audio offset overflow"))?;
        let mut bytes = vec![0; len];
        self.reader.read_at(offset, &mut bytes).map_err(|e| MediaError::Io(format!("{}: {e}", self.info.name)))?;
        Ok(bytes)
    }

    fn sample(&self, bytes: &[u8], frame: usize, channel: usize) -> f32 {
        let bps = self.bits as usize / 8;
        let offset = (frame * self.channels + channel) * bps;
        let Some(b) = bytes.get(offset..offset.saturating_add(bps)) else { return 0.0 };
        match (self.bits, self.float) {
            (8, _) => (b[0] as f32 - 128.0) / 128.0,
            (16, _) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
            (24, _) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
            (32, false) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
            (32, true) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            (64, true) => f64::from_le_bytes(b.try_into().unwrap_or([0; 8])) as f32,
            _ => 0.0,
        }
    }
}

impl MediaSource for WavReaderSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, _req: FrameRequest) -> Result<Arc<VideoFrame>> {
        Err(MediaError::NoStream("video"))
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer> {
        if sample_rate == 0 {
            return Err(decode_error(&self.info.name, "WAV audio sample rate must be positive"));
        }
        // Whole-clip analysis/transcription/extraction asks for the complete media span. Bound
        // requests by that span, with the four-second waveform window and interpolation padding
        // used by existing callers, rather than imposing a new maximum recording duration.
        let span = (self.frames as u128 * sample_rate as u128).div_ceil(self.rate as u128);
        let limit = span + 4 * sample_rate as u128 + 2;
        let bytes = frames.checked_mul(self.channels).and_then(|n| n.checked_mul(std::mem::size_of::<f32>()));
        if frames as u128 > limit || bytes.is_none_or(|n| n > isize::MAX as usize) {
            return Err(decode_error(&self.info.name, "WAV audio request exceeds the media span or address space"));
        }
        let allocation_error = |e| MediaError::Io(format!("{}: cannot allocate WAV audio output: {e}", self.info.name));
        let mut channels = Vec::new();
        channels.try_reserve_exact(self.channels).map_err(allocation_error)?;
        for _ in 0..self.channels {
            let mut samples = Vec::new();
            samples.try_reserve_exact(frames).map_err(allocation_error)?;
            samples.resize(frames, 0.0);
            channels.push(samples);
        }
        let mut out = AudioBuffer { sample_rate, channels };
        let ratio = self.rate as f64 / sample_rate as f64;
        let frame_bytes = self.channels * (self.bits as usize / 8);
        let block_frames = (AUDIO_READ_BYTES / frame_bytes).max(2);
        let mut bytes = Vec::new();
        let mut block_start = 0;
        let mut block_end = 0;
        let last = ((start as f64 + frames.saturating_sub(1) as f64) * ratio).max(0.0).floor() as u64;
        for i in 0..frames {
            let src = (start as f64 + i as f64) * ratio;
            if src < 0.0 {
                continue;
            }
            let first = src.floor() as u64;
            if first >= self.frames {
                break;
            }
            let second = first.saturating_add(1).min(self.frames - 1);
            if first < block_start || second >= block_end {
                let count = (self.frames - first).min(block_frames as u64).min(last.saturating_sub(first).saturating_add(2)) as usize;
                bytes = self.read_frames(first, count)?;
                block_start = first;
                block_end = first + count as u64;
            }
            let frac = (src - first as f64) as f32;
            for (channel, dst) in out.channels.iter_mut().enumerate() {
                let a = self.sample(&bytes, (first - block_start) as usize, channel);
                let b = self.sample(&bytes, (second - block_start) as usize, channel);
                dst[i] = a + (b - a) * frac;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
