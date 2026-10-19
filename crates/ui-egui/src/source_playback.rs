//! Source transport uses the shared playback clock without starting timeline recording passes.

use filmcraft_engine::clip_ops::source_view;
use filmcraft_project::ItemId;
use filmcraft_time::{FrameRate, Tick};
use serde_json::{Value, json};

use crate::dock::PanelKind;
use crate::{FilmcraftApp, PlaybackTarget};

pub(crate) struct Span {
    pub rate: FrameRate,
    pub start: Tick,
    pub end: Tick,
    pub mark_in: Option<Tick>,
    pub mark_out: Option<Tick>,
}

impl FilmcraftApp {
    /// Play the currently open Source item; the Program playhead remains untouched.
    pub fn play_source(&mut self, speed: f64) -> Result<(), String> {
        self.play_source_from(speed, true)
    }

    pub(crate) fn play_source_from(&mut self, speed: f64, restart_from_end: bool) -> Result<(), String> {
        if !speed.is_finite() || speed == 0.0 || speed.abs() > 8.0 {
            return Err("Source playback speed must be nonzero and between -8 and 8".into());
        }
        if !self.ui.dock.contains(PanelKind::Source) {
            return Err("the Source monitor panel is closed".into());
        }
        let item = self.session.state.source_item.ok_or("no clip in the Source monitor")?;
        let view = source_view(&self.session, item).ok_or("the Source monitor item is unavailable")?;
        if view.end <= view.start {
            return Err("the Source monitor item has no duration".into());
        }
        if self.session.trim_play.active() {
            self.session.execute("trim.shuttleStop", json!({"clock": self.last_ui_time})).map_err(|e| e.to_string())?;
        }
        let target = PlaybackTarget::Source(item);
        if self.playback.playing && self.playback.target != Some(target) {
            self.stop();
        }
        let last = (view.end - view.rate.frame_duration()).max(view.start);
        let cur = self.session.state.source_playhead;
        let time = if restart_from_end && speed > 0.0 && self.session.prefs.timeline.return_to_beginning && cur >= last {
            view.start
        } else {
            cur.clamp(view.start, last)
        };
        self.session.execute("source.setPlayhead", json!({"time": time.0})).map_err(|e| e.to_string())?;
        if !self.playback.playing || self.playback.speed != speed {
            self.playback.meter.start(speed);
        }
        self.playback.target = Some(target);
        self.playback.playing = true;
        self.playback.speed = speed;
        self.playback.anchor_tick = self.session.state.source_playhead;
        self.playback.anchor_time = -1.0;
        self.playback.preroll = Some(-1.0);
        self.playback.preroll_ready = false;
        self.playback.stop_at = None;
        self.loudness = None;
        if let Some(audio) = self.audio.as_mut() {
            audio.stop();
        }
        self.playback.audio_clock = false;
        Ok(())
    }

    pub fn source_is_playing(&self) -> bool {
        self.playback.playing && matches!(self.playback.target, Some(PlaybackTarget::Source(item)) if self.session.state.source_item == Some(item))
    }

    pub fn program_is_playing(&self) -> bool {
        self.playback.playing && !matches!(self.playback.target, Some(PlaybackTarget::Source(_)))
    }

    pub(crate) fn playback_position(&self) -> Tick {
        if matches!(self.playback.target, Some(PlaybackTarget::Source(_))) { self.session.state.source_playhead } else { self.session.playhead() }
    }

    pub(crate) fn set_playback_position(&mut self, time: Tick) {
        if matches!(self.playback.target, Some(PlaybackTarget::Source(_))) {
            let Some(span) = self.playback_span().filter(|span| span.end > span.start) else {
                self.stop();
                return;
            };
            // Clock updates, like Program's set_playhead, must not fill the command journal.
            let last = (span.end - span.rate.frame_duration()).max(span.start);
            self.session.state.source_playhead = span.rate.snap(time.clamp(span.start, last)).max(span.start);
        } else {
            self.session.set_playhead(time);
        }
    }

    pub(crate) fn playback_span(&self) -> Option<Span> {
        if let Some(PlaybackTarget::Source(item)) = self.playback.target {
            if self.session.state.source_item != Some(item) || !self.ui.dock.contains(PanelKind::Source) {
                return None;
            }
            let v = source_view(&self.session, item)?;
            return Some(Span { rate: v.rate, start: v.start, end: v.end, mark_in: v.mark_in, mark_out: v.mark_out });
        }
        let seq = self.session.active_sequence()?;
        if matches!(self.playback.target, Some(PlaybackTarget::Program(item)) if self.session.state.active_sequence != Some(item)) {
            return None;
        }
        Some(Span { rate: seq.settings.frame_rate, start: Tick::ZERO, end: seq.duration(), mark_in: seq.mark_in, mark_out: seq.mark_out })
    }
}

/// The toolbar and keyboard dispatch the same canonical transport ids, with an explicit monitor.
pub(crate) fn route_transport(app: &mut FilmcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let source = match params.get("target").and_then(Value::as_str) {
        Some("source") => true,
        Some(_) => false,
        None => app.ui.focused == PanelKind::Source,
    };
    if !source
        || !matches!(
            id,
            "playback.toggle"
                | "playback.stop"
                | "playback.forward"
                | "playback.reverse"
                | "playback.slowForward"
                | "playback.slowReverse"
                | "playback.inToOut"
                | "playback.inToOutPreroll"
                | "playback.toOut"
                | "playback.playAround"
        )
    {
        return None;
    }
    Some((|| {
        match id {
            "playback.stop" => {
                if app.session.trim_play.active() {
                    app.session.execute("trim.shuttleStop", json!({"clock": app.last_ui_time})).map_err(|e| e.to_string())?;
                }
                app.stop();
            }
            "playback.toggle" if app.source_is_playing() => app.stop(),
            "playback.inToOut" | "playback.inToOutPreroll" | "playback.toOut" | "playback.playAround" => {
                let item = app.session.state.source_item.ok_or("no clip in the Source monitor")?;
                let view = source_view(&app.session, item).ok_or("the Source monitor item is unavailable")?;
                let around = id == "playback.playAround";
                let current = app.session.state.source_playhead;
                let mut start = if id == "playback.toOut" || around { current } else { view.mark_in.unwrap_or(view.start) };
                let mut end = if around { current } else { view.mark_out.map(|out| out + view.rate.frame_duration()).unwrap_or(view.end) };
                if id == "playback.inToOutPreroll" || around {
                    start -= Tick::from_seconds_f64(app.session.prefs.playback.preroll_seconds);
                    end += Tick::from_seconds_f64(app.session.prefs.playback.postroll_seconds);
                }
                app.session.execute("source.setPlayhead", json!({"time": start.max(view.start).0})).map_err(|e| e.to_string())?;
                app.play_source_from(1.0, false)?;
                app.playback.stop_at = Some(end.min(view.end).max(view.start));
            }
            _ => {
                let speed = match id {
                    "playback.forward" if app.source_is_playing() && app.playback.speed > 0.0 => (app.playback.speed * 2.0).min(8.0),
                    "playback.reverse" if app.source_is_playing() && app.playback.speed < 0.0 => (app.playback.speed * 2.0).max(-8.0),
                    "playback.reverse" => -1.0,
                    "playback.slowForward" => 0.25,
                    "playback.slowReverse" => -0.25,
                    _ => 1.0,
                };
                app.play_source(speed)?;
            }
        }
        Ok(json!({"playing": app.source_is_playing(), "speed": app.playback.speed, "target": app.playback.target}))
    })())
}

/// Media audio stays in source time (including a restricted subclip's absolute In point).
/// Sequence sources use the same sequence mixer as Program, with recording cues disabled.
pub(crate) fn media_mix(session: &filmcraft_engine::Session, item: ItemId, sr: u32) -> impl FnMut(i64, &mut [f32], usize) + Send + 'static {
    let source = session.media.source_for(&session.project, item, &*session.services);
    let view = source_view(session, item);
    let (lo, hi) = view.map(|v| (v.start.to_units_floor(i64::from(sr)), v.end.to_units_floor(i64::from(sr)))).unwrap_or((0, 0));
    let map = [session.prefs.audio_hardware.map_left, session.prefs.audio_hardware.map_right];
    let mixdown = filmcraft_audio_dsp::channels::Mixdown::from_id(&session.prefs.audio.mixdown_type).unwrap_or_default();
    move |cursor: i64, out: &mut [f32], ch: usize| {
        out.fill(0.0);
        if ch == 0 || sr == 0 {
            return;
        }
        let Some(source) = source.as_ref() else { return };
        let frames = out.len() / ch;
        let end = cursor.saturating_add(i64::try_from(frames).unwrap_or(i64::MAX)).min(hi);
        let start = cursor.max(lo);
        if end <= start {
            return;
        }
        let n = usize::try_from(end.saturating_sub(start)).unwrap_or(0).min(frames);
        let Ok(audio) = source.audio(start, n, sr) else { return };
        let channels = filmcraft_audio_dsp::channels::convert_owned(audio.channels, filmcraft_audio_dsp::channels::Layout::Stereo, mixdown);
        let Some(left) = channels.first() else { return };
        let Some(right) = channels.get(1) else { return };
        let offset = usize::try_from(start.saturating_sub(cursor)).unwrap_or(frames).min(frames);
        let begin = offset.saturating_mul(ch).min(out.len());
        filmcraft_engine::settings::map_output(left, right, &mut out[begin..], ch, map);
    }
}
