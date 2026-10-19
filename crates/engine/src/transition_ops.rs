//! Frame-aligned transition edge resizing, shared by timeline previews and commands.

use filmcraft_edit::Edge;
use filmcraft_project::{Sequence, Transition, TransitionAlign, TransitionId};
use filmcraft_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::bad;
use crate::{EngineError, Result, Session};

const CMD: &str = "timeline.resizeTransition";

/// One-sided fades keep their clip edge fixed, including legacy fades labelled as centered.
pub fn alignment(transition: &Transition) -> TransitionAlign {
    match (transition.from, transition.to) {
        (None, Some(_)) => TransitionAlign::StartAtCut,
        (Some(_), None) => TransitionAlign::EndAtCut,
        _ => transition.align,
    }
}

pub fn movable_edge(transition: &Transition, edge: Edge) -> bool {
    match alignment(transition) {
        TransitionAlign::CenterAtCut => true,
        TransitionAlign::StartAtCut => edge == Edge::Out,
        TransitionAlign::EndAtCut => edge == Edge::In,
    }
}

/// Resize without editing the project. The same limits govern preview and commit.
pub fn preview(seq: &Sequence, id: TransitionId, edge: Edge, target: Tick) -> Result<TimeRange> {
    if !(Tick::MIN..=Tick::MAX).contains(&target) {
        return Err(bad(CMD, "target is outside the supported time range"));
    }
    seq.settings.validate().map_err(|e| bad(CMD, e))?;
    let track = seq.all_tracks().find(|t| t.transitions.iter().any(|x| x.id == id)).ok_or_else(|| bad(CMD, "no such transition"))?;
    track.check_bounds().map_err(|e| bad(CMD, e))?;
    if track.locked {
        return Err(filmcraft_edit::EditError::Locked.into());
    }
    let tr = track.transitions.iter().find(|x| x.id == id).ok_or_else(|| bad(CMD, "no such transition"))?;
    if tr.duration <= Tick::ZERO {
        return Err(bad(CMD, "transition duration must be positive"));
    }
    if !movable_edge(tr, edge) {
        return Err(bad(CMD, "this transition edge is fixed at the cut"));
    }
    let from = tr.from.map(|id| track.item(id).ok_or_else(|| bad(CMD, "outgoing clip is missing"))).transpose()?;
    let to = tr.to.map(|id| track.item(id).ok_or_else(|| bad(CMD, "incoming clip is missing"))).transpose()?;
    if from.is_some_and(|a| to.is_some_and(|b| a.end() != b.start)) {
        return Err(bad(CMD, "transition clips are no longer adjacent"));
    }
    let cut = to.map(|i| i.start).or_else(|| from.map(|i| i.end())).ok_or_else(|| bad(CMD, "transition has no clips"))?;
    let rate = seq.settings.frame_rate;
    // Relative frame edits compose rounded tick durations, so a legitimate cut may sit a
    // tick away from its rational boundary. Use its nearest frame ordinal for resize math.
    let c = rate.frame_at(rate.snap_nearest(cut));
    // Round the left limit up: a partial frame at a clip edge cannot be covered.
    let ceil_frame = |t| {
        let f = rate.frame_at(t);
        if rate.tick_of(f) < t { f + 1 } else { f }
    };
    let mut lo = ceil_frame(from.map_or(cut, |i| i.start));
    let mut hi = rate.frame_at(to.map_or(cut, |i| i.end()));
    // A resize must not cover a different transition on the same track.
    for other in track.transitions.iter().filter(|x| x.id != id) {
        if other.end() <= cut {
            lo = lo.max(ceil_frame(other.end()));
        } else if other.start >= cut {
            hi = hi.min(rate.frame_at(other.start));
        } else {
            return Err(bad(CMD, "another transition covers this cut"));
        }
    }
    let target = rate.frame_at(rate.snap_nearest(target));
    let (start, end) = match alignment(tr) {
        TransitionAlign::StartAtCut => {
            let max = hi - c;
            if max < 1 {
                return Err(bad(CMD, "no room for a one-frame transition"));
            }
            (c, c + (target - c).clamp(1, max))
        }
        TransitionAlign::EndAtCut => {
            let max = c - lo;
            if max < 1 {
                return Err(bad(CMD, "no room for a one-frame transition"));
            }
            (c - (c - target).clamp(1, max), c)
        }
        TransitionAlign::CenterAtCut => {
            let max = (2 * (c - lo)).min(2 * (hi - c) + 1);
            if max < 1 {
                return Err(bad(CMD, "no room for a one-frame transition"));
            }
            // Preserve the odd-frame bias: the extra frame belongs to the outgoing side.
            let parity = (rate.frame_at(tr.end()) - rate.frame_at(tr.start)).rem_euclid(2);
            let duration = match edge {
                Edge::In => 2 * (c - target) - parity,
                Edge::Out => 2 * (target - c) + parity,
            }
            .clamp(1, max);
            let start = c - (duration + 1) / 2;
            (start, start + duration)
        }
    };
    let (mut start, mut end) = (rate.tick_of(start), rate.tick_of(end));
    // One-sided alignment anchors the exact clip cut, including composed fractional ticks.
    match alignment(tr) {
        TransitionAlign::StartAtCut => start = cut,
        TransitionAlign::EndAtCut => end = cut,
        TransitionAlign::CenterAtCut => {}
    }
    Ok(TimeRange::from_bounds(start, end))
}

pub(crate) fn resize(s: &mut Session, params: &Value) -> Result<Value> {
    let id = params.get("transition").and_then(Value::as_u64).map(TransitionId).ok_or_else(|| bad(CMD, "need `transition`"))?;
    let edge = match params.get("edge").and_then(Value::as_str) {
        Some("in") => Edge::In,
        Some("out") => Edge::Out,
        _ => return Err(bad(CMD, "edge must be `in` or `out`")),
    };
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    seq.settings.validate().map_err(|e| bad(CMD, e))?;
    let rate = seq.settings.frame_rate;
    let target = if let Some(time) = params.get("time").and_then(Value::as_i64) {
        Tick(time)
    } else if let Some(frame) = params.get("frame").and_then(Value::as_i64) {
        if frame < rate.frame_at(Tick::MIN) || frame > rate.frame_at(Tick::MAX) {
            return Err(bad(CMD, "target frame is outside the supported time range"));
        }
        rate.tick_of(frame)
    } else {
        return Err(bad(CMD, "need `time` or `frame`"));
    };
    let range = preview(seq, id, edge, target)?;
    let old = seq.all_tracks().flat_map(|t| &t.transitions).find(|x| x.id == id).ok_or_else(|| bad(CMD, "no such transition"))?;
    if old.range() != range {
        s.edit_sequence("Resize Transition", |seq, _, _| {
            let tr = seq.all_tracks_mut().flat_map(|t| &mut t.transitions).find(|x| x.id == id).ok_or_else(|| bad(CMD, "no such transition"))?;
            tr.start = range.start;
            tr.duration = range.duration;
            tr.align = alignment(tr);
            Ok(())
        })?;
    }
    Ok(json!({"transition": id.0, "start": range.start.0, "duration": range.duration.0}))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use filmcraft_media::{Generator, generators::GeneratorSource};
    use filmcraft_project::{Label, SequenceSettings, TrackKind, find_effect};
    use filmcraft_time::FrameRate;

    use super::*;

    /// Two tiny generated clips per track, with a centered 12-frame transition at frame 50.
    fn session(rate: FrameRate, kind: TrackKind) -> (Session, TransitionId) {
        let mut s = Session::default();
        let mut p = (*s.project).clone();
        let generator = if kind == TrackKind::Video { Generator::ColorMatte { color: [0.2, 0.5, 0.8, 1.0] } } else { Generator::Tone { hz: 440.0, db: -12.0 } };
        let item =
            crate::demo::add_generator(&mut p, &s.media, GeneratorSource::new(generator, 32, 18, rate, rate.tick_of(100)), "Resize fixture", Label::Iris, None);
        let seq = p.new_sequence("Transition resize", SequenceSettings { width: 32, height: 18, frame_rate: rate, ..Default::default() }, 1, 1, None);
        let mut a = p.make_track_item(item, kind, Tick::ZERO, TimeRange::new(Tick::ZERO, rate.tick_of(50)), rate).unwrap();
        let mut b = p.make_track_item(item, kind, rate.tick_of(50), TimeRange::new(Tick::ZERO, rate.tick_of(50)), rate).unwrap();
        // Resolve Motion's auto centers so exact preservation/undo assertions compare finite values.
        for clip in [&mut a, &mut b] {
            for effect in &mut clip.effects {
                filmcraft_project::resolve_auto_points(effect, (32, 18), (32, 18));
            }
        }
        // Absolute frame boundaries also cover rates whose frame duration is fractional in ticks.
        a.duration = rate.tick_of(50);
        b.duration = rate.tick_of(100) - b.start;
        let id = TransitionId(p.alloc_id());
        let effect = find_effect(if kind == TrackKind::Video { "cross_dissolve" } else { "constant_power" }).unwrap().instance();
        let tr = Transition {
            id,
            effect,
            start: rate.tick_of(44),
            duration: rate.tick_of(56) - rate.tick_of(44),
            from: Some(a.id),
            to: Some(b.id),
            align: TransitionAlign::CenterAtCut,
            reverse: true,
        };
        let track = &mut p.sequence_mut(seq).unwrap().tracks_mut(kind)[0];
        track.items = vec![a, b];
        track.transitions = vec![tr];
        s.project = Arc::new(p);
        s.state.active_sequence = Some(seq);
        s.state.open_sequences = vec![seq];
        (s, id)
    }

    fn transition(s: &Session, id: TransitionId) -> &Transition {
        s.active_sequence().unwrap().all_tracks().flat_map(|t| &t.transitions).find(|t| t.id == id).unwrap()
    }

    fn bounds(s: &Session, id: TransitionId) -> (i64, i64) {
        let tr = transition(s, id);
        let r = s.sequence_rate();
        (r.frame_at(tr.start), r.frame_at(tr.end()))
    }

    fn resize_frame(s: &mut Session, id: TransitionId, edge: &str, frame: i64) {
        s.execute(CMD, json!({"transition": id.0, "edge": edge, "frame": frame})).unwrap();
    }

    #[test]
    fn both_track_kinds_stretch_compress_and_undo_without_changing_clips_or_effect() {
        for rate in [FrameRate::FPS_24, FrameRate::new(31, 1)] {
            for kind in [TrackKind::Video, TrackKind::Audio] {
                let (mut s, id) = session(rate, kind);
                let before = (*s.project).clone();
                let old = transition(&s, id).clone();
                let clips = s.active_sequence().unwrap().tracks(kind)[0].items.clone();
                resize_frame(&mut s, id, "out", 60);
                assert_eq!(bounds(&s, id), (40, 60));
                assert_eq!(s.history.undo.len(), 1);
                let tr = transition(&s, id);
                assert_eq!((&tr.effect, tr.from, tr.to, tr.align, tr.reverse), (&old.effect, old.from, old.to, old.align, true));
                assert_eq!(s.active_sequence().unwrap().tracks(kind)[0].items, clips);
                assert!((tr.progress(rate.tick_of(50)) - 0.5).abs() < 1e-9, "the cut remains centered");
                assert!(tr.progress(rate.tick_of(55)) < old.progress(rate.tick_of(55)), "stretch slows the transition");
                resize_frame(&mut s, id, "in", 47);
                assert_eq!(bounds(&s, id), (47, 53));
                assert_eq!(s.history.undo.len(), 2);
                s.execute("edit.undo", json!({})).unwrap();
                assert_eq!(bounds(&s, id), (40, 60));
                s.execute("edit.undo", json!({})).unwrap();
                assert_eq!(*s.project, before);
                s.execute("edit.redo", json!({})).unwrap();
                assert_eq!(bounds(&s, id), (40, 60));
                // Releasing the edge where it already sits creates no history entry.
                let undo = s.history.undo.len();
                resize_frame(&mut s, id, "out", 60);
                assert_eq!(s.history.undo.len(), undo);
            }
        }
    }

    #[test]
    fn fractional_rate_roll_then_resize_accepts_composed_cut_and_preserves_alignment_and_undo() {
        let r = FrameRate::new(31, 1);
        for align in [TransitionAlign::CenterAtCut, TransitionAlign::StartAtCut, TransitionAlign::EndAtCut] {
            let (mut s, id) = session(r, TrackKind::Video);
            let before = s.project.clone();
            let items = &s.active_sequence().unwrap().video_tracks[0].items;
            let (left, right) = (items[0].id, items[1].id);
            s.execute("timeline.roll", json!({"left": left.0, "right": right.0, "deltaFrames": 10})).unwrap();
            let cut = s.active_sequence().unwrap().find_item(right).unwrap().1.start;
            assert_eq!(r.frame_at(cut), 60);
            assert_eq!(cut, r.tick_of(60) + Tick(1), "relative ceiling boundaries compose to one extra tick");
            if align != TransitionAlign::CenterAtCut {
                let q = Arc::make_mut(&mut s.project).sequence_mut(s.state.active_sequence.unwrap()).unwrap();
                let tr = &mut q.video_tracks[0].transitions[0];
                tr.align = align;
                if align == TransitionAlign::StartAtCut {
                    tr.start = cut;
                    tr.duration = r.tick_of(72) - cut;
                } else {
                    tr.start = r.tick_of(48);
                    tr.duration = cut - tr.start;
                }
            }
            let rolled = s.project.clone();
            let clips = s.active_sequence().unwrap().video_tracks[0].items.clone();
            resize_frame(&mut s, id, if align == TransitionAlign::EndAtCut { "in" } else { "out" }, if align == TransitionAlign::EndAtCut { 50 } else { 70 });
            assert_eq!(
                bounds(&s, id),
                match align {
                    TransitionAlign::CenterAtCut => (50, 70),
                    TransitionAlign::StartAtCut => (60, 70),
                    TransitionAlign::EndAtCut => (50, 60),
                }
            );
            let tr = transition(&s, id);
            assert_eq!(tr.align, align);
            assert!(tr.start >= clips[0].start && tr.end() <= clips[1].end());
            if align == TransitionAlign::StartAtCut {
                assert_eq!(tr.start, cut);
            }
            if align == TransitionAlign::EndAtCut {
                assert_eq!(tr.end(), cut);
            }
            assert_eq!(s.active_sequence().unwrap().video_tracks[0].items, clips);
            assert_eq!(s.history.undo.len(), 2);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.project, rolled);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.project, before);
        }
    }

    #[test]
    fn centered_odd_duration_clamps_to_a_frame_and_both_clip_bounds() {
        let (mut s, id) = session(FrameRate::FPS_24, TrackKind::Video);
        let r = s.sequence_rate();
        let q = Arc::make_mut(&mut s.project).sequence_mut(s.state.active_sequence.unwrap()).unwrap();
        let tr = &mut q.video_tracks[0].transitions[0];
        tr.start = r.tick_of(43);
        tr.duration = r.tick_of(13);
        resize_frame(&mut s, id, "out", 60);
        assert_eq!(bounds(&s, id), (39, 60), "odd durations retain the outgoing frame bias");
        resize_frame(&mut s, id, "out", 50);
        assert_eq!(bounds(&s, id), (49, 50), "a transition never reaches zero duration");
        resize_frame(&mut s, id, "in", -10_000);
        assert_eq!(bounds(&s, id), (0, 100), "both original clips bound the resize");
    }

    #[test]
    fn one_sided_fades_keep_the_cut_fixed_and_accept_only_the_outer_edge() {
        for fade_in in [true, false] {
            let (mut s, id) = session(FrameRate::FPS_24, TrackKind::Audio);
            let r = s.sequence_rate();
            let q = Arc::make_mut(&mut s.project).sequence_mut(s.state.active_sequence.unwrap()).unwrap();
            let tr = &mut q.audio_tracks[0].transitions[0];
            if fade_in {
                tr.from = None;
                tr.start = r.tick_of(50);
                tr.duration = r.tick_of(12);
            } else {
                tr.to = None;
                tr.start = r.tick_of(38);
                tr.duration = r.tick_of(12);
            }
            // Existing projects label these centered; infer their real one-sided geometry.
            tr.align = TransitionAlign::CenterAtCut;
            let fixed = if fade_in { "in" } else { "out" };
            assert!(s.execute(CMD, json!({"transition": id.0, "edge": fixed, "frame": 55})).is_err());
            resize_frame(&mut s, id, if fade_in { "out" } else { "in" }, if fade_in { 70 } else { 30 });
            assert_eq!(bounds(&s, id), if fade_in { (50, 70) } else { (30, 50) });
            assert_eq!(transition(&s, id).align, if fade_in { TransitionAlign::StartAtCut } else { TransitionAlign::EndAtCut });
            resize_frame(&mut s, id, if fade_in { "out" } else { "in" }, if fade_in { 10_000 } else { -10_000 });
            assert_eq!(bounds(&s, id), if fade_in { (50, 100) } else { (0, 50) });
        }
    }

    #[test]
    fn two_sided_start_and_end_alignment_keep_the_anchored_edge_at_the_cut() {
        for align in [TransitionAlign::StartAtCut, TransitionAlign::EndAtCut] {
            let (mut s, id) = session(FrameRate::FPS_24, TrackKind::Video);
            let r = s.sequence_rate();
            let q = Arc::make_mut(&mut s.project).sequence_mut(s.state.active_sequence.unwrap()).unwrap();
            let tr = &mut q.video_tracks[0].transitions[0];
            tr.align = align;
            tr.start = r.tick_of(if align == TransitionAlign::StartAtCut { 50 } else { 38 });
            tr.duration = r.tick_of(12);
            resize_frame(
                &mut s,
                id,
                if align == TransitionAlign::StartAtCut { "out" } else { "in" },
                if align == TransitionAlign::StartAtCut { 70 } else { 30 },
            );
            assert_eq!(bounds(&s, id), if align == TransitionAlign::StartAtCut { (50, 70) } else { (30, 50) });
            assert_eq!(transition(&s, id).align, align);
        }
    }

    #[test]
    fn neighboring_transition_limits_the_drag_without_removing_it() {
        let (mut s, id) = session(FrameRate::FPS_24, TrackKind::Video);
        let r = s.sequence_rate();
        let q = Arc::make_mut(&mut s.project).sequence_mut(s.state.active_sequence.unwrap()).unwrap();
        let mut neighbor = q.video_tracks[0].transitions[0].clone();
        neighbor.id = TransitionId(id.0 + 1);
        neighbor.start = r.tick_of(60);
        neighbor.duration = r.tick_of(10);
        q.video_tracks[0].transitions.push(neighbor.clone());
        resize_frame(&mut s, id, "out", 80);
        assert_eq!(bounds(&s, id), (39, 60));
        assert_eq!(transition(&s, neighbor.id), &neighbor);
    }

    #[test]
    fn hostile_requests_locked_tracks_and_damaged_geometry_leave_project_and_history_unchanged() {
        let (mut s, id) = session(FrameRate::FPS_24, TrackKind::Video);
        for params in [
            json!({}),
            json!({"transition": -1, "edge": "in", "frame": 40}),
            json!({"transition": id.0, "edge": "side", "frame": 40}),
            json!({"transition": id.0, "edge": "in", "time": i64::MIN}),
            json!({"transition": id.0, "edge": "out", "time": i64::MAX}),
            json!({"transition": id.0, "edge": "out", "frame": i64::MAX}),
            json!({"transition": u64::MAX, "edge": "out", "frame": 60}),
            json!({"transition": id.0, "edge": "out", "time": 1e200}),
        ] {
            let before = s.project.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.execute(CMD, params.clone())));
            assert!(result.is_ok(), "panicked for {params}");
            assert!(result.unwrap().is_err(), "accepted {params}");
            assert_eq!(s.project, before);
            assert!(s.history.undo.is_empty());
        }
        for damage in ["locked", "missing", "gap", "zero", "overflow", "rate"] {
            let (mut s, id) = session(FrameRate::FPS_24, TrackKind::Video);
            let q = Arc::make_mut(&mut s.project).sequence_mut(s.state.active_sequence.unwrap()).unwrap();
            match damage {
                "locked" => q.video_tracks[0].locked = true,
                "missing" => q.video_tracks[0].transitions[0].to = Some(filmcraft_project::ClipId(u64::MAX)),
                "gap" => q.video_tracks[0].items[1].start += Tick(1),
                "zero" => q.video_tracks[0].transitions[0].duration = Tick::ZERO,
                "overflow" => q.video_tracks[0].transitions[0].duration = Tick(i64::MAX),
                "rate" => q.settings.frame_rate.num = 0,
                _ => unreachable!(),
            }
            let before = s.project.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.execute(CMD, json!({"transition": id.0, "edge": "out", "frame": 60}))));
            assert!(result.is_ok(), "panicked on {damage}");
            assert!(result.unwrap().is_err(), "accepted {damage}");
            assert_eq!(s.project, before);
            assert!(s.history.undo.is_empty());
        }
    }
}
