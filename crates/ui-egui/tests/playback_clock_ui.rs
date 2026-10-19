//! Playback clock: the audio device drives the playhead while it plays, and a device that opens
//! but never consumes samples (seen on Linux with ALSA, #136) must not freeze playback.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_time::Tick;
use filmcraft_ui_egui::{AudioOut, FilmcraftApp};
use serde_json::json;

/// An output whose stream opens, then plays `per_poll` frames each time the clock is read
/// (0 = a stalled device that never calls back).
struct FakeOut {
    played: Arc<AtomicU64>,
    per_poll: u64,
    started: bool,
}

impl AudioOut for FakeOut {
    fn start(&mut self, _fill: Box<dyn FnMut(&mut [f32], usize) + Send>) -> Result<u32, String> {
        self.played.store(0, Ordering::SeqCst);
        self.started = true;
        Ok(48_000)
    }
    fn stop(&mut self) {
        self.started = false;
    }
    fn sample_rate(&self) -> u32 {
        48_000
    }
    fn played_frames(&self) -> Option<u64> {
        self.started.then(|| self.played.fetch_add(self.per_poll, Ordering::SeqCst) + self.per_poll)
    }
}

/// Play for `steps` × 0.25 s of simulated time with the given output; the playhead in seconds.
fn play_with(per_poll: u64, steps: usize) -> (f64, bool) {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.set_playhead(Tick::ZERO);
    let mut app = FilmcraftApp::new(s);
    app.audio = Some(Box::new(FakeOut { played: Arc::new(AtomicU64::new(0)), per_poll, started: false }));
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_step_dt(0.25).build_eframe(move |_cc| app);
    h.step();
    h.state_mut().play(1.0);
    for _ in 0..steps {
        h.step();
    }
    let app = h.state_mut();
    (app.session.playhead().seconds(), app.playback.audio_clock)
}

/// #136: the stream opened, the device never asked for samples, the audio clock stayed at 0 and
/// the playhead never moved. After a short grace period playback falls back to the wall clock.
#[test]
fn a_stalled_audio_device_does_not_freeze_playback() {
    let (t, audio_clock) = play_with(0, 16);
    assert!(t > 1.5, "the playhead should move with the wall clock, it is at {t:.2} s");
    assert!(!audio_clock, "the stalled device no longer drives the clock");
}

/// Control: a device that plays keeps driving the clock (0.1 s of audio per poll).
#[test]
fn a_playing_audio_device_drives_the_clock() {
    let (t, audio_clock) = play_with(4_800, 8);
    assert!(t > 0.2, "{t:.2} s");
    assert!(audio_clock);
}

/// Replacing a project must stop its audio callback and clock before reused sequence ids point
/// at different media. Test both editor focus scopes: source focus must not keep an old stream.
#[test]
fn replacing_the_project_stops_its_audio_and_playhead_clock() {
    use filmcraft_ui_egui::dock::PanelKind;

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/playback-replacement-tests");
    std::fs::create_dir_all(&dir).unwrap();
    for focused in [PanelKind::Program, PanelKind::Source] {
        for command in ["file.newProject", "file.openDemoProject", "file.open"] {
            let mut session = Session::default();
            session.execute("file.openDemoProject", json!({})).unwrap();
            session.set_playhead(Tick::ZERO);
            let path = dir.join(format!("saved-{}-{}-{}.fcproj", std::process::id(), focused.id(), command.replace('.', "-")));
            session.execute("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
            if focused == PanelKind::Source {
                let source = session.active_sequence().unwrap().video_tracks[0].items[0].item;
                session.execute("source.open", json!({"item": source.0})).unwrap();
            }
            let mut app = FilmcraftApp::new(session);
            app.ui.focused = focused;
            app.audio = Some(Box::new(FakeOut { played: Arc::new(AtomicU64::new(0)), per_poll: 4_800, started: false }));
            let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_step_dt(0.25).build_eframe(move |_cc| app);
            h.step();
            if focused == PanelKind::Source {
                h.state_mut().play_source(1.0).unwrap();
            } else {
                h.state_mut().play(1.0);
            }
            for _ in 0..12 {
                h.step();
            }
            assert!(h.state_mut().playback.playing, "playback started before {command}");
            if focused == PanelKind::Source {
                assert!(h.state().source_is_playing());
            }
            assert!(h.state_mut().audio.as_ref().unwrap().played_frames().is_some(), "old audio stream started before {command}");
            h.state_mut().session.execute(command, json!({"path": path.to_string_lossy()})).unwrap();
            let replacement_playhead = h.state_mut().session.playhead();
            let replacement_source_playhead = h.state().session.state.source_playhead;
            h.step();
            assert!(!h.state_mut().playback.playing, "{command} stops playback with {focused:?} focused");
            assert!(!h.state_mut().playback.audio_clock);
            assert!(h.state_mut().playback.preroll.is_none());
            assert!(h.state_mut().audio.as_ref().unwrap().played_frames().is_none(), "{command} closes the old audio stream");
            for _ in 0..8 {
                h.step();
            }
            assert_eq!(h.state_mut().session.playhead(), replacement_playhead, "the old clock must not seek in the replacement project after {command}");
            assert_eq!(h.state().session.state.source_playhead, replacement_source_playhead, "the old Source clock must not seek after {command}");
            std::fs::remove_file(path).unwrap();
        }
    }
}
