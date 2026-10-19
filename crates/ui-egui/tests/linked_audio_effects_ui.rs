//! Linked picture/sound controls and real audio-effect drops on the picture. Tiny generated
//! media keeps these tests independent of files, codecs and audio devices.

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, MediaSource};
use filmcraft_project::{ClipId, ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackItem};
use filmcraft_time::{FrameRate, Tick};
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    picture: ClipId,
    audio: Option<ClipId>,
    item: ItemId,
}

impl Driver {
    fn new(generator: Generator) -> Self {
        let rate = FrameRate::FPS_24;
        let duration = rate.tick_of(96);
        let source = GeneratorSource::new(generator.clone(), 32, 18, rate, duration);
        let mut p = Project::new("Linked audio");
        let item = p.add_item(
            "Clip",
            Label::Iris,
            ItemKind::Media(MediaClip {
                media: MediaRef::Generator(generator),
                info: source.info().clone(),
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
        let seq = p.new_sequence("Cut", SequenceSettings { width: 32, height: 18, frame_rate: rate, ..Default::default() }, 1, 2, None);
        let mut s = Session::default();
        s.project = Arc::new(p);
        s.state.active_sequence = Some(seq);
        s.state.open_sequences = vec![seq];
        s.execute("timeline.place", json!({"item": item.0, "track": "V1", "audioTrack": "A1", "time": 0, "insert": false})).unwrap();
        let q = s.active_sequence().unwrap();
        let picture = q.video_tracks[0].items[0].id;
        let audio = q.audio_tracks[0].items.first().map(|i| i.id);
        assert!(s.state.linked_selection, "regression uses ordinary linked selection");
        s.execute("timeline.select", json!({"clips": [picture.0]})).unwrap();
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(s).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Self { harness, tx, picture, audio, item };
        d.frames(4);
        d.ok("ui.set", json!({"workspace": "Effects"}));
        d.ok("ui.panel.show", json!({"panel": "Effect Controls"}));
        d.frames(3);
        d
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (request, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(request).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                assert_eq!(v["ok"], true, "{method} {params}: {v}");
                return v["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    fn has(&mut self, id: &str) -> bool {
        self.ok("ui.elements", json!({"prefix": id})).as_array().unwrap().iter().any(|e| e["id"] == id)
    }

    fn rect(&mut self, id: &str) -> [f64; 4] {
        let elements = self.ok("ui.elements", json!({"prefix": id}));
        let e = elements.as_array().unwrap().iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("no element {id}: {elements}"));
        std::array::from_fn(|i| e["rect"][i].as_f64().unwrap())
    }

    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(3);
    }

    fn clip(&self, clip: ClipId) -> TrackItem {
        self.harness.state().session.active_sequence().unwrap().find_item(clip).unwrap().1.clone()
    }

    fn undo_len(&mut self) -> usize {
        self.exec("history.list", json!({}))["undo"].as_array().unwrap().len()
    }

    fn selection(&self) -> Vec<ClipId> {
        self.harness.state().session.state.selection.clone()
    }

    fn rms(&self) -> f64 {
        let s = &self.harness.state().session;
        let q = s.active_sequence().unwrap();
        let provider = s.media.provider(s.project.clone(), s.services.clone());
        let audio = filmcraft_render::audio::mix_sequence(&s.project, q, 0, 4800, &provider);
        let count: usize = audio.channels.iter().map(Vec::len).sum();
        (audio.channels.iter().flatten().map(|x| f64::from(*x).powi(2)).sum::<f64>() / count as f64).sqrt()
    }

    fn drop_eq(&mut self, clip: ClipId) {
        self.ok("ui.set", json!({"effectsSearch": "parametric"}));
        self.ok("ui.panel.show", json!({"panel": "Effects"}));
        self.frames(3);
        let point = self.ok("ui.timeline.locate", json!({"clip": clip.0}));
        self.ok("ui.drag", json!({"from": {"id": "effects.item.parametric_eq"}, "to": {"x": point["x"], "y": point["y"]}, "steps": 12}));
        self.frames(5);
    }
}

#[test]
fn selected_linked_picture_shows_video_and_audio_controls_and_edits_the_sound() {
    let mut d = Driver::new(Generator::BarsAndTone);
    let audio = d.audio.unwrap();
    let selected = d.selection();
    assert!(selected.contains(&d.picture) && selected.contains(&audio));
    d.exec("effects.apply", json!({"effect": "parametric_eq", "clips": [audio.0]}));
    d.ok("ui.set", json!({"focused": "Effect Controls"}));
    d.ok("ui.menu.invoke", json!({"id": "window.maximizeFrame"}));
    for id in ["effectControls.effect.motion", "effectControls.effect.volume", "effectControls.volume.level.value", "effectControls.effect.parametric_eq.edit"]
    {
        assert!(d.has(id), "{id} appears for a linked picture selection");
    }
    let before = d.clip(audio);
    let picture = d.clip(d.picture);
    let rms = d.rms();
    assert!(rms > 0.001, "fixture supplies actual audible samples");
    let undo = d.undo_len();
    let r = d.rect("effectControls.volume.level.value");
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    d.ok("ui.drag", json!({"from": {"x": x, "y": y}, "to": {"x": x - 30.0, "y": y}, "steps": 6}));
    d.frames(3);
    let after = d.clip(audio);
    assert!(after.effect("volume").unwrap().params["level"].value.as_f64().unwrap() < 0.0);
    assert!(d.rms() < rms * 0.8, "the visible Volume control changes the actual mix");
    assert_eq!(d.clip(d.picture).effects, picture.effects);
    assert_eq!(d.selection(), selected);
    assert_eq!(d.undo_len(), undo + 1, "one gesture, one undo");
    d.exec("edit.undo", json!({}));
    assert_eq!(d.clip(audio).effects, before.effects);
    assert!((d.rms() - rms).abs() < 1e-6);
    d.exec("edit.redo", json!({}));
    assert_eq!(d.clip(audio).effects, after.effects);
    d.click("effectControls.effect.parametric_eq.edit");
    assert!(d.has("fxEditor.parametric_eq.plot"), "linked audio opens its existing EQ editor");
}

#[test]
fn audio_effect_dropped_on_picture_edits_only_its_linked_unlocked_audio_and_undoes() {
    let mut d = Driver::new(Generator::BarsAndTone);
    let audio = d.audio.unwrap();
    let picture = d.clip(d.picture);
    let before = d.clip(audio);
    let selected = d.selection();
    let undo = d.undo_len();
    d.drop_eq(d.picture);
    assert!(d.clip(audio).effect("parametric_eq").is_some());
    assert_eq!(d.clip(d.picture).effects, picture.effects);
    assert_eq!(d.selection(), selected);
    assert_eq!(d.undo_len(), undo + 1);
    d.exec("edit.undo", json!({}));
    assert_eq!(d.clip(audio).effects, before.effects);
    d.exec("edit.redo", json!({}));
    assert!(d.clip(audio).effect("parametric_eq").is_some());
}

#[test]
fn audio_effect_drop_refuses_locked_unlinked_and_silent_picture_targets() {
    for case in ["locked", "unlinked", "silent"] {
        let mut d = Driver::new(if case == "silent" { Generator::BlackVideo } else { Generator::BarsAndTone });
        match case {
            "locked" => {
                d.exec("timeline.setTrack", json!({"track": "A1", "locked": true}));
            }
            "unlinked" => {
                d.exec("clip.link", json!({"clips": [d.picture.0, d.audio.unwrap().0]}));
            }
            _ => {}
        }
        let project = d.harness.state().session.project.clone();
        let selected = d.selection();
        let undo = d.undo_len();
        d.drop_eq(d.picture);
        assert_eq!(d.undo_len(), undo, "{case} drop must not create a no-op edit");
        assert!(Arc::ptr_eq(&project, &d.harness.state().session.project), "{case} leaves the project intact");
        assert_eq!(d.selection(), selected);
        let error = &d.harness.state().ui.status;
        assert!(if case == "locked" { error.contains("locked") } else { error.contains("no linked audio") }, "{case}: {error}");
    }
}

#[test]
fn independently_selected_audio_still_works_and_unrelated_audio_is_not_shown() {
    let mut d = Driver::new(Generator::BarsAndTone);
    let audio = d.audio.unwrap();
    d.exec("sequence.linkedSelection", json!({"on": false}));
    d.exec("timeline.select", json!({"clips": [audio.0]}));
    assert!(d.has("effectControls.volume.level.value"));
    assert!(!d.has("effectControls.effect.motion"));
    d.drop_eq(audio);
    assert!(d.clip(audio).effect("parametric_eq").is_some());
    assert!(d.clip(d.picture).effect("parametric_eq").is_none());
    d.exec("timeline.place", json!({"item": d.item.0, "track": "V1", "audioTrack": "A2", "time": Tick::from_seconds_f64(8.0).0, "insert": false}));
    let other = d.harness.state().session.active_sequence().unwrap().audio_tracks[1].items[0].id;
    d.exec("timeline.select", json!({"clips": [d.picture.0, other.0]}));
    assert!(d.has("effectControls.effect.motion"));
    assert!(!d.has("effectControls.effect.volume"), "an unrelated selected sound is not the picture's controls");
}

#[test]
fn linked_audio_animation_uses_its_own_trim_speed_and_source_offset() {
    let mut d = Driver::new(Generator::BarsAndTone);
    let audio = d.audio.unwrap();
    d.harness
        .state_mut()
        .session
        .edit_sequence("Fixture sound timing", |q, _, _| {
            let sound = q.find_item_mut(audio).unwrap().1;
            sound.source_in = Tick::from_seconds_f64(1.0);
            sound.duration = Tick::from_seconds_f64(2.0);
            sound.speed = 1.5;
            sound.reverse = true;
            Ok(())
        })
        .unwrap();
    d.exec("playhead.set", json!({"seconds": 0.5}));
    d.ok("ui.set", json!({"focused": "Effect Controls"}));
    d.ok("ui.menu.invoke", json!({"id": "window.maximizeFrame"}));
    let time = d.harness.state().session.playhead();
    let before = d.clip(audio);
    d.click("effectControls.volume.level.stopwatch");
    let after = d.clip(audio);
    let keys = &after.effect("volume").unwrap().params["level"].keyframes;
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].time, before.source_time_at(time), "audio controls evaluate the sound's own media time");
    let lane = d.rect("effectControls.lane");
    let key = d.rect(&format!("effectControls.volume.level.keyframe.{}", keys[0].time.0));
    let fraction = (time - d.clip(d.picture).start).seconds() / d.clip(d.picture).duration.seconds();
    assert!((key[0] + key[2] / 2.0 - lane[0] - lane[2] * fraction).abs() < 0.05, "reversed linked sound diamond is at the actual sequence playhead");
    d.click("effectControls.volume.level.graphs");
    let graph_key = d.rect(&format!("effectControls.level.graph.keyframe.{}", keys[0].time.0));
    assert!(
        (graph_key[0] + graph_key[2] / 2.0 - lane[0] - lane[2] * fraction).abs() < 0.05,
        "linked sound graph uses its own timing in the picture's sequence ruler"
    );
    assert_eq!(d.clip(d.picture).source_in, Tick::ZERO);
    d.exec("edit.undo", json!({}));
    assert_eq!(d.clip(audio).effects, before.effects);
}
