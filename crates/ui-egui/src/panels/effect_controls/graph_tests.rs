use super::*;
use filmcraft_geom::Vec2;
use filmcraft_project::{Keyframe, Param};

fn draw(param: &Param, kind: ParamKind) -> (egui::FullOutput, FilmcraftApp) {
    let mut s = filmcraft_engine::Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let it = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
    let mut pd = filmcraft_project::find_effect("motion").unwrap().params.iter().find(|p| p.id == "position").unwrap().clone();
    pd.kind = kind;
    let mut app = FilmcraftApp::new(s);
    let ctx = egui::Context::default();
    let output = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))), ..Default::default() }, |ui| {
        let body = Rect::from_min_max(Pos2::ZERO, pos2(500.0, 800.0));
        let lane = Rect::from_min_max(pos2(504.0, 0.0), pos2(904.0, 800.0));
        ui.set_width(body.width());
        let mut actions = Vec::new();
        graph_rows(&mut app, ui, body, it.id, 0, None, &pd, param, &lane, &it, &mut actions);
        assert!(actions.is_empty());
    });
    (output, app)
}

#[test]
fn point_plots_draw_the_coordinate_values_and_velocities() {
    let mut s = filmcraft_engine::Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let it = &s.active_sequence().unwrap().video_tracks[0].items[0];
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
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let it = &s.active_sequence().unwrap().video_tracks[0].items[0];
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
