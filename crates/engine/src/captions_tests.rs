use serde_json::json;

use crate::Session;
use filmcraft_project::ClipId;
use filmcraft_time::{TICKS_PER_SECOND, Tick};

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

#[test]
fn captions_import_edit_export_and_burn_in() {
    let dir = std::env::temp_dir().join(format!("fc-cap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let srt = dir.join("talk.srt");
    std::fs::write(
        &srt,
        "\u{feff}1\r\n00:00:00,500 --> 00:00:02,000\r\nHello there\r\n\r\n2\r\n00:00:02,500 --> 00:00:04,000\r\nSecond line\r\nwith two rows\r\n",
    )
    .unwrap();
    let mut s = demo();
    let r = s.execute("file.import", json!({"paths": [srt.to_string_lossy()]})).unwrap();
    assert_eq!(r["documents"][0]["captions"], 2, "{r}");
    let q = s.active_sequence().unwrap();
    let rate = q.settings.frame_rate;
    let tr = q.caption_tracks[0].clone();
    assert_eq!(tr.name, "talk");
    assert_eq!(tr.captions[0].start, rate.snap_nearest(Tick(TICKS_PER_SECOND / 2)), "snapped to frames");
    let first = tr.captions[0].id;
    let second = tr.captions[1].id;

    // edit text, split, merge, trim, move, add at playhead, delete — each one undoable
    s.execute("captions.setText", json!({"caption": first.0, "text": "Hi!", "speaker": "Ann"})).unwrap();
    let split_at = rate.tick_of(rate.frame_at(tr.captions[1].start) + 10);
    let r = s.execute("captions.split", json!({"caption": second.0, "time": split_at.0})).unwrap();
    let right = ClipId(r["captions"][0].as_u64().unwrap());
    assert_eq!(s.active_sequence().unwrap().caption_tracks[0].captions.len(), 3);
    s.execute("captions.merge", json!({"captions": [second.0, right.0]})).unwrap();
    assert_eq!(s.active_sequence().unwrap().caption_tracks[0].captions.len(), 2);
    s.execute("captions.trim", json!({"caption": second.0, "edge": "out", "deltaFrames": 12})).unwrap();
    s.execute("captions.move", json!({"captions": [second.0], "deltaFrames": 24})).unwrap();
    s.execute("playhead.set", json!({"seconds": 8.0})).unwrap();
    let r = s.execute("captions.add", json!({"text": "Added"})).unwrap();
    let added = ClipId(r["caption"].as_u64().unwrap());
    assert_eq!(s.state.caption_selection, vec![added]);
    s.execute("edit.clear", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().find_caption(added).is_none(), "Clear deletes selected captions");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().find_caption(added).is_some());
    let list = s.execute("captions.list", json!({})).unwrap();
    assert_eq!(list["tracks"][0]["captions"][0]["text"], "Hi!");
    assert_eq!(list["tracks"][0]["captions"][0]["speaker"], "Ann");
    s.active_sequence().unwrap().check().unwrap();

    // export in all three formats and read back: same texts
    for ext in ["srt", "vtt", "scc"] {
        let path = dir.join(format!("out.{ext}")).to_string_lossy().to_string();
        let r = s.execute("captions.export", json!({"path": path})).unwrap();
        assert_eq!(r["captions"], 3, "{ext}: {r}");
        let back = filmcraft_captions::parse(&std::fs::read(&path).unwrap(), filmcraft_captions::Format::from_name(ext).unwrap()).unwrap();
        let texts: Vec<String> = back.cues.iter().map(|c| filmcraft_project::plain_text(&c.text)).collect();
        assert_eq!(texts.len(), 3, "{ext}");
        assert_eq!(texts[0], "Hi!", "{ext}");
    }

    // the program renders captions; hiding the track removes them
    s.execute("playhead.set", json!({"seconds": 1.0})).unwrap();
    let with = s.render_program(0.25).unwrap();
    s.execute("captions.hideAll", json!({})).unwrap();
    let without = s.render_program(0.25).unwrap();
    let diff = with.px.iter().zip(&without.px).filter(|(a, b)| (*a - *b).abs() > 0.05).count();
    assert!(diff > 200, "captions change the picture ({diff} samples)");
    let half = with.w * 4 * (with.h / 2);
    let top_diff = with.px[..half].iter().zip(&without.px[..half]).filter(|(a, b)| (*a - *b).abs() > 0.05).count();
    assert_eq!(top_diff, 0, "bottom-anchored captions stay in the lower half");
    s.execute("captions.showAll", json!({})).unwrap();
    let plan = filmcraft_render::plan::plan_frame(
        &s.project,
        s.state.active_sequence.unwrap(),
        s.playhead(),
        filmcraft_render::RenderOptions { scale: 0.25, captions: true, ..Default::default() },
        &s.media.provider(s.project.clone(), s.services.clone()),
    );
    let cpu = filmcraft_render::plan::execute_cpu(&plan);
    let pd = cpu.px.iter().zip(&with.px).filter(|(a, b)| (*a - *b).abs() > 0.02).count();
    assert!(pd < cpu.px.len() / 200, "the GPU plan matches the CPU render ({pd})");

    // save/open keeps caption tracks
    let proj = dir.join("cap.fcproj").to_string_lossy().to_string();
    s.execute("file.saveAs", json!({"path": proj})).unwrap();
    let before = s.active_sequence().unwrap().caption_tracks.clone();
    s.execute("file.open", json!({"path": proj})).unwrap();
    assert_eq!(s.active_sequence().unwrap().caption_tracks, before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn caption_track_follows_sync_locked_insert() {
    let mut s = demo();
    s.execute("captions.newTrack", json!({"format": "CEA-608"})).unwrap();
    s.execute("playhead.set", json!({"seconds": 4.0})).unwrap();
    s.execute("captions.add", json!({"text": "later"})).unwrap();
    let start = s.active_sequence().unwrap().caption_tracks[0].captions[0].start;
    let item = s.project.items.values().find(|i| i.as_media().is_some_and(|m| m.info.has_video())).unwrap().id;
    s.execute("source.open", json!({"item": item.0})).unwrap();
    s.execute("playhead.set", json!({"seconds": 0.0})).unwrap();
    s.execute("source.insert", json!({})).unwrap();
    let after = s.active_sequence().unwrap().caption_tracks[0].captions[0].start;
    assert!(after > start, "caption moved right with the insert");
    s.active_sequence().unwrap().check().unwrap();
}

#[test]
fn caption_commands_need_tracks_and_navigate() {
    let mut s = demo();
    assert!(s.execute("captions.split", json!({})).is_err(), "disabled without captions");
    for (sec, text) in [(1.0, "one"), (3.0, "two"), (6.0, "three")] {
        s.execute("playhead.set", json!({"seconds": sec})).unwrap();
        s.execute("captions.add", json!({"text": text, "durationSeconds": 1.0})).unwrap();
    }
    s.execute("playhead.set", json!({"seconds": 0.0})).unwrap();
    s.execute("captions.next", json!({})).unwrap();
    s.execute("captions.next", json!({})).unwrap();
    let id = s.state.caption_selection[0];
    assert_eq!(s.active_sequence().unwrap().find_caption(id).unwrap().1.text, "two");
    s.execute("captions.previous", json!({})).unwrap();
    let id = s.state.caption_selection[0];
    assert_eq!(s.active_sequence().unwrap().find_caption(id).unwrap().1.text, "one");
    s.execute("captions.setStyle", json!({"color": "#ffff00", "anchor": "top", "size": 40})).unwrap();
    let st = &s.active_sequence().unwrap().caption_tracks[0].style;
    assert_eq!(st.color, [255, 255, 0, 255]);
    assert!(s.execute("captions.setStyle", json!({"anchor": "sideways"})).is_err());
    s.execute("captions.setTimes", json!({"caption": id.0, "startSeconds": 0.5, "endSeconds": 2.5})).unwrap();
    assert!(s.execute("captions.setTimes", json!({"caption": id.0, "endSeconds": 3.5})).is_err(), "would overlap `two`");
    s.execute("captions.deleteTrack", json!({"track": "C1"})).unwrap();
    assert!(s.active_sequence().unwrap().caption_tracks.is_empty());
}

#[test]
fn caption_list_track_filter_never_falls_back_to_every_track() {
    let mut s = demo();
    s.execute("captions.newTrack", json!({})).unwrap();
    s.execute("captions.newTrack", json!({})).unwrap();
    let all = s.execute("captions.list", json!({})).unwrap();
    let ids: Vec<u64> = all["tracks"].as_array().unwrap().iter().map(|t| t["id"].as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 2, "{all}");
    for (i, id) in ids.iter().enumerate() {
        for sel in [json!(id), json!(format!("C{}", i + 1))] {
            let r = s.execute("captions.list", json!({"track": sel})).unwrap();
            let got: Vec<u64> = r["tracks"].as_array().unwrap().iter().map(|t| t["id"].as_u64().unwrap()).collect();
            assert_eq!(got, vec![*id], "{sel}");
        }
    }
    let missing = ids.iter().max().unwrap() + 999;
    for sel in [json!(missing), json!("C3"), json!("C0"), json!("nope")] {
        assert!(s.execute("captions.list", json!({"track": sel})).is_err(), "{sel} names no caption track");
    }
}

fn txt_export_fixture() -> (Session, u64, u64) {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "Caption review", "fps": 25, "video": 0, "audio": 0})).unwrap();
    let track = s.execute("captions.newTrack", json!({"name": "Review"})).unwrap()["track"].as_u64().unwrap();
    // Create the later cue first: normal editing establishes chronological storage order.
    let later =
        s.execute("captions.add", json!({"track": track, "seconds": 1, "durationSeconds": 1, "text": "مرحبا 世界"})).unwrap()["caption"].as_u64().unwrap();
    let earlier = s.execute("captions.add", json!({"track": track, "seconds": 0, "durationSeconds": 1, "text": "<i>Café</i> &amp; tea\nSecond line"})).unwrap()
        ["caption"]
        .as_u64()
        .unwrap();
    s.execute("captions.setText", json!({"caption": earlier, "text": "<i>Café</i> &amp; tea\nSecond line", "speaker": "Ann"})).unwrap();
    let other = s.execute("captions.newTrack", json!({"name": "Other"})).unwrap()["track"].as_u64().unwrap();
    s.execute("captions.add", json!({"track": other, "seconds": 0, "durationSeconds": 1, "text": "OTHER TRACK"})).unwrap();
    let empty = s.execute("captions.newTrack", json!({"name": "Empty"})).unwrap()["track"].as_u64().unwrap();
    let captions = &s.active_sequence().unwrap().caption_tracks[2].captions;
    assert_eq!(captions.iter().map(|c| c.id.0).collect::<Vec<_>>(), [earlier, later]);
    s.active_sequence().unwrap().check().unwrap();
    let project = s.project.clone();
    s.execute("captions.setText", json!({"caption": later, "text": "Temporary edit"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.as_ref(), project.as_ref());
    assert!(!s.history.undo.is_empty() && !s.history.redo.is_empty());
    (s, track, empty)
}

fn txt_export_snapshot(s: &Session) -> serde_json::Value {
    let undo: Vec<_> = s.history.undo.iter().map(|(label, project)| json!([label, project.as_ref()])).collect();
    let redo: Vec<_> = s.history.redo.iter().map(|(label, project)| json!([label, project.as_ref()])).collect();
    json!({
        "project": s.project.as_ref(), "editor": s.state,
        "undo": undo, "redo": redo, "limit": s.history.limit, "mergeKey": s.history.merge_key,
        "revision": s.revision, "savedRevision": s.saved_revision, "path": s.path,
    })
}

#[test]
fn captions_txt_exports_rendered_text_in_existing_track_order() {
    let (mut s, track, empty) = txt_export_fixture();
    let dir = std::env::temp_dir().join(format!("fc-caption-txt-content-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let before = txt_export_snapshot(&s);
    let expected = "Café & tea\nSecond line\n\nمرحبا 世界\n".as_bytes();
    for (name, format) in [("review.txt", None), ("review.TXT", None), ("review.data", Some(".TXT"))] {
        let path = dir.join(name);
        let mut params = json!({"path": path, "track": track});
        if let Some(format) = format {
            params["format"] = json!(format);
        }
        let result = s.execute("captions.export", params).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), expected, "{name}");
        assert_eq!(result, json!({"path": path, "format": "Plain text (TXT)", "captions": 2, "bytes": expected.len()}));
        assert_eq!(txt_export_snapshot(&s), before);
    }
    let path = dir.join("empty.txt");
    let result = s.execute("captions.export", json!({"path": path, "track": empty})).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"");
    assert_eq!(result["captions"], 0);
    assert_eq!(result["bytes"], 0);
    assert_eq!(txt_export_snapshot(&s), before);
}

#[test]
fn captions_txt_respects_explicit_formats_without_import_or_sidecar_support() {
    let (mut s, track, _) = txt_export_fixture();
    let dir = std::env::temp_dir().join(format!("fc-caption-txt-scope-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let before = txt_export_snapshot(&s);
    let srt = dir.join("explicit-srt.txt");
    let result = s.execute("captions.export", json!({"path": srt, "track": track, "format": "srt"})).unwrap();
    assert_eq!(result["format"], "SubRip (SRT)");
    assert_eq!(
        std::fs::read_to_string(&srt).unwrap(),
        "1\n00:00:00,000 --> 00:00:01,000\n<i>Café</i> &amp; tea\nSecond line\n\n2\n00:00:01,000 --> 00:00:02,000\nمرحبا 世界\n\n"
    );
    let text = dir.join("explicit-txt.srt");
    let result = s.execute("captions.export", json!({"path": text, "track": "C3", "format": "txt"})).unwrap();
    assert_eq!(result["format"], "Plain text (TXT)");
    let bytes = std::fs::read(&text).unwrap();
    assert_eq!(bytes, "Café & tea\nSecond line\n\nمرحبا 世界\n".as_bytes());
    assert!(filmcraft_captions::Format::from_name("txt").is_none());
    assert!(filmcraft_captions::detect(&bytes, Some("txt")).is_none());
    assert!(s.execute("captions.import", json!({"path": text})).unwrap_err().to_string().contains("not a caption file"));
    let media = dir.join("unsupported.wav");
    let error = s.execute("file.exportMedia", json!({"path": media, "format": "wav", "captionSidecar": "txt", "wait": true})).unwrap_err();
    assert!(error.to_string().contains("caption sidecar `txt`: srt | vtt"), "{error}");
    assert!(!media.exists());
    assert_eq!(txt_export_snapshot(&s), before);
}
