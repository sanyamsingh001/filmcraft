//! A caller that names its targets in the parameters (`clips`, `clip`, `item`, `items`) does not
//! need a selection: enablement is checked against the named targets. Without such parameters
//! (the menus, shortcuts and `engine.commands`) enablement is the selection's, as before.

use filmcraft_project::{ClipId, ItemId, TrackItem};
use serde_json::json;

use crate::{EngineError, Session};

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.execute("sequence.linkedSelection", json!({"on": false})).unwrap();
    s.state.selection.clear();
    s.state.project_selection.clear();
    s
}

fn item_named(s: &Session, name: &str) -> ItemId {
    s.project.items.values().find(|i| i.name == name).map(|i| i.id).unwrap_or_else(|| panic!("no item {name}"))
}

fn v1(s: &Session) -> Vec<TrackItem> {
    s.active_sequence().unwrap().video_tracks[0].items.clone()
}

fn a1(s: &Session) -> Vec<TrackItem> {
    s.active_sequence().unwrap().audio_tracks[0].items.clone()
}

fn clip(s: &Session, id: ClipId) -> Option<TrackItem> {
    s.active_sequence().unwrap().find_item(id).map(|(_, i)| i.clone())
}

fn disabled(r: crate::Result<serde_json::Value>) -> String {
    match r {
        Err(EngineError::Disabled(_, why)) => why,
        other => panic!("expected a disabled command, got {other:?}"),
    }
}

#[test]
fn replace_from_bin_takes_an_explicit_item_and_clips_without_any_selection() {
    let mut s = demo();
    let c = v1(&s)[0].clone();
    let dunes = item_named(&s, "Desert_Dunes.mp4");
    assert_ne!(c.item, dunes);
    // nothing named, nothing selected: disabled, for the menus and for a caller alike
    assert!(!s.is_enabled("clip.replaceFromBin"));
    assert_eq!(disabled(s.execute("clip.replaceFromBin", json!({}))), "no clips selected");
    // the clip is named but the replacement is not: still the Project panel's call
    assert_eq!(disabled(s.execute("clip.replaceFromBin", json!({"clips": [c.id.0]}))), "select a clip in the Project panel");
    // both named: runs with no timeline and no Project-panel selection
    s.execute("clip.replaceFromBin", json!({"clips": [c.id.0], "item": dunes.0})).unwrap();
    assert_eq!(clip(&s, c.id).unwrap().item, dunes);
    assert!(s.state.selection.is_empty() && s.state.project_selection.is_empty(), "the selections are left as they were");
    assert!(!s.is_enabled("clip.replaceFromBin"), "menu enablement still follows the selection");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(clip(&s, c.id).unwrap().item, c.item);
    // a timeline selection plus an explicit item (no Project-panel selection)
    s.execute("timeline.select", json!({"clips": [c.id.0]})).unwrap();
    s.execute("clip.replaceFromBin", json!({"item": dunes.0})).unwrap();
    assert_eq!(clip(&s, c.id).unwrap().item, dunes);
}

#[test]
fn clip_commands_take_explicit_clips_without_a_selection() {
    let mut s = demo();
    let (a, b) = (v1(&s)[0].clone(), v1(&s)[1].clone());
    for id in ["edit.clear", "edit.rippleDelete", "clip.audioGain", "edit.pasteAttributes", "clip.enable"] {
        assert!(!s.is_enabled(id), "{id} is disabled without a selection");
        assert!(matches!(s.execute(id, json!({})), Err(EngineError::Disabled(..))), "{id} with no parameters stays disabled");
    }
    // Clear
    s.execute("edit.clear", json!({"clips": [a.id.0]})).unwrap();
    assert!(clip(&s, a.id).is_none() && clip(&s, b.id).is_some());
    s.execute("edit.undo", json!({})).unwrap();
    assert!(clip(&s, a.id).is_some());
    // Ripple Delete
    let a_and_its_audio: Vec<u64> = s
        .active_sequence()
        .unwrap()
        .all_tracks()
        .flat_map(|t| &t.items)
        .filter(|i| i.id == a.id || (i.link.is_some() && i.link == a.link))
        .map(|i| i.id.0)
        .collect();
    s.execute("timeline.setTrack", json!({"track": "A2", "syncLock": false})).unwrap(); // the music on A2 would block the ripple
    s.execute("edit.rippleDelete", json!({"clips": a_and_its_audio})).unwrap();
    assert!(clip(&s, a.id).is_none());
    assert_eq!(clip(&s, b.id).unwrap().start, a.start, "the next clip closes the gap");
    s.execute("edit.undo", json!({})).unwrap();
    // Audio Gain
    let au = a1(&s)[0].clone();
    s.execute("clip.audioGain", json!({"clips": [au.id.0], "mode": "set", "db": -6.0})).unwrap();
    assert_eq!(clip(&s, au.id).unwrap().gain_db, -6.0);
    // Enable
    s.execute("clip.enable", json!({"clips": [a.id.0]})).unwrap();
    assert!(!clip(&s, a.id).unwrap().enabled);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(clip(&s, a.id).unwrap().enabled);
    s.execute("edit.redo", json!({})).unwrap();
    assert!(!clip(&s, a.id).unwrap().enabled);
    // `clip.enable` registers `clips` only: an unregistered key is not a named target
    assert_eq!(disabled(s.execute("clip.enable", json!({"clip": a.id.0}))), "no clips selected");
    // Paste Attributes: naming the clips lifts only the selection condition, not the clipboard one
    assert_eq!(disabled(s.execute("edit.pasteAttributes", json!({"clips": [b.id.0]}))), "copy a clip first");
    s.execute("timeline.select", json!({"clips": [a.id.0]})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.state.selection.clear();
    s.execute("edit.pasteAttributes", json!({"clips": [b.id.0]})).unwrap();
    assert!(s.state.selection.is_empty(), "the selection is left as it was");
    for id in ["edit.clear", "edit.rippleDelete", "clip.audioGain", "edit.pasteAttributes"] {
        assert!(!s.is_enabled(id), "menu enablement of {id} still follows the selection");
    }
}

#[test]
fn naming_nothing_usable_does_not_enable_a_command() {
    let mut s = demo();
    let before = s.project.clone();
    // no ids, ids of no clip, the wrong type, and a key the command does not take
    for p in [json!({"clips": []}), json!({"clips": [987_654_321u64]}), json!({"clips": "all"}), json!({"clips": [-1, 1.5, null]}), json!({"items": [1]})] {
        assert_eq!(disabled(s.execute("edit.rippleDelete", p.clone())), "nothing selected", "{p}");
    }
    assert_eq!(disabled(s.execute("clip.replaceFromBin", json!({"clips": [v1(&s)[0].id.0], "item": 987_654_321u64}))), "select a clip in the Project panel");
    // `edit.cut` takes no `clips`: it works on the selection only
    assert_eq!(disabled(s.execute("edit.cut", json!({"clips": [v1(&s)[0].id.0]}))), "no clips selected");
    assert!(std::sync::Arc::ptr_eq(&before, &s.project), "nothing was edited");
}

#[test]
fn link_and_speed_take_explicit_clips_without_a_selection() {
    let mut s = demo();
    let (v, au) = (v1(&s)[0].clone(), a1(&s)[0].clone());
    for id in ["clip.link", "clip.speedDuration"] {
        assert!(!s.is_enabled(id));
        assert_eq!(disabled(s.execute(id, json!({}))), "no clips selected");
    }
    // Link toggles the named pair
    let was = v.link.is_some() && v.link == au.link;
    let r = s.execute("clip.link", json!({"clips": [v.id.0, au.id.0]})).unwrap();
    assert_eq!(r["linked"], !was);
    assert_eq!(clip(&s, v.id).unwrap().link.is_some(), !was);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(clip(&s, v.id).unwrap().link, v.link);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(clip(&s, v.id).unwrap().link.is_some(), !was);
    s.execute("edit.undo", json!({})).unwrap();
    // Speed/Duration on the last clip of V1 (nothing after it to run into)
    let last = v1(&s).last().unwrap().clone();
    s.execute("clip.speedDuration", json!({"clips": [last.id.0], "speed": 200})).unwrap();
    let fast = clip(&s, last.id).unwrap();
    assert!(fast.duration < last.duration, "{:?} -> {:?}", last.duration, fast.duration);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(clip(&s, last.id).unwrap().duration, last.duration);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(clip(&s, last.id).unwrap().duration, fast.duration);
    assert!(s.state.selection.is_empty() && !s.is_enabled("clip.speedDuration"), "menu enablement still follows the selection");
}

#[test]
fn naming_targets_lifts_only_the_selection_condition() {
    // no sequence: named clips cannot stand in for one
    let mut empty = Session::default();
    assert_eq!(disabled(empty.execute("edit.clear", json!({"clips": [1]}))), "no sequence is open");
    assert_eq!(disabled(empty.execute("clip.speedDuration", json!({"clips": [1], "speed": 50}))), "no sequence is open");

    let mut s = demo();
    let a = v1(&s)[0].clone();
    // the wrong kind of target: a video clip is not a graphic
    assert_eq!(disabled(s.execute("graphics.setText", json!({"clip": a.id.0, "text": "x"}))), "select a graphic clip");
    // a locked track is still the edit's own business: named or selected, the clip stays
    s.execute("timeline.setTrack", json!({"track": "V1", "locked": true})).unwrap();
    let _ = s.execute("edit.clear", json!({"clips": [a.id.0]}));
    assert!(clip(&s, a.id).is_some(), "a clip on a locked track is not removed");
}

#[test]
fn named_target_enablement_uses_metadata_instead_of_parameter_documentation() {
    use crate::commands::{self, TargetKey};

    let mut s = demo();
    let c = v1(&s)[0].id;
    let original = *commands::find("edit.clear").unwrap();
    let mut explicit = original;
    explicit.params = "arbitrary documentation, with no target key";
    assert!(s.enabled_for(&explicit, &json!({"clips": [c.0]})).is_ok());
    assert!(s.state.selection.is_empty() && s.state.project_selection.is_empty());
    assert!(!s.is_enabled("edit.clear"));

    let mut selection_only = original;
    selection_only.params = r#"{"clips":[id],"clip":id,"items":[id],"item":id}"#;
    selection_only.named_targets = &[];
    assert!(s.enabled_for(&selection_only, &json!({"clips": [c.0]})).is_err(), "descriptive text cannot opt a selection-only handler in");
    assert!(commands::named_clips(&s, &selection_only, &json!({"clips": [c.0]})).is_none());

    let mut both = explicit;
    both.named_targets = &[TargetKey::Clips, TargetKey::Clip];
    for plural in [json!([]), json!([u64::MAX]), json!([-1, null, 1.5])] {
        assert!(
            s.enabled_for(&both, &json!({"clips": plural, "clip": c.0})).is_err(),
            "a provided array suppresses the singular key even when no id is usable"
        );
    }
    assert!(s.enabled_for(&both, &json!({"clips": "not an array", "clip": c.0})).is_ok());
    assert!(s.enabled_for(&both, &json!({"clip": c.0})).is_ok());
    assert!(s.enabled_for(&explicit, &json!({"clip": c.0})).is_err(), "a singular alias must be explicitly registered");
    assert!(s.state.selection.is_empty() && s.state.project_selection.is_empty());
}

#[test]
fn every_registered_target_contract_is_independent_of_docs_and_restores_selections() {
    use crate::commands::{self, TargetKey};

    let mut s = demo();
    let c = v1(&s)[0].clone();
    let other = v1(&s)[1].clone();
    let p = json!({"clips": [c.id.0], "clip": c.id.0, "items": [c.item.0], "item": c.item.0});
    // Keep a different Project selection to exercise restoration of both selection domains.
    s.state.project_selection = vec![other.item];
    let before = (s.state.selection.clone(), s.state.project_selection.clone(), s.project.clone());
    for registered in commands::command_specs().iter().filter(|c| !c.named_targets.is_empty()) {
        let mut altered = *registered;
        altered.params = "documentation renamed and reformatted";
        let expected = s.enabled_for(registered, &p);
        let actual = s.enabled_for(&altered, &p);
        assert_eq!(actual, expected, "{}", registered.id);
        assert_eq!(
            (s.state.selection.clone(), s.state.project_selection.clone()),
            (before.0.clone(), before.1.clone()),
            "{} restored selections",
            registered.id
        );
        assert!(std::sync::Arc::ptr_eq(&s.project, &before.2));
        let has_clips = registered.named_targets.iter().any(|k| matches!(k, TargetKey::Clip | TargetKey::Clips));
        let has_items = registered.named_targets.iter().any(|k| matches!(k, TargetKey::Item | TargetKey::Items));
        assert_eq!(commands::named_clips(&s, &altered, &p).is_some(), has_clips, "{}", registered.id);
        assert_eq!(commands::named_items(&s, &altered, &p).is_some(), has_items, "{}", registered.id);
    }
}

#[test]
fn registered_named_targets_match_the_audited_handler_families() {
    use crate::commands::{self, TargetKey};
    use std::collections::BTreeMap;

    let mut expected = BTreeMap::new();
    for id in [
        "edit.clear",
        "edit.rippleDelete",
        "clip.speedDuration",
        "clip.enable",
        "clip.link",
        "clip.audioGain",
        "edit.pasteAttributes",
        "edit.removeAttributes",
        "edit.selectAllMatching",
        "clip.frameHoldOptions",
        "clip.fieldOptions",
        "clip.timeInterpolation.frameSampling",
        "clip.timeInterpolation.frameBlending",
        "clip.timeInterpolation.opticalFlow",
        "clip.setTimeInterpolation",
        "clip.fitToFrame",
        "clip.fillFrame",
        "clip.replaceFromSource",
        "clip.replaceFromSourceMatchFrame",
        "clip.synchronize",
        "clip.multicamEnable",
        "clip.multicamFlatten",
        "sequence.applyDefaultTransitionsToSelection",
        "clip.sceneEditDetection",
    ] {
        expected.insert(id.to_string(), vec![TargetKey::Clips]);
    }
    for id in [
        "project.delete",
        "file.newSequenceFromClip",
        "file.newBinFromSelection",
        "clip.audioChannels",
        "clip.breakoutToMono",
        "clip.extractAudio",
        "clip.interpretFootage",
        "clip.mergeClips",
        "clip.createMulticam",
        "file.exportSelectionProject",
        "file.exportAle",
        "file.mediaProperties",
        "edit.editOriginal",
        "clip.updateMetadata",
        "clip.generateAudioWaveform",
        "clip.automateToSequence",
    ] {
        expected.insert(id.to_string(), vec![TargetKey::Items]);
    }
    for id in ["clip.editSubclip", "clip.modifyTimecode", "clip.setPosterFrame", "clip.clearPosterFrame", "clip.editOffline", "clip.sourceSettings"] {
        expected.insert(id.to_string(), vec![TargetKey::Item]);
    }
    for id in [
        "graphics.setText",
        "graphics.set",
        "graphics.setTextType",
        "graphics.selectLayer",
        "graphics.deleteLayer",
        "graphics.arrangeLayer",
        "graphics.align",
        "graphics.distribute",
        "graphics.resetAllParameters",
        "graphics.resetDuration",
        "graphics.distributeVertically",
        "graphics.distributeSpaceVertically",
        "graphics.distributeHorizontally",
        "graphics.distributeSpaceHorizontally",
        "graphics.template.export",
        "file.exportGraphicsTemplate",
        "graphics.template.set",
        "graphics.setRoll",
        "graphics.setResponsiveTime",
        "graphics.pin",
        "graphics.setCharStyle",
        "graphics.upgradeToSourceGraphic",
        "clip.remix.enable",
        "clip.remix.properties",
        "clip.remix.revert",
        "graphics.bringToFront",
        "graphics.bringForward",
        "graphics.sendBackward",
        "graphics.sendToBack",
    ] {
        expected.insert(id.to_string(), vec![TargetKey::Clip]);
    }
    expected.insert("clip.makeSubclip".into(), vec![TargetKey::Clip, TargetKey::Item]);
    expected.insert("clip.replaceFromBin".into(), vec![TargetKey::Clips, TargetKey::Item]);
    for label in [
        "violet",
        "iris",
        "caribbean",
        "lavender",
        "cerulean",
        "forest",
        "rose",
        "mango",
        "purple",
        "blue",
        "teal",
        "magenta",
        "tan",
        "green",
        "brown",
        "yellow",
    ] {
        expected.insert(format!("edit.label.{label}"), vec![TargetKey::Items, TargetKey::Clips]);
    }
    for domain in ["Frame", "Group", "Selection"] {
        for axis in ["left", "hcenter", "right", "top", "vcenter", "bottom"] {
            expected.insert(format!("graphics.align{domain}.{axis}"), vec![TargetKey::Clip]);
        }
    }
    let actual: BTreeMap<_, _> =
        commands::command_specs().iter().filter(|c| !c.named_targets.is_empty()).map(|c| (c.id.to_string(), c.named_targets.to_vec())).collect();
    assert_eq!(actual, expected, "a new target contract needs a handler audit and a regression fixture");
    // Similar-looking data does not denote a top-level target, and selection-only handlers stay opt-in.
    for id in ["edit.cut", "edit.copy", "timeline.move"] {
        assert!(commands::find(id).unwrap().named_targets.is_empty());
    }
    assert_eq!(commands::find("clip.audioChannels").unwrap().named_targets, &[TargetKey::Items], "clips here is channel-layout data");
}

#[derive(Default)]
struct TargetFiles(std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>);

impl crate::Services for TargetFiles {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        self.0.lock().unwrap().get(path).cloned().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, path.to_string()))
    }
    fn write_file(&self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.0.lock().unwrap().insert(path.to_string(), bytes.to_vec());
        Ok(())
    }
}

fn graphic_target_fixture() -> (Session, ClipId, std::sync::Arc<TargetFiles>) {
    let files = std::sync::Arc::new(TargetFiles::default());
    let mut s = Session::new(files.clone());
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.set_playhead(filmcraft_time::Tick::from_seconds_f64(600.0));
    let c = ClipId(s.execute("graphics.newText", json!({"text": "First", "position": [200, 800]})).unwrap()["clip"].as_u64().unwrap());
    for (text, position) in [("Second", [900, 600]), ("Third", [1500, 300])] {
        s.execute("graphics.newText", json!({"clip": c.0, "text": text, "position": position})).unwrap();
    }
    s.set_playhead(filmcraft_time::Tick::ZERO); // no graphic under the Program playhead
    s.state.selection.clear();
    s.state.project_selection.clear();
    s.state.graphic_layers = vec![0, 1, 2];
    (s, c, files)
}

#[test]
fn every_flagged_graphics_handler_reads_its_named_clip_without_a_selection() {
    use crate::Services;
    use crate::commands::TargetKey;

    // Each variant reaches a real handler with no selected clip or graphic at the playhead.
    // A handler that ignored `clip` would fail, even if its enablement metadata looked correct.
    for spec in crate::commands::command_specs()
        .iter()
        .filter(|c| c.named_targets == [TargetKey::Clip] && (c.id.starts_with("graphics.") || c.id == "file.exportGraphicsTemplate"))
    {
        let (mut s, c, files) = graphic_target_fixture();
        let mut params = json!({"clip": c.0, "layer": 0, "layers": [0, 1, 2], "text": "Changed", "props": {"size": 42}, "type": "paragraph", "to": "frame", "align": "left", "axis": "horizontal", "seconds": 2.0,
            "mode": "roll", "introFrames": 2, "outroFrames": 2, "start": 0, "end": 2, "style": {"size": 42}, "name": "Named graphic", "path": "named.fcgt"});
        if matches!(spec.id, "graphics.arrangeLayer" | "graphics.pin") {
            params["to"] = json!(if spec.id == "graphics.pin" { "frame" } else { "back" });
        }
        if spec.id == "graphics.template.set" {
            s.set_playhead(filmcraft_time::Tick::from_seconds_f64(620.0));
            let templates = s.execute("graphics.template.list", json!({})).unwrap();
            let template = templates.as_array().unwrap().first().unwrap()["id"].clone();
            let result = s.execute("graphics.template.apply", json!({"template": template})).unwrap();
            let t = ClipId(result["clip"].as_u64().unwrap());
            let controls = s.execute("graphics.template.controls", json!({"clip": t.0})).unwrap();
            let control = controls["controls"].as_array().unwrap().iter().find(|v| v["kind"] == "text").unwrap();
            params = json!({"clip": t.0, "control": control["id"], "value": "Changed by named target"});
            s.set_playhead(filmcraft_time::Tick::ZERO);
            s.state.selection.clear();
            s.state.project_selection.clear();
        }
        assert!(!s.is_enabled(spec.id), "{} remains disabled in the menu without a graphic selection", spec.id);
        assert!(matches!(s.execute(spec.id, json!({})), Err(EngineError::Disabled(..))), "{} needs an explicit target", spec.id);
        let result = s.execute(spec.id, params).unwrap_or_else(|e| panic!("{} ignored its named clip: {e}", spec.id));
        if matches!(spec.id, "graphics.template.export" | "file.exportGraphicsTemplate") {
            let exported = filmcraft_project::gtemplate::GraphicsTemplate::from_bytes(&files.read_file("named.fcgt").unwrap()).unwrap();
            assert_eq!(exported.layers.len(), 3);
            assert_eq!(exported.name, "Named graphic");
        }
        if matches!(
            spec.id,
            "graphics.selectLayer"
                | "graphics.arrangeLayer"
                | "graphics.bringToFront"
                | "graphics.bringForward"
                | "graphics.sendBackward"
                | "graphics.sendToBack"
        ) {
            assert_eq!(s.state.selection, vec![c], "selection is this command's intended result");
        } else {
            assert!(s.state.selection.is_empty(), "{} does not change the timeline selection", spec.id);
        }
        if spec.id == "graphics.upgradeToSourceGraphic" {
            assert_eq!(s.state.project_selection, vec![ItemId(result["item"].as_u64().unwrap())]);
        } else {
            assert!(s.state.project_selection.is_empty(), "{} does not change the Project selection", spec.id);
        }
    }
}

#[test]
fn every_label_handler_reads_both_registered_target_domains() {
    for spec in crate::commands::command_specs().iter().filter(|c| c.id.starts_with("edit.label.")) {
        for project_item in [false, true] {
            let mut s = demo();
            let c = v1(&s)[0].clone();
            let params = if project_item { json!({"items": [c.item.0]}) } else { json!({"clips": [c.id.0]}) };
            assert!(!s.is_enabled(spec.id));
            let result = s.execute(spec.id, params).unwrap();
            assert_eq!(result["labelled"], 1, "{}", spec.id);
            let label = if project_item { s.project.item(c.item).unwrap().label } else { clip(&s, c.id).unwrap().label };
            assert_eq!(label.name(), spec.label);
            assert!(s.state.selection.is_empty() && s.state.project_selection.is_empty());
            assert!(!s.is_enabled(spec.id));
        }
    }
}

#[test]
fn item_interpretation_and_poster_handlers_read_named_items() {
    let mut s = demo();
    let item = v1(&s)[0].item;
    assert!(!s.is_enabled("clip.interpretFootage"));
    s.execute("clip.interpretFootage", json!({"items": [item.0], "colorSpace": "rec709"})).unwrap();
    assert_eq!(s.project.item(item).unwrap().as_media().unwrap().interpret.color_space, Some(filmcraft_color::ColorSpace::Rec709));
    assert!(s.state.selection.is_empty() && s.state.project_selection.is_empty());
    for (id, clear) in [("clip.setPosterFrame", false), ("clip.clearPosterFrame", true)] {
        assert!(!s.is_enabled(id));
        s.execute(id, json!({"item": item.0, "frame": 12})).unwrap();
        assert_eq!(crate::keyboard::poster_frame(s.project.item(item).unwrap()).is_none(), clear);
        assert!(s.state.selection.is_empty() && s.state.project_selection.is_empty());
        assert!(!s.is_enabled(id));
    }
}

#[test]
fn project_tool_handlers_and_queries_read_named_items() {
    use crate::Services;
    use filmcraft_project::MediaRef;

    for id in [
        "file.exportSelectionProject",
        "file.exportAle",
        "file.mediaProperties",
        "edit.editOriginal",
        "clip.editOffline",
        "clip.sourceSettings",
        "clip.updateMetadata",
        "clip.generateAudioWaveform",
        "clip.automateToSequence",
    ] {
        let files = std::sync::Arc::new(TargetFiles::default());
        let mut s = Session::new(files.clone());
        s.execute("file.openDemoProject", json!({})).unwrap();
        let target = v1(&s)[0].item;
        let source = s.source(target).unwrap();
        s.media.insert(target, source); // keep the synthetic source while testing file metadata
        std::sync::Arc::make_mut(&mut s.project).item_mut(target).unwrap().as_media_mut().unwrap().media = MediaRef::File { path: "named.mov".into() };
        if id == "clip.editOffline" {
            std::sync::Arc::make_mut(&mut s.project).item_mut(target).unwrap().as_media_mut().unwrap().offline = true;
        }
        s.set_playhead(filmcraft_time::Tick::from_seconds_f64(600.0));
        s.state.selection.clear();
        s.state.project_selection.clear();
        let menu_enabled = s.is_enabled(id);
        assert_eq!(menu_enabled, id == "file.exportAle", "{id}");
        let params = if matches!(id, "clip.editOffline" | "clip.sourceSettings") {
            json!({"item": target.0, "mediaName": "Named offline", "description": "Named description"})
        } else {
            json!({"items": [target.0], "path": "selection.fcproj", "videoTransition": false, "audioTransition": false, "overlapFrames": 0})
        };
        let result = s.execute(id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
        match id {
            "file.exportSelectionProject" => {
                let p = filmcraft_format::decode(&files.read_file("selection.fcproj").unwrap()).unwrap().project;
                assert!(p.item(target).is_some());
                assert_eq!(p.items.len(), 1);
            }
            "file.exportAle" => {
                let bytes = files.read_file("selection.fcproj").unwrap();
                let text = String::from_utf8(bytes).unwrap();
                assert!(text.contains(&s.project.item(target).unwrap().name));
                assert_eq!(result["clips"], 1);
            }
            "file.mediaProperties" => assert_eq!(result[0]["item"], target.0),
            "edit.editOriginal" => assert_eq!(result["paths"], json!(["named.mov"])),
            "clip.editOffline" => {
                assert_eq!(result["item"], target.0);
                assert_eq!(s.project.item(target).unwrap().name, "Named offline");
            }
            "clip.sourceSettings" => assert_eq!(result["item"], target.0),
            "clip.updateMetadata" => assert_eq!(result["written"][0]["item"], target.0),
            "clip.generateAudioWaveform" => assert_eq!(result["items"], json!([target.0])),
            "clip.automateToSequence" => {
                assert!(s.active_sequence().unwrap().all_tracks().flat_map(|t| &t.items).any(|i| i.item == target && i.start.seconds() >= 599.0))
            }
            _ => unreachable!(),
        }
        assert!(s.state.project_selection.is_empty(), "{id}");
        if id != "clip.automateToSequence" {
            assert!(s.state.selection.is_empty(), "{id}");
        }
        assert_eq!(s.is_enabled(id), menu_enabled, "menu enablement remains unchanged: {id}");
    }
}

#[test]
fn video_option_and_replacement_handlers_read_named_clips() {
    use filmcraft_project::TimeInterpolation;
    for id in [
        "edit.removeAttributes",
        "edit.selectAllMatching",
        "clip.frameHoldOptions",
        "clip.fieldOptions",
        "clip.timeInterpolation.frameSampling",
        "clip.timeInterpolation.frameBlending",
        "clip.timeInterpolation.opticalFlow",
        "clip.setTimeInterpolation",
        "clip.fitToFrame",
        "clip.fillFrame",
        "clip.replaceFromSource",
        "clip.replaceFromSourceMatchFrame",
        "sequence.applyDefaultTransitionsToSelection",
    ] {
        let mut s = demo();
        let c = v1(&s)[0].clone();
        let replacement = item_named(&s, "Desert_Dunes.mp4");
        s.state.source_item = Some(replacement);
        s.state.source_playhead = filmcraft_time::Tick::ZERO;
        s.set_playhead(c.start);
        // Seed a non-default attribute so removal is observable.
        if id == "edit.removeAttributes" {
            let q = std::sync::Arc::make_mut(&mut s.project).sequence_mut(s.state.active_sequence.unwrap()).unwrap();
            q.find_item_mut(c.id).unwrap().1.effect_mut("opacity").unwrap().params.get_mut("opacity").unwrap().value =
                filmcraft_project::ParamValue::Float(30.0);
        }
        if id == "sequence.applyDefaultTransitionsToSelection" {
            let q = std::sync::Arc::make_mut(&mut s.project).sequence_mut(s.state.active_sequence.unwrap()).unwrap();
            q.video_tracks[0].transitions.clear();
        }
        assert!(!s.is_enabled(id), "{id}");
        assert!(matches!(s.execute(id, json!({})), Err(EngineError::Disabled(..))), "{id}");
        let result =
            s.execute(id, json!({"clips": [c.id.0], "processing": "alwaysDeinterlace", "mode": "frameBlending"})).unwrap_or_else(|e| panic!("{id}: {e}"));
        let after = clip(&s, c.id).unwrap();
        match id {
            "edit.removeAttributes" => assert_eq!(result["clips"], 1),
            "edit.selectAllMatching" => assert!(s.state.selection.contains(&c.id)),
            "clip.frameHoldOptions" => assert!(after.frame_hold.is_some()),
            "clip.fieldOptions" => assert!(after.field_options.is_some()),
            "clip.timeInterpolation.frameSampling" => assert_eq!(after.time_interpolation, TimeInterpolation::FrameSampling),
            "clip.timeInterpolation.frameBlending" | "clip.setTimeInterpolation" => assert_eq!(after.time_interpolation, TimeInterpolation::FrameBlending),
            "clip.timeInterpolation.opticalFlow" => assert_eq!(after.time_interpolation, TimeInterpolation::OpticalFlow),
            "clip.fitToFrame" | "clip.fillFrame" => assert_eq!(result["clips"][0]["clip"], c.id.0),
            "clip.replaceFromSource" | "clip.replaceFromSourceMatchFrame" => assert_eq!(after.item, replacement),
            "sequence.applyDefaultTransitionsToSelection" => assert!(!result["transitions"].as_array().unwrap().is_empty()),
            _ => unreachable!(),
        }
        assert!(s.state.project_selection.is_empty(), "{id}");
        if id != "edit.selectAllMatching" {
            assert!(s.state.selection.is_empty(), "{id}");
            assert!(!s.is_enabled(id), "{id}");
        }
    }
}

#[test]
fn project_item_clip_handlers_read_named_items() {
    use crate::Services;
    use filmcraft_project::ItemKind;
    for id in ["file.newSequenceFromClip", "file.newBinFromSelection", "clip.audioChannels", "clip.modifyTimecode", "clip.breakoutToMono", "clip.extractAudio"]
    {
        let files = std::sync::Arc::new(TargetFiles::default());
        let mut s = Session::new(files.clone());
        s.execute("file.openDemoProject", json!({})).unwrap();
        let item = v1(&s)[0].item;
        s.state.selection.clear();
        s.state.project_selection.clear();
        // Extract a small, ordinary synthesized range; no files or external decoders are needed.
        if id == "clip.extractAudio" {
            std::sync::Arc::make_mut(&mut s.project).item_mut(item).unwrap().as_media_mut().unwrap().info.duration =
                filmcraft_time::Tick::from_seconds_f64(0.1);
        }
        assert!(!s.is_enabled(id), "{id}");
        assert!(matches!(s.execute(id, json!({})), Err(EngineError::Disabled(..))), "{id}");
        let mut params = json!({"items": [item.0], "name": "Named bin", "format": "mono", "clips": [[0]], "dir": "named-audio"});
        if id == "clip.modifyTimecode" {
            params = json!({"item": item.0, "frame": 240});
        }
        let result = s.execute(id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
        match id {
            "file.newSequenceFromClip" => {
                let q = s.project.sequence(ItemId(result["sequence"].as_u64().unwrap())).unwrap();
                assert!(q.all_tracks().flat_map(|t| &t.items).any(|c| c.item == item));
            }
            "file.newBinFromSelection" => assert_eq!(s.project.root.parent_of(item).unwrap().0, result["bin"].as_u64().unwrap()),
            "clip.audioChannels" => {
                assert_eq!(s.project.item(item).unwrap().as_media().unwrap().interpret.audio_channels.as_ref().unwrap().clips, vec![vec![0]])
            }
            "clip.modifyTimecode" => assert_eq!(s.project.item(item).unwrap().as_media().unwrap().info.start_timecode, Some(240)),
            "clip.breakoutToMono" => {
                let made = result["items"].as_array().unwrap();
                assert!(!made.is_empty());
                assert_eq!(s.state.project_selection, made.iter().map(|v| ItemId(v.as_u64().unwrap())).collect::<Vec<_>>());
            }
            "clip.extractAudio" => {
                let row = &result["extracted"][0];
                assert_eq!(row["from"], item.0);
                assert!(!files.read_file(row["path"].as_str().unwrap()).unwrap().is_empty());
                assert!(matches!(s.project.item(ItemId(row["item"].as_u64().unwrap())).unwrap().kind, ItemKind::Media(_)));
            }
            _ => unreachable!(),
        }
        if id != "clip.breakoutToMono" {
            assert!(s.state.project_selection.is_empty(), "{id}");
        }
        assert!(s.state.selection.is_empty(), "{id}");
    }
    // Both registered domains of Make Subclip reach the same media; creation intentionally selects it.
    for by_clip in [false, true] {
        let mut s = demo();
        let c = v1(&s)[0].clone();
        let mut params = json!({"name": "Named subclip", "startFrame": 1, "endFrame": 12});
        params[if by_clip { "clip" } else { "item" }] = json!(if by_clip { c.id.0 } else { c.item.0 });
        assert!(!s.is_enabled("clip.makeSubclip"));
        let sub = ItemId(s.execute("clip.makeSubclip", params).unwrap()["item"].as_u64().unwrap());
        assert_eq!(s.state.project_selection, vec![sub]);
        s.state.project_selection.clear();
        assert!(!s.is_enabled("clip.editSubclip"));
        s.execute("clip.editSubclip", json!({"item": sub.0, "endFrame": 8})).unwrap();
        match &s.project.item(sub).unwrap().kind {
            ItemKind::Subclip { parent, range, .. } => {
                assert_eq!(*parent, c.item);
                assert_eq!(range.end(), s.project.item(c.item).unwrap().frame_rate().tick_of(8));
            }
            other => panic!("expected a subclip, got {other:?}"),
        }
        assert!(s.state.selection.is_empty() && s.state.project_selection.is_empty());
        assert!(!s.is_enabled("project.delete"));
        s.execute("project.delete", json!({"items": [sub.0]})).unwrap();
        assert!(s.project.item(sub).is_none());
        assert!(s.state.selection.is_empty() && s.state.project_selection.is_empty());
    }
}

#[test]
fn multicamera_handlers_read_named_items_and_clips() {
    for id in ["clip.mergeClips", "clip.createMulticam"] {
        let mut s = demo();
        let v = v1(&s)[0].item;
        let a = s.active_sequence().unwrap().audio_tracks[1].items[0].item;
        assert!(!s.is_enabled(id));
        let r = s.execute(id, json!({"items": [v.0, a.0], "method": "in"})).unwrap();
        let created = ItemId(r[if id == "clip.mergeClips" { "item" } else { "sequence" }].as_u64().unwrap());
        let q = s.project.sequence(created).unwrap();
        assert!(q.all_tracks().flat_map(|t| &t.items).any(|c| c.item == v));
        assert!(q.all_tracks().flat_map(|t| &t.items).any(|c| c.item == a));
        assert_eq!(s.state.project_selection, vec![created], "creation intentionally selects the new item");
        assert!(s.state.selection.is_empty());
    }
    let mut s = demo();
    let v = v1(&s)[0].clone();
    let a = s.active_sequence().unwrap().audio_tracks[1].items[0].clone();
    assert!(!s.is_enabled("clip.synchronize"));
    let r = s.execute("clip.synchronize", json!({"clips": [v.id.0, a.id.0], "method": "in"})).unwrap();
    assert_eq!(r["clips"].as_array().unwrap().len(), 2);
    s.state.selection.clear();
    let source = ItemId(s.execute("clip.createMulticam", json!({"items": [v.item.0], "method": "in"})).unwrap()["sequence"].as_u64().unwrap());
    // Place the real nested source in an existing timeline slot to audit only the targeting contract.
    let q = std::sync::Arc::make_mut(&mut s.project).sequence_mut(s.state.active_sequence.unwrap()).unwrap();
    q.find_item_mut(v.id).unwrap().1.item = source;
    s.state.project_selection.clear();
    assert!(!s.is_enabled("clip.multicamEnable"));
    s.execute("clip.multicamEnable", json!({"clips": [v.id.0], "enabled": true})).unwrap();
    assert!(clip(&s, v.id).unwrap().multicam.unwrap().enabled);
    assert!(s.state.selection.is_empty() && s.state.project_selection.is_empty());
    assert!(!s.is_enabled("clip.multicamFlatten"));
    s.execute("clip.multicamFlatten", json!({"clips": [v.id.0]})).unwrap();
    assert!(s.active_sequence().unwrap().all_tracks().flat_map(|t| &t.items).any(|c| c.item == v.item && c.start == v.start));
    assert!(s.state.project_selection.is_empty());
}

#[test]
fn remix_and_scene_handlers_read_named_clips() {
    use filmcraft_render::remix::{Remix, TickPiece};
    use filmcraft_time::Tick;
    let mut s = demo();
    let a = a1(&s)[0].clone();
    // Seed an ordinary existing remix, avoiding expensive DSP analysis in a targeting test.
    let q = std::sync::Arc::make_mut(&mut s.project).sequence_mut(s.state.active_sequence.unwrap()).unwrap();
    let it = q.find_item_mut(a.id).unwrap().1;
    Remix {
        target: a.duration,
        segments: 50.0,
        variations: 50.0,
        original: a.duration,
        xfade: Tick::ZERO,
        pieces: vec![TickPiece { src: a.source_in, len: a.duration }],
    }
    .store(it);
    for id in ["clip.remix.enable", "clip.remix.properties", "clip.remix.revert"] {
        assert!(!s.is_enabled(id), "{id}");
        assert!(matches!(s.execute(id, json!({})), Err(EngineError::Disabled(..))), "{id}");
        let r = s.execute(id, json!({"clip": a.id.0})).unwrap();
        assert_eq!(r["clip"], a.id.0);
        assert_eq!(r["remixed"], id != "clip.remix.revert");
        assert!(s.state.selection.is_empty() && s.state.project_selection.is_empty());
    }
    let c = v1(&s)[0].clone();
    let fd = s.sequence_rate().frame_duration();
    let q = std::sync::Arc::make_mut(&mut s.project).sequence_mut(s.state.active_sequence.unwrap()).unwrap();
    q.find_item_mut(c.id).unwrap().1.duration = fd;
    assert!(!s.is_enabled("clip.sceneEditDetection"));
    let r = s.execute("clip.sceneEditDetection", json!({"clips": [c.id.0], "wait": true})).unwrap();
    assert_eq!(r["frames"], 1);
    assert_eq!(r["clips"][0]["clip"], c.id.0);
    assert!(s.state.selection.is_empty() && s.state.project_selection.is_empty());
}
