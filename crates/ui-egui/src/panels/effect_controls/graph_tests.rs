use super::*;
use filmcraft_geom::Vec2;
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, MediaSource};
use filmcraft_project::{ItemKind, Keyframe, Label, MediaClip, MediaRef, Param, Project, SequenceSettings};
use filmcraft_time::FrameRate;
use std::sync::Arc;

fn fixture() -> (filmcraft_engine::Session, TrackItem) {
    let rate = FrameRate::FPS_24;
    let source = GeneratorSource::new(Generator::BlackVideo, 32, 18, rate, Tick::from_seconds_f64(8.0));
    let mut project = Project::new("Point plots");
    let item = project.add_item(
        "Clip",
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::Generator(Generator::BlackVideo),
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
    let seq = project.new_sequence("Cut", SequenceSettings { width: 32, height: 18, frame_rate: rate, ..Default::default() }, 1, 1, None);
    let mut session = filmcraft_engine::Session::default();
    session.project = Arc::new(project);
    session.state.active_sequence = Some(seq);
    session.state.open_sequences = vec![seq];
    session.execute("timeline.place", json!({"item": item.0, "track": "V1", "time": 0, "insert": false})).unwrap();
    let clip = session.active_sequence().unwrap().video_tracks[0].items[0].clone();
    (session, clip)
}

fn draw(param: &Param, kind: ParamKind) -> (egui::FullOutput, FilmcraftApp) {
    let (_, item) = fixture();
    draw_item(param, kind, item)
}

fn draw_item(param: &Param, kind: ParamKind, it: TrackItem) -> (egui::FullOutput, FilmcraftApp) {
    draw_effect(param, kind, it, 0)
}

fn draw_effect(param: &Param, kind: ParamKind, it: TrackItem, idx: usize) -> (egui::FullOutput, FilmcraftApp) {
    let (s, _) = fixture();
    let mut pd = filmcraft_project::find_effect("motion").unwrap().params.iter().find(|p| p.id == "position").unwrap().clone();
    pd.kind = kind;
    let mut app = FilmcraftApp::new(s);
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))), ..Default::default() }, |ui| {
        let body = Rect::from_min_max(Pos2::ZERO, pos2(500.0, 800.0));
        let lane = Rect::from_min_max(pos2(504.0, 0.0), pos2(904.0, 800.0));
        ui.set_width(body.width());
        let mut actions = Vec::new();
        graph_rows(&mut app, ui, body, it.id, idx, None, &pd, param, &lane, &it, &mut actions);
        assert!(actions.is_empty());
    });
    // These tests inspect shapes without uploading textures to a renderer.
    output.textures_delta.clear();
    (output, app)
}

#[test]
fn point_plots_draw_the_coordinate_values_and_velocities() {
    let (_, it) = fixture();
    let a = ParamValue::Vec2(Vec2::new(800.0, 400.0));
    let b = ParamValue::Vec2(Vec2::new(1200.0, 400.0));
    let param = Param { value: a.clone(), keyframes: vec![Keyframe::new(it.source_in, a), Keyframe::new(it.source_in + it.duration, b)] };
    let (output, _) = draw(&param, ParamKind::Point);
    let value_lines: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Path(p) if p.stroke.width == 1.5 && p.points.len() > 100 => Some(p),
            _ => None,
        })
        .collect();
    let velocity_lines: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Path(p) if p.stroke.width == 1.2 && p.points.len() > 100 => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(value_lines.len(), 2, "one value curve per coordinate");
    assert_eq!(velocity_lines.len(), 2, "one velocity curve per coordinate");
    assert!(value_lines[0].points.first().unwrap().y > value_lines[0].points.last().unwrap().y + 50.0, "X increases across the clip");
    let y = value_lines[1].points[0].y;
    assert!(value_lines[1].points.iter().all(|p| (p.y - y).abs() < 0.01), "Y stays constant");
    assert!(value_lines.iter().chain(&velocity_lines).flat_map(|p| &p.points).all(|p| p.is_finite()));
}

#[test]
fn unresolved_auto_and_extreme_points_do_not_create_invalid_geometry_or_edits() {
    for (a, b) in [(Vec2::new(f64::NAN, 400.0), Vec2::new(f64::NAN, 800.0)), (Vec2::new(-f64::MAX, 400.0), Vec2::new(f64::MAX, 800.0))] {
        let (_, it) = fixture();
        let param = Param {
            value: ParamValue::Vec2(a),
            keyframes: vec![Keyframe::new(it.source_in, ParamValue::Vec2(a)), Keyframe::new(it.source_in + it.duration, ParamValue::Vec2(b))],
        };
        let (output, app) = draw(&param, ParamKind::Point);
        for shape in output.shapes {
            if let egui::Shape::Path(path) = shape.shape {
                assert!(path.points.iter().all(|p| p.is_finite()));
            }
        }
        if a.x.is_nan() {
            assert!(!app.auto.elements.iter().any(|e| e.id.contains(".keyframe.")), "leave unresolved auto coordinates unchanged");
        }
    }
}

#[test]
fn speed_and_reverse_plots_use_sequence_time_and_actual_velocity() {
    for reverse in [false, true] {
        let (_, mut item) = fixture();
        item.start = Tick::from_seconds_f64(3.0);
        item.source_in = Tick::from_seconds_f64(1.0);
        item.duration = Tick::from_seconds_f64(2.0);
        item.speed = 2.0;
        item.reverse = reverse;
        let lo = item.effect_time_at(if reverse { item.end() } else { item.start });
        let hi = item.effect_time_at(if reverse { item.start } else { item.end() });
        let mut keys = Vec::new();
        for fraction in [0.0, 0.25, 0.75, 1.0] {
            keys.push(Keyframe::new(lo + (hi - lo).mul_ratio((fraction * 4.0) as i64, 4), ParamValue::Vec2(Vec2::new(100.0 + 400.0 * fraction, 400.0))));
        }
        let param = Param { value: keys[0].value.clone(), keyframes: keys };
        let quarter = item.start + item.duration.mul_ratio(1, 4);
        let media = item.effect_time_at(quarter);
        let (output, app) = draw_item(&param, ParamKind::Point, item.clone());
        let lines: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Path(p) if p.stroke.width == 1.5 && p.points.len() > 100 => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(lines.len(), 2);
        let first = lines[0].points.first().unwrap().y;
        let last = lines[0].points.last().unwrap().y;
        assert!(if reverse { first < last - 50.0 } else { first > last + 50.0 }, "curve follows sequence order, reverse={reverse}");
        assert!(
            output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == "230.0/s")),
            "400 source units over two sequence seconds is 200/s, padded to 230/s"
        );
        let velocities: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Path(p) if p.stroke.width == 1.2 && p.points.len() > 100 => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(velocities.len(), 2);
        let velocity_area = app.auto.elements.iter().find(|e| e.id == "effectControls.position.graph.x.velocity").unwrap().rect;
        assert!(
            velocities[0].points.iter().all(|p| if reverse {
                p.y > (velocity_area[1] + velocity_area[3] / 2.0)
            } else {
                p.y < (velocity_area[1] + velocity_area[3] / 2.0)
            }),
            "velocity sign follows sequence direction"
        );
        let key = app.auto.elements.iter().find(|e| e.id == format!("effectControls.position.graph.x.keyframe.{}", media.0)).unwrap();
        assert!(((key.rect[0] + key.rect[2] / 2.0) - 604.0).abs() < 0.01, "quarter-sequence key uses the quarter lane position");
        let timing = ClipTimeMap::new(&item, &item.effects[0]).unwrap();
        assert_eq!(timing.timeline_at_media(media), Some(quarter));
    }
}

#[test]
fn effect_time_mapping_respects_filter_holds_and_rejects_invalid_spans() {
    let (_, mut item) = fixture();
    item.frame_hold = Some(Tick::from_seconds_f64(1.0));
    item.hold_filters = false;
    let timing = ClipTimeMap::new(&item, &item.effects[0]).unwrap();
    assert_eq!(timing.media_at_fraction(0.5), Some(item.effect_time_at(item.start + item.duration.mul_ratio(1, 2))));
    assert_ne!(timing.media_at_fraction(0.5), item.frame_hold);
    item.hold_filters = true;
    let timing = ClipTimeMap::new(&item, &item.effects[0]).unwrap();
    assert_eq!(timing.media_at_fraction(0.5), item.frame_hold);
    assert_eq!(timing.timeline_at_media(item.frame_hold.unwrap()), Some(item.start));
    assert!(timing.timeline_at_fraction(f64::NAN).is_none());
    for speed in [0.0, f64::NAN, f64::INFINITY, f64::MAX] {
        item.speed = speed;
        assert!(ClipTimeMap::new(&item, &item.effects[0]).is_none());
    }
    item.speed = 1.0;
    item.start = Tick(i64::MAX);
    assert!(ClipTimeMap::new(&item, &item.effects[0]).is_none());
}

fn responsive_item(reverse: bool) -> (TrackItem, usize) {
    let (_, mut item) = fixture();
    item.start = Tick::from_seconds_f64(3.0);
    item.source_in = Tick::from_seconds_f64(2.0);
    item.duration = Tick::from_seconds_f64(8.0);
    item.speed = 2.0;
    item.reverse = reverse;
    item.graphic = Some(Box::new(filmcraft_project::graphic_design::GraphicMeta {
        design_in: Tick::from_seconds_f64(1.0),
        design_duration: Tick::from_seconds_f64(8.0),
        intro: Tick::from_seconds_f64(1.0),
        outro: Tick::from_seconds_f64(1.0),
        ..Default::default()
    }));
    let layer = item.effects.len();
    item.effects.push(filmcraft_project::graphic::new_shape_layer(0, Vec2::new(8.0, 9.0), Vec2::new(6.0, 6.0), vec![]));
    (item, layer)
}

#[test]
fn responsive_layer_clock_and_inverse_follow_rendering_at_protected_boundaries() {
    let second = filmcraft_time::TICKS_PER_SECOND;
    for reverse in [false, true] {
        let (item, layer) = responsive_item(reverse);
        let timing = ClipTimeMap::new(&item, &item.effects[layer]).unwrap();
        let motion = ClipTimeMap::new(&item, &item.effects[0]).unwrap();
        for offset in [-second, 0, second / 4, second / 2, second, 4 * second, 7 * second, 15 * second / 2, 8 * second, 9 * second] {
            let timeline = item.start + Tick(offset);
            let raw = item.effect_time_at(timeline);
            let actual = filmcraft_render::graphic_clip::parameter_time(&item, &item.effects[layer], raw);
            assert_eq!(timing.media_at_timeline(timeline), Some(actual));
            assert_eq!(motion.media_at_timeline(timeline), Some(raw), "intrinsic Motion keeps the raw clock");
            let inverse = timing.timeline_at_media(actual).unwrap();
            assert!((inverse.0 - timeline.0).abs() <= 2, "inverse agrees within integer stretch rounding: {inverse:?} {timeline:?}");
            assert_eq!(timing.media_at_timeline(inverse), Some(actual));
        }
        assert_ne!(timing.media_at_fraction(0.25), motion.media_at_fraction(0.25));
        assert_eq!(timing.reversed(), reverse);
    }
}

#[test]
fn responsive_point_curve_key_positions_and_velocity_use_the_layer_clock() {
    let second = filmcraft_time::TICKS_PER_SECOND;
    for reverse in [false, true] {
        let (item, layer) = responsive_item(reverse);
        let times = [second, 2 * second, 8 * second, 9 * second];
        let param = Param {
            value: ParamValue::Vec2(Vec2::new(1.0, 9.0)),
            keyframes: times.into_iter().map(|t| Keyframe::new(Tick(t), ParamValue::Vec2(Vec2::new(t as f64 / second as f64, 9.0)))).collect(),
        };
        let (output, app) = draw_effect(&param, ParamKind::Point, item.clone(), layer);
        let value_lines: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Path(p) if p.stroke.width == 1.5 && p.points.len() > 100 => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(value_lines.len(), 2);
        let timing = ClipTimeMap::new(&item, &item.effects[layer]).unwrap();
        let area = app.auto.elements.iter().find(|e| e.id == "effectControls.position.graph.x.value").unwrap().rect;
        let (lo, hi) = graph_range([1.0, 9.0].into_iter(), Some((-100_000.0, 100_000.0)));
        for fraction in [0.25, 0.5, 0.75] {
            let clock = timing.media_at_fraction(fraction).unwrap();
            let x = param.value_at(clock).as_vec2().unwrap().x;
            let expected_y = area[1] + area[3] - ((x - lo) / (hi - lo)) as f32 * area[3];
            let point = value_lines[0].points[(fraction * (value_lines[0].points.len() - 1) as f64) as usize];
            assert!((point.y - expected_y).abs() < 0.01, "curve reads the actual graphic layer clock");
        }
        for media in [Tick(2 * second), Tick(8 * second)] {
            let key = app.auto.elements.iter().find(|e| e.id == format!("effectControls.position.graph.x.keyframe.{}", media.0)).unwrap();
            let fraction = timing.fraction_at_media(media).unwrap();
            assert!((key.rect[0] + key.rect[2] / 2.0 - (504.0 + 400.0 * fraction)).abs() < 0.01);
            let expected = if reverse {
                if media.0 == 2 * second { 15.0 / 16.0 } else { 1.0 / 16.0 }
            } else if media.0 == 2 * second {
                1.0 / 16.0
            } else {
                15.0 / 16.0
            };
            assert!((fraction - expected).abs() < 1e-6, "protected boundaries retain their sequence positions");
        }
        assert!(
            output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == "2.3/s")),
            "protected ends run at two design units per sequence second"
        );
    }
}

#[test]
fn responsive_inverse_handles_constant_holds_plateaus_and_unreachable_keys() {
    let second = filmcraft_time::TICKS_PER_SECOND;
    let (mut item, layer) = responsive_item(false);
    item.frame_hold = Some(item.source_in + Tick(4 * second));
    item.hold_filters = true;
    let timing = ClipTimeMap::new(&item, &item.effects[layer]).unwrap();
    let held = filmcraft_render::graphic_clip::parameter_time(&item, &item.effects[layer], item.frame_hold.unwrap());
    assert_eq!(timing.media_at_fraction(0.75), Some(held));
    assert_eq!(timing.timeline_at_media(held), Some(item.start));
    assert_eq!(timing.timeline_at_media(held + Tick(1)), None);
    item.hold_filters = false;
    assert_ne!(ClipTimeMap::new(&item, &item.effects[layer]).unwrap().media_at_fraction(0.75), Some(held));
    item.frame_hold = None;
    // A designed middle with zero length produces a plateau when the clip is extended.
    item.graphic.as_mut().unwrap().design_duration = Tick(2 * second);
    let timing = ClipTimeMap::new(&item, &item.effects[layer]).unwrap();
    let middle = Tick(2 * second);
    let position = timing.timeline_at_media(middle).unwrap();
    assert_eq!(timing.media_at_timeline(position), Some(middle));
    // Shortening below the combined protected ends skips a design interval.
    item.graphic.as_mut().unwrap().design_duration = Tick(8 * second);
    item.duration = Tick(second / 2);
    let timing = ClipTimeMap::new(&item, &item.effects[layer]).unwrap();
    assert_eq!(timing.timeline_at_media(Tick(4 * second)), None);
    // Neither a saturated endpoint nor an invalid project span may create overflowing inverse
    // arithmetic or send navigation to an unrepresentable sequence time.
    item.source_in = Tick(i64::MAX - second);
    assert_eq!(ClipTimeMap::new(&item, &item.effects[layer]).unwrap().timeline_at_media(Tick(i64::MIN)), None);
    item.graphic.as_mut().unwrap().intro = Tick(i64::MAX);
    assert!(ClipTimeMap::new(&item, &item.effects[layer]).is_none());
}

#[test]
fn mixed_interpolation_handles_and_curves_follow_the_active_segment_in_both_directions() {
    use filmcraft_project::Interpolation;
    for reverse in [false, true] {
        for point in [false, true] {
            for preceding in [Interpolation::Bezier, Interpolation::ContinuousBezier] {
                let (_, mut item) = fixture();
                item.start = Tick::from_seconds_f64(3.0);
                item.duration = Tick::from_seconds_f64(2.0);
                item.source_in = Tick::from_seconds_f64(1.0);
                item.speed = 2.0;
                item.reverse = reverse;
                let low = item.effect_time_at(if reverse { item.end() } else { item.start });
                let high = item.effect_time_at(if reverse { item.start } else { item.end() });
                let a_time = low + (high - low).mul_ratio(1, 4);
                let b_time = low + (high - low).mul_ratio(3, 4);
                let values = if point {
                    [ParamValue::Vec2(Vec2::new(8.0, 9.0)), ParamValue::Vec2(Vec2::new(20.0, 9.0))]
                } else {
                    [ParamValue::Float(20.0), ParamValue::Float(80.0)]
                };
                let mut a = Keyframe::new(a_time, values[0].clone());
                a.interp = preceding;
                a.out_influence = 0.25;
                let mut b = Keyframe::new(b_time, values[1].clone());
                b.in_influence = 0.25;
                assert_eq!(b.interp, Interpolation::Linear);
                let param = Param { value: values[0].clone(), keyframes: vec![a, b] };
                let kind =
                    if point { ParamKind::Point } else { ParamKind::Float { min: 0.0, max: 100.0, soft_min: 0.0, soft_max: 100.0, unit: "", decimals: 1 } };
                let (output, app) = draw_item(&param, kind, item.clone());
                let suffix = if point { ".x" } else { "" };
                let graph_id = format!("effectControls.position.graph{suffix}");
                let element = |id: &str| app.auto.elements.iter().find(|e| e.id == id).unwrap_or_else(|| panic!("missing {id}"));
                let key = element(&format!("{graph_id}.keyframe.{}", b_time.0));
                let handle = element(&format!("{graph_id}.keyframe.{}.in", b_time.0));
                let (kx, hx) = (key.rect[0] + key.rect[2] / 2.0, handle.rect[0] + handle.rect[2] / 2.0);
                assert!(if reverse { hx > kx } else { hx < kx }, "incoming influence points toward its actual sequence neighbour");
                if point {
                    element(&format!("effectControls.position.graph.y.keyframe.{}.in", b_time.0));
                }
                let value_lines: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Path(p) if p.stroke.width == 1.5 && p.points.len() > 100 => Some(p),
                        _ => None,
                    })
                    .collect();
                let velocity_lines: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Path(p) if p.stroke.width == 1.2 && p.points.len() > 100 => Some(p),
                        _ => None,
                    })
                    .collect();
                assert_eq!(value_lines.len(), if point { 2 } else { 1 });
                assert_eq!(velocity_lines.len(), value_lines.len());
                let area = element(&format!("{graph_id}.value")).rect;
                let velocity_area = element(&format!("{graph_id}.velocity")).rect;
                let n = value_lines[0].points.len() - 1;
                let component = if point { GraphComponent::X } else { GraphComponent::Scalar };
                let samples: Vec<_> = (0..=n)
                    .map(|i| {
                        let timeline = item.start + Tick((item.duration.0 as f64 * i as f64 / n as f64).round() as i64);
                        component.read(&param.value_at(item.effect_time_at(timeline))).unwrap()
                    })
                    .collect();
                let edit_bounds = if point { (-100_000.0, 100_000.0) } else { (0.0, 100.0) };
                let (lo, hi) = graph_range(samples.iter().copied(), Some(edit_bounds));
                for fraction in [0.375, 0.5, 0.625] {
                    let i = (fraction * n as f64) as usize;
                    let expected_y = area[1] + area[3] - ((samples[i] - lo) / (hi - lo)) as f32 * area[3];
                    assert!((value_lines[0].points[i].y - expected_y).abs() < 0.01, "value graph evaluates the same mixed easing as the renderer");
                }
                let seconds_per_sample = item.duration.seconds() / n as f64;
                let velocities: Vec<_> = samples.windows(2).map(|v| (v[1] - v[0]) / seconds_per_sample).collect();
                let max = velocities.iter().fold(1e-6_f64, |a, v| a.max(v.abs())) * 1.15;
                for i in [n / 3, n / 2, n * 2 / 3] {
                    let expected_y = velocity_area[1] + velocity_area[3] / 2.0 - (velocities[i] / max) as f32 * velocity_area[3] / 2.0;
                    assert!((velocity_lines[0].points[i].y - expected_y).abs() < 0.01, "velocity measures the active easing per sequence second");
                }
                let sample = a_time + (b_time - a_time).mul_ratio(1, 3);
                let before = component.read(&param.value_at(sample)).unwrap();
                let mut changed = param;
                changed.keyframes[1].in_influence = 0.6;
                let after = component.read(&changed.value_at(sample)).unwrap();
                assert!(after > before + if point { 1.0 } else { 5.0 }, "the missing incoming control has a real effect on the evaluated curve");
            }
        }
    }
}
