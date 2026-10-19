use super::*;
use crate::reader::{ByteReader, MemReader};
use crate::wav::WavSource;
use std::io;
use std::sync::Mutex;

/// A sparse WAV with a repeating PCM16 sample, without allocating its audio data.
struct SparseWav {
    len: u64,
    parts: Vec<(u64, Vec<u8>)>,
    data: std::ops::Range<u64>,
    reads: Mutex<Vec<(u64, usize)>>,
}

impl SparseWav {
    fn new(frames: u64, rf64: bool, junk: u32, trailing_bext: bool) -> Self {
        let mut head = crate::wav::write_wav16(&[], 1, 48_000);
        let data_len = frames * 2;
        if rf64 {
            head[..4].copy_from_slice(b"RF64");
            head[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
            head[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
            let mut ds64 = b"ds64".to_vec();
            ds64.extend_from_slice(&28u32.to_le_bytes());
            ds64.extend_from_slice(&(data_len + 72).to_le_bytes());
            ds64.extend_from_slice(&data_len.to_le_bytes());
            ds64.extend_from_slice(&frames.to_le_bytes());
            ds64.extend_from_slice(&0u32.to_le_bytes());
            head.splice(12..12, ds64);
        } else {
            head[40..44].copy_from_slice(&(data_len as u32).to_le_bytes());
        }
        let full_len = head.len() as u64 + data_len + if junk > 0 { 8 + junk as u64 + (junk as u64 & 1) } else { 0 } + if trailing_bext { 354 } else { 0 };
        if rf64 {
            head[20..28].copy_from_slice(&(full_len - 8).to_le_bytes());
        } else {
            head[4..8].copy_from_slice(&((full_len - 8) as u32).to_le_bytes());
        }
        let mut parts = Vec::new();
        if junk > 0 {
            let mut prefix = head[..12].to_vec();
            prefix.extend_from_slice(b"JUNK");
            prefix.extend_from_slice(&junk.to_le_bytes());
            parts.push((0, prefix));
            parts.push((20 + junk as u64 + (junk as u64 & 1), head[12..].to_vec()));
        } else {
            parts.push((0, head.clone()));
        }
        let data_off = head.len() as u64 + if junk > 0 { 8 + junk as u64 + (junk as u64 & 1) } else { 0 };
        let mut len = data_off + data_len;
        if trailing_bext {
            let mut bext = vec![0; 354];
            bext[..4].copy_from_slice(b"bext");
            bext[4..8].copy_from_slice(&346u32.to_le_bytes());
            bext[346..354].copy_from_slice(&(86_400u64 * 2002).to_le_bytes());
            parts.push((len, bext));
            len += 354;
        }
        Self { len, parts, data: data_off..data_off + data_len, reads: Mutex::new(Vec::new()) }
    }

    fn bytes_read(&self) -> usize {
        self.reads.lock().unwrap().iter().map(|(_, n)| n).sum()
    }
}

impl ByteReader for SparseWav {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if offset.checked_add(buf.len() as u64).is_none_or(|end| end > self.len) {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.reads.lock().unwrap().push((offset, buf.len()));
        for (i, byte) in buf.iter_mut().enumerate() {
            let at = offset + i as u64;
            *byte = if self.data.contains(&at) { if (at - self.data.start) & 1 == 0 { 0x40 } else { 0x20 } } else { 0 };
            for (start, bytes) in &self.parts {
                if let Some(b) = at.checked_sub(*start).and_then(|n| bytes.get(n as usize)) {
                    *byte = *b;
                    break;
                }
            }
        }
        Ok(())
    }
}

/// #279: duration does not affect import bytes or memory; seeking reads only a short sample range.
#[test]
fn opening_and_seeking_long_wavs_have_bounded_reads() {
    for (frames, rf64) in [(48_000, false), (48_000 * 5400, false), (5_000_000_000, true)] {
        let reader = Arc::new(SparseWav::new(frames, rf64, 0, true));
        let source = crate::reader::open_reader_within("long.wav", reader.clone(), &[], &[], 0).unwrap();
        assert_eq!(source.info().duration, Tick::from_units(frames as i64, 48_000));
        assert_eq!(source.info().start_timecode, Some(86_400));
        assert!(reader.bytes_read() <= crate::reader::HEAD_LEN + 116, "opening read {} bytes", reader.bytes_read());
        let before = reader.bytes_read();
        let out = source.audio(frames as i64 - 1000, 240, 48_000).unwrap();
        assert!(out.channels[0].iter().all(|&sample| sample == 0x2040 as f32 / 32768.0));
        assert!(reader.bytes_read() - before <= 482);
        let reads = reader.reads.lock().unwrap();
        assert!(reads.iter().skip(1).all(|(_, n)| *n <= AUDIO_READ_BYTES));
        assert!(reads.iter().any(|(off, _)| *off >= reader.data.start + (frames - 1000) * 2));
    }
}

#[test]
fn unrelated_large_chunks_are_skipped_without_reading_the_body() {
    let reader = Arc::new(SparseWav::new(48_000, false, 2_000_000_000, false));
    let source = WavReaderSource::open("metadata.wav", reader.clone()).unwrap();
    assert_eq!(source.frames, 48_000);
    assert_eq!(reader.bytes_read(), 52);
    assert!(source.audio(40_000, 4, 48_000).unwrap().channels[0].iter().all(|&s| s > 0.25));
}

#[test]
fn oversized_optional_ixml_is_skipped_without_reading_its_body() {
    let mut sparse = SparseWav::new(48_000, false, 2_000_000, true);
    sparse.parts[0].1[12..16].copy_from_slice(b"iXML");
    let reader = Arc::new(sparse);
    let source = WavReaderSource::open("large-ixml.wav", reader.clone()).unwrap();
    assert_eq!(source.info().start_timecode, Some(86_400));
    assert_eq!(source.frames, 48_000);
    assert_eq!(reader.bytes_read(), 68);
    assert!(source.audio(47_900, 10, 48_000).unwrap().channels[0].iter().all(|&s| s > 0.25));
}

/// A full-clip request may exceed the former fixed sample cap, as transcription and extraction do.
#[test]
fn whole_clip_requests_are_bounded_by_media_length_without_a_duration_cap() {
    let frames = 17 * 1024 * 1024;
    let reader = Arc::new(SparseWav::new(frames as u64, false, 0, false));
    let source = WavReaderSource::open("long-analysis.wav", reader.clone()).unwrap();
    let actual = source.audio(0, frames, 48_000).unwrap();
    assert_eq!(actual.frames(), frames);
    assert_eq!(actual.channel_count(), 1);
    assert!(reader.reads.lock().unwrap().iter().all(|(_, n)| *n <= AUDIO_READ_BYTES));
    // Compare bounded windows with the legacy byte decoder, avoiding a second full audio copy.
    let legacy = WavSource::parse("window.wav", crate::wav::write_wav16(&vec![0x2040 as f32 / 32767.0; 256], 1, 48_000).into()).unwrap();
    let want = legacy.audio(0, 256, 48_000).unwrap();
    for start in [0, frames / 2, frames - 256] {
        assert_eq!(&actual.channels[0][start..start + 256], &want.channels[0]);
    }
    assert!(source.audio(0, frames + 4 * 48_000 + 3, 48_000).is_err());
    assert!(source.audio(0, usize::MAX, 48_000).is_err());
}

fn wav_bytes(tag: u16, bits: u16, channels: u16, extensible: bool) -> Vec<u8> {
    let mut data = Vec::new();
    for i in 0..9000 {
        let v = ((i % 37) as f64 - 18.0) / 32.0;
        match (tag, bits) {
            (1, 8) => data.push((v * 128.0 + 128.0) as u8),
            (1, 16) => data.extend_from_slice(&((v * 32768.0) as i16).to_le_bytes()),
            (1, 24) => data.extend_from_slice(&((v * 8_388_608.0) as i32).to_le_bytes()[..3]),
            (1, 32) => data.extend_from_slice(&((v * 2_147_483_648.0) as i32).to_le_bytes()),
            (3, 32) => data.extend_from_slice(&(v as f32).to_le_bytes()),
            (3, 64) => data.extend_from_slice(&v.to_le_bytes()),
            _ => unreachable!(),
        }
    }
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&(if extensible { 0xfffe } else { tag }).to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&44_100u32.to_le_bytes());
    fmt.extend_from_slice(&(44_100 * channels as u32 * bits as u32 / 8).to_le_bytes());
    fmt.extend_from_slice(&(channels * bits / 8).to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    if extensible {
        fmt.extend_from_slice(&22u16.to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        fmt.extend_from_slice(&0u32.to_le_bytes());
        fmt.extend_from_slice(&tag.to_le_bytes());
        fmt.extend_from_slice(&[0; 14]);
    }
    let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    wav.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    wav.extend_from_slice(&fmt);
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    let riff_len = wav.len() as u32 - 8;
    wav[4..8].copy_from_slice(&riff_len.to_le_bytes());
    wav
}

#[test]
fn pcm_float_extensible_and_resampling_match_the_byte_source() {
    for (tag, bits) in [(1, 8), (1, 16), (1, 24), (1, 32), (3, 32), (3, 64)] {
        for channels in [1, 2, 6] {
            for extensible in [false, true] {
                let bytes: Arc<[u8]> = wav_bytes(tag, bits, channels, extensible).into();
                let byte_source = WavSource::parse("parity.wav", bytes.clone()).unwrap();
                let streamed = WavReaderSource::open("parity.wav", Arc::new(MemReader(bytes))).unwrap();
                assert_eq!(byte_source.info(), streamed.info());
                for (start, frames, rate) in [(-20, 50, 44_100), (113, 2700, 44_100), (11, 3600, 48_000), (1490, 1500, 22_050), (9000, 4, 44_100)] {
                    assert_eq!(
                        byte_source.audio(start, frames, rate).unwrap().channels,
                        streamed.audio(start, frames, rate).unwrap().channels,
                        "tag {tag} bits {bits} channels {channels} extensible {extensible} start {start} rate {rate}"
                    );
                }
            }
        }
    }
}

fn with_chunk(wav: &[u8], id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut bytes = wav[..36].to_vec();
    bytes.extend_from_slice(id);
    bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
    bytes.extend_from_slice(body);
    if body.len() & 1 == 1 {
        bytes.push(0);
    }
    bytes.extend_from_slice(&wav[36..]);
    let len = bytes.len() as u32 - 8;
    bytes[4..8].copy_from_slice(&len.to_le_bytes());
    bytes
}

#[test]
fn broadcast_metadata_matches_the_byte_source() {
    let wav = crate::wav::write_wav16(&[0.25; 16], 2, 48_000);
    let mut bext = vec![0; 602];
    bext[338..346].copy_from_slice(&(86_400u64 * 2002).to_le_bytes());
    let xml = b"<BWFXML><BWF_TIME_REFERENCE_LOW>123</BWF_TIME_REFERENCE_LOW><BWF_TIME_REFERENCE_HIGH>1</BWF_TIME_REFERENCE_HIGH></BWFXML>";
    for bytes in [
        with_chunk(&wav, b"bext", &bext),
        with_chunk(&wav, b"iXML", xml),
        with_chunk(&with_chunk(&wav, b"iXML", xml), b"bext", &bext),
        with_chunk(&wav, b"JUNK", b"odd"),
    ] {
        let bytes: Arc<[u8]> = bytes.into();
        let old = WavSource::parse("bwf.wav", bytes.clone()).unwrap();
        let source = WavReaderSource::open("bwf.wav", Arc::new(MemReader(bytes))).unwrap();
        assert_eq!(source.info(), old.info());
        assert_eq!(source.time_reference(), old.time_reference());
    }
}

#[test]
fn malformed_headers_and_hostile_audio_requests_do_not_panic() {
    let wav = wav_bytes(1, 16, 2, false);
    for end in 0..80 {
        let bytes: Arc<[u8]> = wav[..end].into();
        assert!(
            std::panic::catch_unwind(|| {
                let _ = WavReaderSource::open("truncated.wav", Arc::new(MemReader(bytes)));
            })
            .is_ok()
        );
    }
    for i in 0..80 {
        let mut bytes = wav.clone();
        bytes[i] ^= 0xff;
        assert!(
            std::panic::catch_unwind(|| {
                if let Ok(source) = WavReaderSource::open("mutated.wav", Arc::new(MemReader(bytes.into()))) {
                    let _ = source.audio(-5, 20, 48_000);
                    let _ = source.audio(i64::MAX, 20, 48_000);
                }
            })
            .is_ok()
        );
    }
    assert!(WavReaderSource::open("oversized.wav", Arc::new(SparseWav::new(u64::MAX / 4, true, 0, false))).is_err());
    let source = WavReaderSource::open("ok.wav", Arc::new(MemReader(wav.into()))).unwrap();
    assert!(source.audio(0, usize::MAX, 48_000).is_err());
    assert!(source.audio(0, 20, 0).is_err());
    assert!(source.audio(i64::MIN, 20, 48_000).unwrap().channels.iter().flatten().all(|&v| v == 0.0));
}

#[test]
fn read_failures_are_reported_and_not_replaced_with_silence() {
    struct FailsAfterHeader(MemReader);
    impl ByteReader for FailsAfterHeader {
        fn len(&self) -> u64 {
            self.0.len()
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
            if offset >= 44 { Err(io::ErrorKind::UnexpectedEof.into()) } else { self.0.read_at(offset, buf) }
        }
    }
    let source = WavReaderSource::open("failed.wav", Arc::new(FailsAfterHeader(MemReader(wav_bytes(1, 16, 1, false).into())))).unwrap();
    assert!(matches!(source.audio(0, 20, 44_100), Err(MediaError::Io(_))));
}

#[cfg(any(unix, windows))]
#[test]
fn ffmpeg_pcm_and_float_samples_match() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let dir = filmcraft_testkit::fixtures_dir("media");
    std::fs::create_dir_all(&dir).unwrap();
    for codec in ["pcm_u8", "pcm_s16le", "pcm_s24le", "pcm_s32le", "pcm_f32le", "pcm_f64le"] {
        let path = filmcraft_testkit::temp_path(&dir.join(format!("streamed-{codec}.wav")));
        let result = std::process::Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=997:sample_rate=48000:duration=0.2", "-ac", "2", "-c:a", codec, "-f", "wav"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        let oracle = std::process::Command::new(&ff).args(["-v", "error", "-i"]).arg(&path).args(["-f", "f32le", "-c:a", "pcm_f32le", "-"]).output().unwrap();
        assert!(oracle.status.success(), "{}", String::from_utf8_lossy(&oracle.stderr));
        let reader = Arc::new(crate::reader::FileReader::open(&path).unwrap());
        let source = WavReaderSource::open("oracle.wav", reader).unwrap();
        let samples: Vec<f32> = oracle.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        for start in [0, 123, 9200] {
            let count = 333.min(samples.len() / 2 - start);
            let actual = source.audio(start as i64, count, 48_000).unwrap();
            for c in 0..2 {
                for i in 0..count {
                    assert_eq!(actual.channels[c][i], samples[(start + i) * 2 + c], "{codec} sample {} channel {c}", start + i);
                }
            }
        }
        drop(source);
        std::fs::remove_file(path).unwrap();
    }
}
