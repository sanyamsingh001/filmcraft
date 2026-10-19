//! Source transport must play its own media/audio clock without changing the Program edit.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_media::{Generator, generators::GeneratorSource};
use filmcraft_project::{ItemId, ItemKind, Label, Project, SequenceSettings, TrackKind};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use filmcraft_ui_egui::control::ControlRequest;
use filmcraft_ui_egui::dock::PanelKind;
use filmcraft_ui_egui::{AudioOut, FilmcraftApp, PlaybackTarget};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
struct Items {
    video: ItemId,
    audio: ItemId,
    subclip: ItemId,
    sequence: ItemId,
    program: ItemId,
}

fn fixture() -> (Session, Items) {
    let mut session = Session::default();
    let mut p = Project::new("Source transport regression");
    let rate = FrameRate::FPS_25;
    let duration = Tick::from_units(8, 1);
    let add = |p: &mut Project, name| {
        filmcraft_engine::demo::add_generator(p, &session.media, GeneratorSource::new(Generator::BarsAndTone, 64, 36, rate, duration), name, Label::Iris, None)
    };
    let video = add(&mut p, "Video and tone");
    let audio = add(&mut p, "Audio only");
    if let ItemKind::Media(m) = &mut p.item_mut(audio).unwrap().kind {
        m.info.video = None;
        m.info.kind = filmcraft_media::MediaKind::AudioOnly;
    }
    let subclip = p.add_item(
        "Restricted subclip",
        Label::Iris,
        ItemKind::Subclip { parent: video, range: TimeRange::new(Tick::from_units(2, 1), Tick::from_units(3, 1)), restrict_trims: true },
        None,
    );
    let program = p.new_sequence("Program", SequenceSettings { width: 64, height: 36, frame_rate: rate, ..Default::default() }, 1, 1, None);
    for (start, source_in) in [(0, 1), (4, 2)] {
        let range = TimeRange::new(Tick::from_units(source_in, 1), Tick::from_units(4, 1));
        let v = p.make_track_item(video, TrackKind::Video, Tick::from_units(start, 1), range, rate).unwrap();
        let a = p.make_track_item(audio, TrackKind::Audio, Tick::from_units(start, 1), range, rate).unwrap();
        let seq = p.sequence_mut(program).unwrap();
        seq.video_tracks[0].items.push(v);
        seq.audio_tracks[0].items.push(a);
    }
    let source_rate = FrameRate::FPS_30;
    let sequence = p.new_sequence(
        "Source sequence",
        SequenceSettings { width: 64, height: 36, frame_rate: source_rate, sample_rate: 32_000, ..Default::default() },
        1,
        1,
        None,
    );
    let range = TimeRange::new(Tick::ZERO, Tick::from_units(4, 1));
    let v = p.make_track_item(video, TrackKind::Video, Tick::ZERO, range, source_rate).unwrap();
    let a = p.make_track_item(audio, TrackKind::Audio, Tick::ZERO, range, source_rate).unwrap();
    let seq = p.sequence_mut(sequence).unwrap();
    seq.video_tracks[0].items.push(v);
    seq.audio_tracks[0].items.push(a);
    session.project = Arc::new(p);
    session.state.active_sequence = Some(program);
    session.state.open_sequences = vec![program];
    session.set_playhead(Tick::from_units(1, 1));
    (session, Items { video, audio, subclip, sequence, program })
}

struct FakeOut {
    played: AtomicU64,
    per_poll: u64,
    lose_after: Option<u64>,
    running: Arc<AtomicBool>,
    first_peak: Arc<AtomicU32>,
}

impl AudioOut for FakeOut {
    fn start(&mut self, mut fill: Box<dyn FnMut(&mut [f32], usize) + Send>) -> Result<u32, String> {
        self.played.store(0, Ordering::SeqCst);
        self.running.store(true, Ordering::SeqCst);
        let mut buf = [0.0; 1024];
        fill(&mut buf, 2);
        let peak = buf.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
        self.first_peak.store(peak.to_bits(), Ordering::SeqCst);
        Ok(48_000)
    }
    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
    fn sample_rate(&self) -> u32 {
        48_000
    }
    fn played_frames(&self) -> Option<u64> {
        if !self.running.load(Ordering::SeqCst) {
            return None;
        }
        let played = self.played.fetch_add(self.per_poll, Ordering::SeqCst) + self.per_poll;
        (!self.lose_after.is_some_and(|limit| played >= limit)).then_some(played)
    }
}

struct Driver {
    h: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    running: Arc<AtomicBool>,
    peak: Arc<AtomicU32>,
    items: Items,
}

impl Driver {
    fn new(per_poll: u64) -> Self {
        let (session, items) = fixture();
        let (tx, rx) = channel();
        let running = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU32::new(0));
        let mut app = FilmcraftApp::new(session).with_control(rx);
        app.audio = Some(Box::new(FakeOut { played: AtomicU64::new(0), per_poll, lose_after: None, running: running.clone(), first_peak: peak.clone() }));
        let h = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_step_dt(0.25).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Self { h, tx, running, peak, items };
        d.frames(3);
        d
    }
    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.h.ctx.clone();
            let mut raw = std::mem::take(self.h.input_mut());
            eframe::App::raw_input_hook(self.h.state_mut(), &ctx, &mut raw);
            *self.h.input_mut() = raw;
            self.h.step();
        }
    }
    fn call(&mut self, method: &str, params: Value) -> Value {
        let (request, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(request).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(value) = reply.try_recv() {
                return value;
            }
        }
        panic!("no reply to {method} {params}");
    }
    fn ok(&mut self, method: &str, params: Value) -> Value {
        let reply = self.call(method, params.clone());
        assert_eq!(reply["ok"], true, "{method} {params}: {reply}");
        reply["result"].clone()
    }
    fn open(&mut self, item: ItemId) {
        self.ok("engine.execute", json!({"command": "source.open", "params": {"item": item.0}}));
        self.frames(2);
    }
    fn key(&mut self, key: &str) {
        self.ok("ui.key", json!({"key": key}));
    }
}

#[test]
fn source_video_audio_subclip_and_sequence_use_their_own_healthy_or_stalled_clock() {
    for per_poll in [4_800, 0] {
        for index in 0..4 {
            let mut d = Driver::new(per_poll);
            let item = [d.items.video, d.items.audio, d.items.subclip, d.items.sequence][index];
            d.open(item);
            let program = d.h.state().session.playhead();
            let view = filmcraft_engine::clip_ops::source_view(&d.h.state().session, item).unwrap();
            let initial = d.h.state().session.state.source_playhead;
            d.ok("ui.playback", json!({"target": "source", "action": "play"}));
            d.frames(if per_poll == 0 { 12 } else { 8 });
            let app = d.h.state();
            assert!(app.source_is_playing(), "Source item {index} with {per_poll} frames per poll must still play");
            assert!(app.session.state.source_playhead > initial + Tick::from_seconds_f64(0.2));
            assert!(app.session.state.source_playhead >= view.start && app.session.state.source_playhead < view.end);
            assert_eq!(app.session.playhead(), program, "Source playback must preserve Program time");
            assert_eq!(app.playback.audio_clock, per_poll != 0);
            assert_eq!(app.playback.target, Some(PlaybackTarget::Source(item)));
            assert!(!app.session.mixrec.active());
            assert!(!app.session.mcrec.active());
            assert!(f32::from_bits(d.peak.load(Ordering::SeqCst)) > 0.01, "the Source item must play its tone/mix");
            assert_eq!(d.running.load(Ordering::SeqCst), per_poll != 0, "stalled audio stops when the wall clock takes over");
            d.ok("ui.playback", json!({"target": "source", "action": "stop"}));
            let stopped = d.h.state().session.state.source_playhead;
            d.frames(4);
            assert_eq!(d.h.state().session.state.source_playhead, stopped);
            assert!(!d.running.load(Ordering::SeqCst));
        }
    }
}

#[test]
fn source_button_and_space_jkl_work_and_program_button_switches_the_target() {
    let mut d = Driver::new(4_800);
    d.open(d.items.video);
    let program = d.h.state().session.playhead();
    d.ok("ui.click", json!({"id": "source.transport.src.play"}));
    d.frames(4);
    assert!(d.h.state().source_is_playing());
    assert_eq!(d.h.state().session.playhead(), program);
    d.key("Space");
    assert!(!d.h.state().playback.playing);
    d.key("L");
    assert!(d.h.state().source_is_playing());
    d.key("L");
    assert_eq!(d.h.state().playback.speed, 2.0);
    d.key("K");
    assert!(!d.h.state().playback.playing);
    d.key("J");
    assert_eq!(d.h.state().playback.speed, -1.0);
    d.key("K");
    d.key("Space");
    let source = d.h.state().session.state.source_playhead;
    // The click is routed before the dock processes its focus action, so the button must name Program.
    d.ok("ui.click", json!({"id": "program.transport.playback.toggle"}));
    assert!(d.h.state().program_is_playing());
    assert_eq!(d.h.state().playback.target, Some(PlaybackTarget::Program(d.items.program)));
    let stopped_source = d.h.state().session.state.source_playhead;
    assert!(stopped_source >= source);
    d.frames(5);
    assert!(d.h.state().session.playhead() > program);
    assert_eq!(d.h.state().session.state.source_playhead, stopped_source);
}

#[test]
fn restricted_source_playback_stops_at_both_bounds_and_in_out_controls_its_range() {
    let mut d = Driver::new(4_800);
    d.open(d.items.subclip);
    let view = filmcraft_engine::clip_ops::source_view(&d.h.state().session, d.items.subclip).unwrap();
    let program = d.h.state().session.playhead();
    d.h.state_mut().session.prefs.timeline.return_to_beginning = false;
    d.h.state_mut().session.execute("source.setPlayhead", json!({"time": (view.end - view.rate.frame_duration()).0})).unwrap();
    d.h.state_mut().play_source(1.0).unwrap();
    d.frames(8);
    assert!(!d.h.state().playback.playing);
    assert_eq!(d.h.state().session.state.source_playhead, view.rate.snap(view.end - view.rate.frame_duration()));
    d.h.state_mut().session.execute("source.setPlayhead", json!({"time": (view.start + view.rate.frame_duration()).0})).unwrap();
    d.h.state_mut().play_source(-1.0).unwrap();
    d.frames(8);
    assert!(!d.h.state().playback.playing);
    assert_eq!(d.h.state().session.state.source_playhead, view.start);
    assert_eq!(d.h.state().session.playhead(), program);

    d.open(d.items.video);
    d.h.state_mut().session.execute("source.setPlayhead", json!({"seconds": 2.0})).unwrap();
    d.h.state_mut().session.execute("markers.markIn", json!({"target": "source"})).unwrap();
    d.h.state_mut().session.execute("source.setPlayhead", json!({"seconds": 2.4})).unwrap();
    d.h.state_mut().session.execute("markers.markOut", json!({"target": "source"})).unwrap();
    let source_out = filmcraft_engine::clip_ops::source_view(&d.h.state().session, d.items.video).unwrap().mark_out.unwrap();
    d.ok("ui.menu.invoke", json!({"id": "playback.inToOut", "params": {"target": "source"}}));
    d.frames(12);
    assert!(!d.h.state().playback.playing);
    assert_eq!(d.h.state().session.state.source_playhead, source_out, "the stopped picture must be exactly the marked Out");
    assert_eq!(d.h.state().session.playhead(), program);
}

#[test]
fn source_change_reopen_removal_and_panel_close_stop_the_old_audio_clock() {
    for change in 0..4 {
        let mut d = Driver::new(4_800);
        d.open(d.items.video);
        d.h.state_mut().play_source(1.0).unwrap();
        d.frames(6);
        assert!(d.running.load(Ordering::SeqCst));
        match change {
            0 => d.open(d.items.audio),
            1 => d.open(d.items.video),
            2 => d.h.state_mut().session.state.source_item = None,
            _ => d.h.state_mut().ui.dock.close(PanelKind::Source),
        }
        d.frames(1);
        assert!(!d.h.state().playback.playing, "Source transition {change}");
        assert!(!d.running.load(Ordering::SeqCst));
        let source = d.h.state().session.state.source_playhead;
        d.frames(4);
        assert_eq!(d.h.state().session.state.source_playhead, source);
    }
}

#[test]
fn source_plays_without_an_open_program_and_rejects_invalid_or_empty_targets() {
    let mut d = Driver::new(4_800);
    for speed in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, 9.0, -9.0] {
        assert!(d.h.state_mut().play_source(speed).is_err());
        assert!(!d.h.state().playback.playing);
    }
    let error = d.call("ui.playback", json!({"target": "source", "action": "play"}));
    assert_eq!(error["ok"], false);
    assert_eq!(d.call("ui.playback", json!({"target": "bogus"}))["ok"], false);
    d.open(d.items.audio);
    d.h.state_mut().session.state.active_sequence = None;
    d.h.state_mut().session.state.open_sequences.clear();
    d.key("Space");
    d.frames(6);
    assert!(d.h.state().source_is_playing());
    assert!(d.h.state().session.state.source_playhead > Tick::ZERO);
    assert!(d.running.load(Ordering::SeqCst));
    d.key("K");
    d.open(d.items.sequence);
    let sequence = d.items.sequence;
    let seq = Arc::make_mut(&mut d.h.state_mut().session.project).sequence_mut(sequence).unwrap();
    seq.video_tracks[0].items.clear();
    seq.audio_tracks[0].items.clear();
    assert!(d.h.state_mut().play_source(1.0).is_err());
    d.h.state_mut().ui.dock.close(PanelKind::Source);
    assert!(d.h.state_mut().play_source(1.0).is_err());
    assert!(!d.h.state().playback.playing);
}

#[test]
fn source_preroll_postroll_and_to_out_keep_program_time_unchanged() {
    for command in ["playback.inToOutPreroll", "playback.toOut"] {
        let mut d = Driver::new(4_800);
        d.open(d.items.video);
        d.h.state_mut().session.prefs.playback.preroll_seconds = 0.5;
        d.h.state_mut().session.prefs.playback.postroll_seconds = 0.5;
        d.h.state_mut().session.execute("source.setPlayhead", json!({"seconds": 2.0})).unwrap();
        d.h.state_mut().session.execute("markers.markIn", json!({"target": "source"})).unwrap();
        d.h.state_mut().session.execute("source.setPlayhead", json!({"seconds": 3.0})).unwrap();
        d.h.state_mut().session.execute("markers.markOut", json!({"target": "source"})).unwrap();
        d.h.state_mut().session.execute("source.setPlayhead", json!({"seconds": 2.5})).unwrap();
        let program = d.h.state().session.playhead();
        let source = d.h.state().session.state.source_playhead;
        d.ok("ui.menu.invoke", json!({"id": command, "params": {"target": "source"}}));
        let app = d.h.state();
        assert_eq!(app.playback.target, Some(PlaybackTarget::Source(d.items.video)));
        let expected_stop = Tick::from_seconds_f64(if command == "playback.toOut" { 3.04 } else { 3.54 });
        assert_eq!(app.playback.stop_at, Some(expected_stop));
        if command == "playback.toOut" {
            assert!(app.session.state.source_playhead >= source);
        } else {
            assert!(app.session.state.source_playhead < Tick::from_units(2, 1));
        }
        d.frames(24);
        assert!(!d.h.state().playback.playing);
        assert_eq!(d.h.state().session.playhead(), program);
    }
}

#[test]
fn terminal_one_frame_source_ranges_and_loops_stay_at_the_marked_frame() {
    for looping in [false, true] {
        let mut d = Driver::new(4_800);
        d.open(d.items.video);
        let view = filmcraft_engine::clip_ops::source_view(&d.h.state().session, d.items.video).unwrap();
        let last = view.end - view.rate.frame_duration();
        d.h.state_mut().session.execute("source.setPlayhead", json!({"time": last.0})).unwrap();
        d.h.state_mut().session.execute("markers.markIn", json!({"target": "source"})).unwrap();
        d.h.state_mut().session.execute("markers.markOut", json!({"target": "source"})).unwrap();
        let program = d.h.state().session.playhead();
        d.h.state_mut().playback.looping = looping;
        d.ok("ui.menu.invoke", json!({"id": "playback.inToOut", "params": {"target": "source"}}));
        for _ in 0..8 {
            d.frames(1);
            assert_eq!(d.h.state().session.state.source_playhead, last, "an explicit terminal range must not return to the source start");
        }
        assert_eq!(d.h.state().playback.playing, looping);
        assert_eq!(d.h.state().session.playhead(), program);
    }
}

#[test]
fn a_source_device_losing_its_clock_continues_without_sound_and_does_not_fill_the_journal() {
    let mut d = Driver::new(4_800);
    d.open(d.items.video);
    d.h.state_mut().audio = Some(Box::new(FakeOut {
        played: AtomicU64::new(0),
        per_poll: 4_800,
        lose_after: Some(14_400),
        running: d.running.clone(),
        first_peak: d.peak.clone(),
    }));
    let program = d.h.state().session.playhead();
    d.h.state_mut().play_source(1.0).unwrap();
    let journal = d.h.state().session.journal.len();
    d.frames(12);
    assert!(d.h.state().source_is_playing());
    assert!(!d.h.state().playback.audio_clock);
    assert!(!d.running.load(Ordering::SeqCst));
    assert!(d.h.state().session.state.source_playhead > Tick::from_units(1, 1));
    assert_eq!(d.h.state().session.playhead(), program);
    assert_eq!(d.h.state().session.journal.len(), journal, "playback clock updates must not evict edit commands from the journal");
}

#[test]
fn source_playback_commits_a_running_dynamic_trim_and_leaves_the_edit_unchanged_afterward() {
    let mut d = Driver::new(4_800);
    d.open(d.items.video);
    let clip = d.h.state().session.active_sequence().unwrap().video_tracks[0].items[0].id;
    d.h.state_mut().session.execute("trim.selectEditPoint", json!({"clip": clip.0, "edge": "out", "kind": "roll"})).unwrap();
    d.h.state_mut().ui.focused = PanelKind::Timeline;
    d.key("L");
    d.frames(2);
    assert!(d.h.state().session.trim_play.dynamic.is_some());
    d.h.state_mut().ui.focused = PanelKind::Source;
    d.key("Space");
    assert!(d.h.state().source_is_playing());
    assert!(!d.h.state().session.trim_play.active());
    let edited = serde_json::to_value(&d.h.state().session.project).unwrap();
    let program = d.h.state().session.playhead();
    d.frames(8);
    d.key("K");
    assert_eq!(serde_json::to_value(&d.h.state().session.project).unwrap(), edited);
    assert_eq!(d.h.state().session.playhead(), program);
}

#[test]
fn source_play_around_and_mixer_buttons_do_not_inherit_the_other_monitors_target() {
    let mut d = Driver::new(4_800);
    d.open(d.items.video);
    d.h.state_mut().session.execute("source.setPlayhead", json!({"seconds": 2.0})).unwrap();
    d.h.state_mut().session.prefs.playback.preroll_seconds = 0.5;
    d.h.state_mut().session.prefs.playback.postroll_seconds = 0.5;
    let program = d.h.state().session.playhead();
    d.ok("ui.menu.invoke", json!({"id": "playback.playAround", "params": {"target": "source"}}));
    assert!(d.h.state().source_is_playing());
    d.frames(16);
    assert!(!d.h.state().playback.playing);
    assert!(!d.h.state().session.trim_play.active());
    assert_eq!(d.h.state().session.playhead(), program);

    d.ok("ui.menu.invoke", json!({"id": "window.panel.audioTrackMixer"}));
    d.h.state_mut().ui.focused = PanelKind::Source;
    d.ok("ui.click", json!({"id": "mixer.transport.playback.toggle"}));
    assert!(d.h.state().program_is_playing());
    assert_eq!(d.h.state().playback.target, Some(PlaybackTarget::Program(d.items.program)));
}

#[test]
fn invalid_program_control_requests_report_errors_and_keep_source_playing() {
    let mut d = Driver::new(4_800);
    d.open(d.items.audio);
    d.h.state_mut().play_source(1.0).unwrap();
    d.frames(4);
    assert_eq!(d.call("ui.playback", json!({"target": "program", "action": "play", "speed": 0.0}))["ok"], false);
    assert!(d.h.state().source_is_playing());
    d.h.state_mut().session.state.active_sequence = None;
    d.h.state_mut().session.state.open_sequences.clear();
    assert_eq!(d.call("ui.playback", json!({"target": "program", "action": "play"}))["ok"], false);
    assert!(d.h.state().source_is_playing());
}

#[test]
fn clicking_source_waveform_stops_its_clock_and_preserves_program_time() {
    let mut d = Driver::new(4_800);
    d.open(d.items.audio);
    let program = d.h.state().session.playhead();
    d.ok("ui.playback", json!({"target": "source", "action": "play"}));
    d.frames(4);
    assert!(d.h.state().source_is_playing());
    let elements = d.ok("ui.elements", json!({"prefix": "source.waveform"}));
    let wave = &elements.as_array().unwrap().iter().find(|e| e["id"] == "source.waveform").unwrap()["rect"];
    let x = wave[0].as_f64().unwrap() + wave[2].as_f64().unwrap() * 0.75;
    let y = wave[1].as_f64().unwrap() + wave[3].as_f64().unwrap() * 0.5;
    d.ok("ui.click", json!({"x": x, "y": y}));
    assert!(!d.h.state().source_is_playing());
    assert!(!d.running.load(Ordering::SeqCst));
    let seek = d.h.state().session.state.source_playhead;
    assert!((seek - Tick::from_units(6, 1)).abs() <= FrameRate::FPS_25.frame_duration());
    d.frames(8);
    assert_eq!(d.h.state().session.state.source_playhead, seek, "the old Source clock must not override the waveform seek");
    assert_eq!(d.h.state().session.playhead(), program);

    // A Source waveform seek also leaves ongoing Program playback alone.
    d.ok("ui.playback", json!({"target": "program", "action": "play"}));
    d.ok("ui.click", json!({"x": x, "y": y}));
    assert!(d.h.state().program_is_playing());
    assert_eq!(d.h.state().playback.target, Some(PlaybackTarget::Program(d.items.program)));
    let seek = d.h.state().session.state.source_playhead;
    d.frames(4);
    assert_eq!(d.h.state().session.state.source_playhead, seek);
    assert!(d.h.state().session.playhead() > program);
}
