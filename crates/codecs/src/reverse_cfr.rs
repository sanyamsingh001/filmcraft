//! Exact logical frame ordinals for reverse Frame Sampling of indexed progressive AVC.
//!
//! Container timestamps may be rounded or truncated to a coarse unit. A consistent declarative
//! rate plus the complete sample table can validate that cadence; a median duration cannot.
//! Unsupported layouts retain ordinary media-time lookup. Validation adds no frame/index buffer.

use filmcraft_isobmff::{AvcConfig, CodecConfig};
use filmcraft_time::{FrameRate, Tick};

/// At most about nine hours at 30 fps; longer tracks retain ordinary lookup.
const MAX_SAMPLES: usize = 1 << 20;
const MAX_AVCC_BYTES: usize = 1 << 20;
const MAX_SPS_BYTES: usize = u16::MAX as usize;

/// A validated logical ordinal. Callers map it to the existing sample's actual PTS.
pub(crate) fn sample_index(rate: FrameRate, time: Tick, count: usize) -> Option<usize> {
    let last = i64::try_from(count).ok()?.checked_sub(1)?;
    if last < 0 || rate.num <= 0 || rate.den <= 0 {
        return None;
    }
    usize::try_from(rate.frame_at(time.max(Tick::ZERO)).clamp(0, last)).ok()
}

// Outer None means inconsistent/unsupported SPS metadata. Inner None means progressive SPS
// metadata has no timing hint: a uniform MP4 STTS table may still prove an exact cadence.
fn avc_hint(config: &AvcConfig) -> Option<Option<FrameRate>> {
    if config.sps.is_empty() || config.sps.len() > 32 {
        return None;
    }
    let mut common = None;
    for nal in &config.sps {
        if nal.len() > MAX_SPS_BYTES || nal.first().is_none_or(|header| *header & 0x80 != 0 || *header & 0x1f != 7) {
            return None;
        }
        let rbsp = filmcraft_bitstream::unescape_rbsp(nal.get(1..)?);
        let sps = filmcraft_h264::params::Sps::parse(&rbsp).ok()?;
        if !sps.frame_mbs_only {
            return None;
        }
        let rate = match sps.vui.as_ref().and_then(|vui| vui.timing) {
            Some((units, scale, _)) if units > 0 && scale > 0 => Some(FrameRate::new(i64::from(scale), i64::from(units) * 2)),
            Some(_) => return None,
            None => None,
        };
        if common.is_some_and(|previous| previous != rate) {
            return None;
        }
        common = Some(rate);
    }
    common
}

fn stamp(index: usize, rate: FrameRate, unit: (u64, u64), nearest: bool) -> Option<i64> {
    if rate.num <= 0 || rate.den <= 0 || unit.0 == 0 || unit.1 == 0 {
        return None;
    }
    let numerator = i128::try_from(index).ok()?.checked_mul(i128::from(rate.den))?.checked_mul(i128::from(unit.1))?;
    let denominator = i128::from(rate.num).checked_mul(i128::from(unit.0))?;
    let value = if nearest {
        numerator.checked_mul(2)?.checked_add(denominator)?.checked_div(denominator.checked_mul(2)?)?
    } else {
        numerator.checked_div(denominator)?
    };
    i64::try_from(value).ok()
}

fn policy_matches(
    count: usize,
    rate: FrameRate,
    unit: (u64, u64),
    terminal: Option<i64>,
    sample: &impl Fn(usize) -> Option<(i64, Option<i64>)>,
    nearest: bool,
) -> Option<()> {
    let mut previous = None;
    for index in 0..count {
        let (pts, end) = sample(index)?;
        if previous.is_some_and(|previous| previous >= pts) || stamp(index, rate, unit, nearest)? != pts {
            return None;
        }
        if let Some(end) = end
            && stamp(index.checked_add(1)?, rate, unit, nearest)? != end
        {
            return None;
        }
        previous = Some(pts);
    }
    if let Some(end) = terminal
        && stamp(count, rate, unit, nearest)? != end
    {
        return None;
    }
    Some(())
}

fn validates(count: usize, rate: FrameRate, unit: (u64, u64), terminal: Option<i64>, sample: impl Fn(usize) -> Option<(i64, Option<i64>)>) -> bool {
    if !(2..=MAX_SAMPLES).contains(&count) || rate.num <= 0 || rate.den <= 0 || unit.0 == 0 || unit.1 == 0 {
        return false;
    }
    [false, true].into_iter().any(|nearest| policy_matches(count, rate, unit, terminal, &sample, nearest).is_some())
}

fn choose(candidates: [Option<FrameRate>; 2], accepts: impl Fn(FrameRate) -> bool) -> Option<FrameRate> {
    let mut selected = None;
    let mut previous_candidate = None;
    for rate in candidates.into_iter().flatten() {
        if previous_candidate == Some(rate) {
            continue;
        }
        previous_candidate = Some(rate);
        if !accepts(rate) {
            continue;
        }
        if selected.is_some_and(|previous| previous != rate) {
            return None;
        }
        selected = Some(rate);
    }
    selected
}

pub(crate) fn mp4_rate(track: &filmcraft_isobmff::Track) -> Option<FrameRate> {
    if !(2..=MAX_SAMPLES).contains(&track.samples.len())
        || track.edit_offset != 0
        || track.entries.len() != 1
        || track.edits.len() > 1
        || track.edits.iter().any(|edit| edit.media_time != 0 || edit.media_rate != 0x0001_0000)
        || track.samples.iter().any(|sample| sample.description_index != 1)
    {
        return None;
    }
    let entry = track.entries.first()?;
    // avc3 permits in-band configuration changes; this initial contract excludes that layout.
    if entry.format.0 != *b"avc1" {
        return None;
    }
    let CodecConfig::Avc(config) = &entry.codec else { return None };
    let hint = avc_hint(config)?;
    let first_duration = track.samples.first()?.duration;
    let uniform = (first_duration > 0 && track.samples.iter().all(|sample| sample.duration == first_duration))
        .then(|| FrameRate::new(i64::from(track.timescale), i64::from(first_duration)));
    let terminal = i64::try_from(track.duration).ok()?;
    choose([hint, uniform], |rate| {
        validates(track.samples.len(), rate, (1, u64::from(track.timescale)), Some(terminal), |index| {
            let sample = track.samples.get(index)?;
            Some((sample.pts, Some(sample.pts.checked_add(i64::from(sample.duration))?)))
        })
    })
}

pub(crate) fn mkv_rate(track: &filmcraft_matroska::Track) -> Option<FrameRate> {
    if !(2..=MAX_SAMPLES).contains(&track.samples.len()) || track.codec_delay_ns != 0 {
        return None;
    }
    let filmcraft_matroska::Codec::Avc { avcc } = &track.codec else { return None };
    if avcc.len() > MAX_AVCC_BYTES {
        return None;
    }
    let hint = avc_hint(&AvcConfig::parse(avcc).ok()?)?;
    let duration = track.default_duration_ns.filter(|duration| *duration > 0).map(|duration| FrameRate::from_f64(1e9 / duration as f64));
    choose([hint, duration], |rate| validates(track.samples.len(), rate, track.timebase, None, |index| Some((track.samples.get(index)?.pts, None))))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Literal timestamps from independently generated 12-frame, 1000-timescale fixtures.
    const NEAREST: [i64; 12] = [0, 33, 67, 100, 133, 167, 200, 233, 267, 300, 333, 367];
    const FLOOR: [i64; 12] = [0, 33, 66, 100, 133, 166, 200, 233, 266, 300, 333, 366];

    fn accepts(pts: &[i64], terminal: i64) -> bool {
        validates(pts.len(), FrameRate::FPS_30, (1, 1000), Some(terminal), |index| {
            Some((*pts.get(index)?, Some(pts.get(index + 1).copied().unwrap_or(terminal))))
        })
    }

    #[test]
    fn nearest_and_floor_tables_validate_but_vfr_and_terminal_disagreement_do_not() {
        assert!(accepts(&NEAREST, 400));
        assert!(accepts(&FLOOR, 400));
        assert!(!accepts(&FLOOR, 399));
        assert!(!accepts(&[0, 40, 100, 120, 180, 220, 260, 310, 335, 375, 430, 480], 505));
        assert!(!accepts(&[0, 33, 67, 100, 133, 167, 167, 233, 267, 300, 333, 367], 400));
        assert!(!accepts(&[1, 34, 68, 101, 134, 168, 201, 234, 268, 301, 334, 368], 401));
    }

    #[test]
    fn misleading_rate_and_excessive_count_fall_back_without_reading_more_samples() {
        assert!(!validates(FLOOR.len(), FrameRate::new(1000, 1), (1, 1000), None, |index| Some((*FLOOR.get(index)?, None))));
        assert!(!validates(MAX_SAMPLES + 1, FrameRate::FPS_30, (1, 1000), None, |_| panic!("a rejected table must not be scanned")));
        assert!(!validates(12, FrameRate::new(0, 1), (1, 1000), None, |_| panic!("a rejected rate must not be scanned")));
    }

    #[test]
    fn logical_ordinal_keeps_the_existing_one_tick_reverse_clock() {
        let rate = FrameRate::new(31, 1);
        assert_eq!(sample_index(rate, rate.tick_of(9) - Tick(1), 12), Some(8));
        assert_eq!(sample_index(rate, rate.tick_of(9), 12), Some(9));
        assert_eq!(sample_index(rate, rate.tick_of(1) - Tick(1), 12), Some(0));
        assert_eq!(sample_index(rate, Tick::ZERO, 0), None);
        assert_eq!(sample_index(rate, Tick::MAX, 12), Some(11));
    }
}
