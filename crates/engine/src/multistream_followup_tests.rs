//! Selected-stream contracts beyond PR361's decoder and default placement.
use std::sync::{Arc, Mutex};

use filmcraft_frame::{AudioBuffer, VideoFrame};
use filmcraft_media::{FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource};
use filmcraft_project::{AudioChannelMap, AudioChannels, ClipId, ItemId, ItemKind, Label, MediaClip, MediaRef, SequenceSettings, TrackItem};
use filmcraft_time::{FrameRate, Tick};
use serde_json::{Value, json};

use crate::{Services, Session};

type Calls = Arc<Mutex<Vec<(usize, i64, usize, u32)>>>;

struct Tones {
    info: MediaInfo,
    calls: Calls,
}

impl Tones {
    fn new(streams: usize) -> Self {
        let source =
            filmcraft_media::generators::GeneratorSource::new(filmcraft_media::Generator::BarsAndTone, 32, 18, FrameRate::FPS_24, Tick::from_seconds_f64(2.0));
        let mut info = source.info().clone();
        info.kind = MediaKind::AudioOnly;
        info.video = None;
        info.audio_streams = vec![info.audio_streams[0].clone(); streams];
        Self { info, calls: Arc::default() }
    }

    fn sample(stream: usize, channel: usize, frame: i64, rate: u32) -> f32 {
        let frequency = (stream * 2 + channel + 1) as f64 * 240.0;
        let amplitude = 0.05 * (stream + 1) as f64;
        (amplitude * (std::f64::consts::TAU * frequency * frame as f64 / rate as f64).sin()) as f32
    }
}

impl MediaSource for Tones {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, _: FrameRequest) -> filmcraft_media::Result<Arc<VideoFrame>> {
        Err(MediaError::NoStream("video"))
    }
    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<AudioBuffer> {
        self.audio_stream(0, start, frames, sample_rate)
    }
    fn audio_stream(&self, stream: usize, start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<AudioBuffer> {
        if stream >= self.info.audio_streams.len() {
            return Err(MediaError::NoStream("audio"));
        }
        self.calls.lock().unwrap().push((stream, start, frames, sample_rate));
        Ok(AudioBuffer {
            sample_rate,
            channels: (0..2).map(|channel| (0..frames).map(|i| Self::sample(stream, channel, start + i as i64, sample_rate)).collect()).collect(),
        })
    }
}

#[derive(Default)]
struct MemoryFiles {
    writes: Mutex<Vec<(String, Vec<u8>)>>,
}
impl Services for MemoryFiles {
    fn read_file(&self, _: &str) -> std::io::Result<Vec<u8>> {
        Err(std::io::ErrorKind::NotFound.into())
    }
    fn write_file(&self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.writes.lock().unwrap().push((path.to_owned(), bytes.to_vec()));
        Ok(())
    }
    fn export_in_memory(&self) -> bool {
        true
    }
}

fn add_source(s: &mut Session, streams: usize) -> (ItemId, Calls) {
    let source = Arc::new(Tones::new(streams));
    let calls = source.calls.clone();
    let mut project = (*s.project).clone();
    let item = project.add_item(
        "distinct tones",
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::File { path: format!("tone-source-{streams}.wav") },
            info: source.info.clone(),
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        }),
        None,
    );
    s.project = Arc::new(project);
    s.media.insert(item, source);
    (item, calls)
}

fn fixture() -> (Session, ItemId, ItemId, Calls, Arc<MemoryFiles>) {
    let files = Arc::new(MemoryFiles::default());
    let mut session = Session::new(files.clone());
    let mut project = (*session.project).clone();
    let sequence = project.new_sequence("streams", SequenceSettings { width: 32, height: 18, frame_rate: FrameRate::FPS_24, ..Default::default() }, 0, 1, None);
    session.project = Arc::new(project);
    session.state.active_sequence = Some(sequence);
    session.state.open_sequences = vec![sequence];
    let (item, calls) = add_source(&mut session, 3);
    session.state.project_selection = vec![item];
    session.execute("sequence.linkedSelection", json!({"on": false})).unwrap();
    (session, item, sequence, calls, files)
}

fn place(s: &mut Session, item: ItemId, seconds: f64) -> Vec<ClipId> {
    let result = s.execute("timeline.place", json!({"item": item.0, "track": "A1", "seconds": seconds})).unwrap();
    result["clips"].as_array().unwrap().iter().map(|id| ClipId(id.as_u64().unwrap())).collect()
}
fn clip(s: &Session, id: ClipId) -> TrackItem {
    s.active_sequence().unwrap().find_item(id).unwrap().1.clone()
}
fn snapshot(s: &Session) -> (Value, Value, usize, usize) {
    (serde_json::to_value(&*s.project).unwrap(), serde_json::to_value(&s.state).unwrap(), s.history.undo.len(), s.history.redo.len())
}
fn select_audio(s: &mut Session, id: ClipId) {
    s.state.project_selection.clear();
    s.execute("timeline.select", json!({"clips": [id.0]})).unwrap();
}

#[test]
fn explicit_master_map_places_only_selected_stream_and_survives_schema_roundtrip() {
    let (mut s, item, _, _, _) = fixture();
    s.execute("clip.audioChannels", json!({"items": [item.0], "format": "stereo", "clips": [[1, 0]], "streams": [2]})).unwrap();
    let mapped = s.project.clone();
    assert_eq!(mapped.item(item).unwrap().as_media().unwrap().interpret.audio_channels.as_ref().unwrap().streams, [2]);
    let ids = place(&mut s, item, 0.0);
    assert_eq!(ids.len(), 1);
    let it = clip(&s, ids[0]);
    assert_eq!(it.audio_stream, 2);
    assert_eq!(it.source_channels, [1, 0]);
    s.undo().unwrap();
    assert_eq!(*s.project, *mapped);
    s.redo().unwrap();
    let bytes = filmcraft_format::encode(&s.project, false);
    let envelope: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(envelope["schema_version"], 14);
    let decoded = filmcraft_format::decode(&bytes).unwrap();
    assert_eq!(decoded.project, *s.project);
    assert!(matches!(
        filmcraft_format::migrate_with(&filmcraft_format::MIGRATIONS[..12], envelope, 14),
        Err(filmcraft_format::FormatError::TooNew { found: 14, supported: 13 })
    ));
}

#[test]
fn channel_only_edit_retains_each_stream_and_invalid_edits_are_atomic() {
    let (mut s, item, _, _, _) = fixture();
    let ids = place(&mut s, item, 0.0);
    assert_eq!(ids.len(), 3);
    select_audio(&mut s, ids[2]);
    s.state.selection.clear();
    let unmapped = s.project.clone();
    let undo_depth = s.history.undo.len();
    s.execute("clip.audioChannels", json!({"clips": [ids[2].0], "channels": [1]})).unwrap();
    assert!(s.state.selection.is_empty());
    assert_eq!(s.history.undo.len(), undo_depth + 1);
    let mapped = s.project.clone();
    assert_eq!(s.undo().as_deref(), Some("Modify Audio Channels"));
    assert_eq!(*s.project, *unmapped);
    assert_eq!(s.redo().as_deref(), Some("Modify Audio Channels"));
    assert_eq!(*s.project, *mapped);
    assert_eq!((clip(&s, ids[2]).audio_stream, clip(&s, ids[2]).source_channels), (2, vec![1]));
    let before = snapshot(&s);
    for params in [json!({"clips": [ids[2].0], "channels": [2]}), json!({"clips": [ids[2].0], "stream": 3}), json!({"clips": [ids[2].0], "stream": -1})] {
        assert!(s.execute("clip.audioChannels", params).is_err());
        assert_eq!(snapshot(&s), before);
    }
    s.state.project_selection = vec![item];
    let before = snapshot(&s);
    for params in [json!({"items": [item.0], "clips": [[0], [1]], "streams": [2]}), json!({"items": [item.0], "clips": [[2]], "streams": [2]})] {
        assert!(s.execute("clip.audioChannels", params).is_err());
        assert_eq!(snapshot(&s), before);
    }
    let mut project = (*s.project).clone();
    project.sequence_mut(s.state.active_sequence.unwrap()).unwrap().audio_tracks[2].locked = true;
    s.project = Arc::new(project);
    let before = snapshot(&s);
    assert!(s.execute("clip.audioChannels", json!({"clips": [ids[2].0], "stream": 1})).is_err());
    assert_eq!(snapshot(&s), before);
}

#[test]
fn replacing_third_stream_with_single_stream_resets_selector_and_remains_audible() {
    let (mut s, item, _, _, _) = fixture();
    let ids = place(&mut s, item, 0.0);
    let original = clip(&s, ids[2]);
    let (replacement, _) = add_source(&mut s, 1);
    select_audio(&mut s, ids[2]);
    s.state.project_selection = vec![replacement];
    s.execute("clip.replaceFromBin", json!({"clips": [ids[2].0], "item": replacement.0})).unwrap();
    let replaced = clip(&s, ids[2]);
    assert_eq!(replaced.item, replacement);
    assert_eq!(replaced.audio_stream, 0);
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let signal = filmcraft_render::audio::clip_signal(&replaced, 0, 480, 48_000, &provider).unwrap();
    assert!(signal[0].iter().any(|x| x.abs() > 0.04));
    s.undo().unwrap();
    assert_eq!(clip(&s, ids[2]), original);
}

#[test]
fn selected_stream_peak_sync_and_extraction_use_the_selected_decoder() {
    let (mut s, item, _, calls, files) = fixture();
    let ids = place(&mut s, item, 0.0);
    select_audio(&mut s, ids[2]);
    calls.lock().unwrap().clear();
    let peak = s.execute("clip.audioPeak", json!({"clips": [ids[2].0]})).unwrap();
    assert!((peak["peakDb"].as_f64().unwrap() - 20.0 * 0.15_f64.log10()).abs() < 0.02);
    assert!(calls.lock().unwrap().iter().all(|call| call.0 == 2));
    let sync = crate::sync::timeline_clip(&s.project, &clip(&s, ids[2]));
    let mono = crate::sync::clip_audio(&s, &sync).unwrap();
    assert!((mono[25] - (Tones::sample(2, 0, 25, 48_000) + Tones::sample(2, 1, 25, 48_000)) * 0.5).abs() < 1e-6);
    s.execute("clip.audioChannels", json!({"items": [item.0], "clips": [[0, 1]], "streams": [2]})).unwrap();
    assert_eq!(crate::sync::project_clip(&s.project, item).unwrap().audio_stream, 2);
    s.state.project_selection = vec![item];
    calls.lock().unwrap().clear();
    let result = s.execute("clip.extractAudio", json!({"items": [item.0]})).unwrap();
    assert!(!result["extracted"].as_array().unwrap().is_empty());
    assert!(calls.lock().unwrap().iter().all(|call| call.0 == 2));
    assert!(!files.writes.lock().unwrap().is_empty());
}

#[test]
fn aaf_essence_distinguishes_streams_and_linked_refusal_precedes_writes() {
    use crate::aaf_omf::{AudioMode, AudioPlan};
    use filmcraft_interchange::essence::{EssenceData, EssenceKey, NestNeeds};
    let (mut s, item, sequence, calls, files) = fixture();
    place(&mut s, item, 0.0);
    let plan = AudioPlan {
        mode: AudioMode::Embedded,
        aiff: false,
        mxf: false,
        sample_rate: 48_000,
        bits: 16,
        trim: true,
        handles: Tick::ZERO,
        render_effects: false,
        breakout: false,
        nests: NestNeeds::Inside,
    };
    let (essences, paths) = crate::aaf_omf::prepare_audio(&s, sequence, &plan, "ignored").unwrap();
    assert!(paths.is_empty());
    assert_eq!(essences.len(), 3);
    for (stream, essence) in essences.iter().enumerate() {
        assert_eq!(essence.key, EssenceKey::media(item, stream));
        let EssenceData::Embedded(bytes) = &essence.data else { panic!("expected embedded PCM") };
        for channel in 0..2 {
            let offset = (25 * 2 + channel) * 2;
            let actual = i16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
            let expected = (Tones::sample(stream, channel, 25, 48_000) * 32767.0).round() as i16;
            assert_eq!(actual, expected);
        }
    }
    let options = filmcraft_interchange::aaf::AafOptions {
        media: filmcraft_interchange::essence::MediaOptions { essence: essences, ..Default::default() },
        ..Default::default()
    };
    let (aaf, _) = filmcraft_interchange::aaf::export(&s.project, sequence, &options).unwrap();
    let omf_options = filmcraft_interchange::omf::OmfOptions { media: options.media.clone(), ..Default::default() };
    let (omf, _) = filmcraft_interchange::omf::export(&s.project, sequence, &omf_options).unwrap();
    for (kind, document) in [("AAF", aaf), ("OMF", omf)] {
        let (imported, extracted, _) = if kind == "AAF" {
            filmcraft_interchange::aaf::import(&document, &Default::default()).unwrap()
        } else {
            filmcraft_interchange::omf::import(&document, &Default::default()).unwrap()
        };
        assert_eq!(extracted.len(), 3, "{kind}: composition memo must retain distinct selected-stream essence");
        let tracks = &imported.project.sequence(imported.sequences[0]).unwrap().audio_tracks;
        assert_eq!(tracks.len(), 3, "{kind}");
        let mut source_ids: Vec<_> = tracks.iter().map(|track| track.items[0].item).collect();
        source_ids.sort();
        source_ids.dedup();
        assert_eq!(source_ids.len(), 3, "{kind}: each stream remains a distinct imported source");
        let mut waveforms: Vec<_> = extracted.iter().map(|media| media.wav.clone()).collect();
        waveforms.sort();
        waveforms.dedup();
        assert_eq!(waveforms.len(), 3, "{kind}");
    }
    assert!(calls.lock().unwrap().iter().any(|call| call.0 == 2));
    assert!(files.writes.lock().unwrap().is_empty());
    let error =
        s.execute("file.exportAaf", json!({"path": "must-not-be-written.aaf", "sequence": sequence.0, "audio": "linked", "trimAudio": false})).unwrap_err();
    assert!(error.to_string().contains("embedded") || error.to_string().contains("separate"));
    assert!(files.writes.lock().unwrap().is_empty());
}

#[test]
fn mono_breakout_retains_master_stream_without_adding_other_streams() {
    let (mut s, item, _, _, _) = fixture();
    s.execute("clip.audioChannels", json!({"items": [item.0], "clips": [[0, 1]], "streams": [2]})).unwrap();
    let before = s.project.clone();
    let result = s.execute("clip.breakoutToMono", json!({"items": [item.0]})).unwrap();
    let derived: Vec<ItemId> = result["items"].as_array().unwrap().iter().map(|i| ItemId(i.as_u64().unwrap())).collect();
    assert_eq!(derived.len(), 2);
    for (channel, &item) in derived.iter().enumerate() {
        let media = s.project.item(item).unwrap().as_media().unwrap();
        let map = media.interpret.audio_channels.as_ref().unwrap();
        assert_eq!(map.streams, [2]);
        assert_eq!(map.clips, [vec![channel as u16]]);
        assert_eq!(crate::commands::mapped_audio_placement_specs(Some(map), &media.info).unwrap().len(), 1);
    }
    s.undo().unwrap();
    assert_eq!(*s.project, *before);
}

#[test]
fn new_sequence_preflights_later_complete_map_before_any_history_change() {
    let (mut s, first, _, _, _) = fixture();
    let (later, _) = add_source(&mut s, 3);
    let mut project = (*s.project).clone();
    project.item_mut(later).unwrap().as_media_mut().unwrap().interpret.audio_channels =
        Some(AudioChannelMap { format: AudioChannels::Mono, clips: vec![vec![0]; filmcraft_project::MAX_AUDIO_CHANNEL_CLIPS], streams: vec![] });
    s.project = Arc::new(project);
    s.state.project_selection = vec![first, later];
    let before = snapshot(&s);
    assert!(s.execute("file.newSequenceFromClip", json!({"items": [first.0, later.0]})).is_err());
    assert_eq!(snapshot(&s), before);
}

struct MusicStreams {
    info: MediaInfo,
    samples: Vec<Vec<Vec<f32>>>,
    calls: Calls,
}
impl MediaSource for MusicStreams {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, _: FrameRequest) -> filmcraft_media::Result<Arc<VideoFrame>> {
        Err(MediaError::NoStream("video"))
    }
    fn audio(&self, start: i64, frames: usize, rate: u32) -> filmcraft_media::Result<AudioBuffer> {
        self.audio_stream(0, start, frames, rate)
    }
    fn audio_stream(&self, stream: usize, start: i64, frames: usize, rate: u32) -> filmcraft_media::Result<AudioBuffer> {
        let channels = self.samples.get(stream).ok_or(MediaError::NoStream("audio"))?;
        assert_eq!(rate, 48_000, "fixture's independent native/output rate");
        self.calls.lock().unwrap().push((stream, start, frames, rate));
        Ok(AudioBuffer {
            sample_rate: rate,
            channels: channels
                .iter()
                .map(|channel| {
                    (0..frames).map(|i| usize::try_from(start + i as i64).ok().and_then(|frame| channel.get(frame)).copied().unwrap_or(0.0)).collect()
                })
                .collect(),
        })
    }
}
fn install_music(s: &mut Session, item: ItemId, base_bpm: f64) -> Calls {
    let mut info = Tones::new(3).info;
    let samples: Vec<_> =
        (0..3).map(|stream| filmcraft_audio_dsp::remix::test_music(48_000, base_bpm + stream as f64 * 10.0, 8, stream as u32 + 1).0).collect();
    // All supported tempos supply more than eight seconds of independently generated music.
    info.duration = Tick::from_seconds_f64(8.0);
    let calls = Calls::default();
    let source = Arc::new(MusicStreams { info: info.clone(), samples, calls: calls.clone() });
    let mut project = (*s.project).clone();
    project.item_mut(item).unwrap().as_media_mut().unwrap().info = info;
    s.project = Arc::new(project);
    s.media.insert(item, source);
    calls
}

#[test]
fn remix_cache_separates_streams_sessions_and_reopened_sources() {
    let (mut s, item, _, _, _) = fixture();
    let calls = install_music(&mut s, item, 120.0);
    let ids = place(&mut s, item, 0.0);
    select_audio(&mut s, ids[0]);
    let first = s.execute("clip.remix.enable", json!({})).unwrap();
    assert!(calls.lock().unwrap().iter().all(|call| call.0 == 0));
    assert!(!calls.lock().unwrap().is_empty());
    let count = calls.lock().unwrap().len();
    s.execute("clip.remix.properties", json!({"seconds": 6.0})).unwrap();
    assert_eq!(calls.lock().unwrap().len(), count, "same source/stream/range is cached");
    s.execute("clip.audioChannels", json!({"clips": [ids[0].0], "stream": 2})).unwrap();
    let second = s.execute("clip.remix.properties", json!({"seconds": 6.0})).unwrap();
    assert!(calls.lock().unwrap()[count..].iter().all(|call| call.0 == 2));
    assert!(calls.lock().unwrap().len() > count, "a new selected stream must be analysed");
    assert!((first["bpm"].as_f64().unwrap() - second["bpm"].as_f64().unwrap()).abs() > 10.0);
    let (mut other, other_item, _, _, _) = fixture();
    assert_eq!(item, other_item, "new project reuses item IDs");
    let other_calls = install_music(&mut other, other_item, 90.0);
    let other_clip = place(&mut other, other_item, 0.0)[0];
    select_audio(&mut other, other_clip);
    let third = other.execute("clip.remix.enable", json!({})).unwrap();
    assert!(!other_calls.lock().unwrap().is_empty(), "another session must decode its own source");
    assert!((first["bpm"].as_f64().unwrap() - third["bpm"].as_f64().unwrap()).abs() > 10.0);
    // A host can reopen/relink an item in the same pool. Equal IDs and range must not keep
    // analysis of the retired MediaSource after the pool forgets it.
    other.media.clear();
    let reopened_calls = install_music(&mut other, other_item, 160.0);
    other.execute("clip.remix.revert", json!({})).unwrap();
    let reopened = other.execute("clip.remix.enable", json!({})).unwrap();
    assert!(!reopened_calls.lock().unwrap().is_empty());
    assert!((third["bpm"].as_f64().unwrap() - reopened["bpm"].as_f64().unwrap()).abs() > 20.0);
}

#[test]
fn subclip_placement_inherits_parent_stream_map_and_absolute_source_in() {
    let (mut s, item, _, calls, _) = fixture();
    s.execute("clip.audioChannels", json!({"items": [item.0], "clips": [[1, 0]], "streams": [2]})).unwrap();
    let result = s
        .execute(
            "clip.makeSubclip",
            json!({"item": item.0, "name": "selected-stream subclip", "start": Tick::from_seconds_f64(0.5).0, "end": Tick::from_seconds_f64(1.5).0}),
        )
        .unwrap();
    let subclip = ItemId(result["item"].as_u64().unwrap());
    s.execute("sequence.addTracks", json!({"video": 1, "audio": 0})).unwrap();
    let before_error = snapshot(&s);
    assert!(s.execute("timeline.place", json!({"item": subclip.0, "track": "V1", "seconds": 0})).is_err());
    assert_eq!(snapshot(&s), before_error, "an audio-only subclip cannot create a picture clip");
    let ids = place(&mut s, subclip, 0.0);
    assert_eq!(ids.len(), 1);
    let it = clip(&s, ids[0]);
    assert_eq!(it.audio_stream, 2);
    assert_eq!(it.source_channels, [1, 0]);
    assert_eq!(it.source_in, Tick::from_seconds_f64(0.5));
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    calls.lock().unwrap().clear();
    let signal = filmcraft_render::audio::clip_signal(&it, 0, 480, 48_000, &provider).unwrap();
    assert_eq!(&*calls.lock().unwrap(), &[(2, 24_000, 480, 48_000)], "one exact source-range read from the selected stream");
    assert!((signal[0][25] - Tones::sample(2, 1, 24_025, 48_000)).abs() < 1e-5);
    assert!((signal[1][25] - Tones::sample(2, 0, 24_025, 48_000)).abs() < 1e-5);
    s.execute("source.open", json!({"item": subclip.0})).unwrap();
    for command in ["source.insert", "source.overwrite"] {
        s.set_playhead(Tick::ZERO);
        let before = s.project.clone();
        let inserted = s.execute(command, json!({})).unwrap();
        let ids = inserted["clips"].as_array().unwrap();
        assert_eq!(ids.len(), 1, "{command} places only the selected audio component");
        let audio = clip(&s, ClipId(ids[0].as_u64().unwrap()));
        assert_eq!((audio.audio_stream, audio.source_channels.as_slice(), audio.source_in), (2, &[1, 0][..], Tick::from_seconds_f64(0.5)));
        assert!(s.active_sequence().unwrap().video_tracks.iter().all(|track| track.items.is_empty()));
        s.undo().unwrap();
        assert_eq!(*s.project, *before, "{command} is one undoable edit");
    }
    let target = s.execute("timeline.place", json!({"item": item.0, "track": "A1", "seconds": 2, "duration": Tick::from_seconds_f64(1.0).0})).unwrap();
    let target = ClipId(target["clips"][0].as_u64().unwrap());
    let before = s.project.clone();
    let replaced = s.execute("clip.replaceFromBin", json!({"item": subclip.0, "clips": [target.0]})).unwrap();
    assert_eq!(replaced["clips"], json!([target.0]));
    let audio = clip(&s, target);
    assert_eq!(audio.item, subclip);
    assert_eq!((audio.audio_stream, audio.source_channels.as_slice(), audio.source_in), (2, &[1, 0][..], Tick::from_seconds_f64(0.5)));
    calls.lock().unwrap().clear();
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let signal = filmcraft_render::audio::clip_signal(&audio, 96_000, 480, 48_000, &provider).unwrap();
    assert_eq!(&*calls.lock().unwrap(), &[(2, 24_000, 480, 48_000)]);
    assert!((signal[0][25] - Tones::sample(2, 1, 24_025, 48_000)).abs() < 1e-5);
    assert!((signal[1][25] - Tones::sample(2, 0, 24_025, 48_000)).abs() < 1e-5);
    s.undo().unwrap();
    assert_eq!(*s.project, *before, "subclip replacement is one undoable edit");
}

#[test]
fn picture_subclip_keeps_linked_audio_and_parent_anchor() {
    let (mut s, _, _, _, _) = fixture();
    s.execute("sequence.addTracks", json!({"video": 1, "audio": 0})).unwrap();
    let parent = s.execute("file.newBarsAndTone", json!({"width": 32, "height": 18, "seconds": 2})).unwrap()["item"].as_u64().unwrap();
    let subclip = s.execute("clip.makeSubclip", json!({"item": parent, "start": Tick::from_seconds_f64(0.5).0, "end": Tick::from_seconds_f64(1.5).0})).unwrap()
        ["item"]
        .as_u64()
        .unwrap();
    let before = s.project.clone();
    let result = s.execute("timeline.place", json!({"item": subclip, "track": "V1", "seconds": 0})).unwrap();
    assert_eq!(result["clips"].as_array().unwrap().len(), 2);
    let q = s.active_sequence().unwrap();
    let picture = &q.video_tracks[0].items[0];
    let audio = &q.audio_tracks[0].items[0];
    assert!(picture.link.is_some());
    assert_eq!(picture.link, audio.link);
    assert_eq!(picture.item, ItemId(subclip));
    assert_eq!(audio.item, ItemId(subclip));
    assert_eq!((picture.source_in, audio.source_in), (Tick::from_seconds_f64(0.5), Tick::from_seconds_f64(0.5)));
    assert_eq!(picture.effect("motion").unwrap().vec2_at("anchor", picture.source_in), filmcraft_geom::Vec2::new(16.0, 9.0));
    assert_eq!(audio.audio_stream, 0);
    s.undo().unwrap();
    assert_eq!(*s.project, *before);
}
