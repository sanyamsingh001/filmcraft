//! Control clicks stay short when the host draws slowly; drags keep separate steps.

use egui::{Event, Key, Rect};
use serde_json::{Value, json};

use super::{ControlRequest, Outcome};
use crate::FilmcraftApp;

struct Probe {
    rect: Rect,
    clicks: usize,
    double_clicks: usize,
    drag_started: usize,
    dragged: usize,
    drag_stopped: usize,
    events: Vec<Vec<Event>>,
    keys: Vec<(bool, bool, bool)>,
    text: String,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            rect: Rect::NOTHING,
            clicks: 0,
            double_clicks: 0,
            drag_started: 0,
            dragged: 0,
            drag_stopped: 0,
            events: Vec::new(),
            keys: Vec::new(),
            text: String::new(),
        }
    }
}

fn frame(app: &mut FilmcraftApp, ctx: &egui::Context, time: f64, probe: &mut Probe) {
    let mut raw = egui::RawInput { time: Some(time), screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 200.0))), ..Default::default() };
    eframe::App::raw_input_hook(app, ctx, &mut raw);
    probe.events.push(raw.events.clone());
    app.auto.begin_frame();
    let mut output = ctx.run_ui(raw, |ui| {
        let response = ui.add_sized(egui::vec2(220.0, 40.0), egui::Button::new("Probe").sense(egui::Sense::click_and_drag()));
        probe.rect = response.rect;
        app.auto.add("probe.button", response.rect, "Probe");
        probe.clicks += usize::from(response.clicked());
        probe.double_clicks += usize::from(response.double_clicked());
        probe.drag_started += usize::from(response.drag_started());
        probe.dragged += usize::from(response.dragged());
        probe.drag_stopped += usize::from(response.drag_stopped());
        probe.keys.push(ui.input(|i| (i.key_pressed(Key::K), i.key_released(Key::K), i.key_down(Key::K))));
        ui.input(|i| {
            for event in &i.events {
                if let Event::Text(text) = event {
                    probe.text.push_str(text);
                }
            }
        });
    });
    output.textures_delta.clear();
}

fn request(app: &mut FilmcraftApp, ctx: &egui::Context, method: &str, params: Value) {
    let (req, _) = ControlRequest::new(method, params);
    assert!(matches!(super::handle(app, ctx, &req), Outcome::AfterInput));
}

fn setup() -> (FilmcraftApp, egui::Context, Probe) {
    let mut app = FilmcraftApp::new(filmcraft_engine::Session::default());
    let ctx = egui::Context::default();
    let mut probe = Probe::default();
    frame(&mut app, &ctx, 0.0, &mut probe);
    (app, ctx, probe)
}

#[test]
fn a_control_click_fires_once_with_three_second_frame_spacing() {
    let (mut app, ctx, mut probe) = setup();
    request(&mut app, &ctx, "ui.click", json!({"id": "probe.button"}));
    // A press held from one host frame to the next exceeds egui's click duration.
    for time in [3.0, 6.0, 9.0] {
        frame(&mut app, &ctx, time, &mut probe);
    }
    assert_eq!(probe.clicks, 1);
    assert_eq!(probe.double_clicks, 0);
    assert_eq!(probe.drag_started, 0);
    assert_eq!(probe.events[1].len(), 3, "move, press and release share the first input packet");
    assert!(probe.events[2].is_empty() && probe.events[3].is_empty());
    assert!(app.synthetic.is_empty());
    assert!(!ctx.input(|i| i.pointer.any_down()));
}

#[test]
fn a_double_click_count_is_preserved_in_one_slow_host_frame() {
    let (mut app, ctx, mut probe) = setup();
    request(&mut app, &ctx, "ui.click", json!({"id": "probe.button", "count": 2}));
    frame(&mut app, &ctx, 3.0, &mut probe);
    frame(&mut app, &ctx, 6.0, &mut probe);
    assert_eq!(probe.clicks, 1, "egui reports one response for the completed double click");
    assert_eq!(probe.double_clicks, 1);
    assert_eq!(probe.events[1].len(), 5);
    assert!(probe.events[2].is_empty());
    assert!(app.synthetic.is_empty());
}

#[test]
fn drag_steps_remain_separate_and_produce_a_drag_at_slow_frame_rate() {
    let (mut app, ctx, mut probe) = setup();
    let a = probe.rect.center();
    let b = a + egui::vec2(60.0, 0.0);
    request(&mut app, &ctx, "ui.drag", json!({"from": {"x": a.x, "y": a.y}, "to": {"x": b.x, "y": b.y}, "steps": 3}));
    for time in [3.0, 6.0, 9.0, 12.0, 15.0, 18.0] {
        frame(&mut app, &ctx, time, &mut probe);
    }
    assert_eq!(probe.drag_started, 1);
    assert!(probe.dragged >= 3);
    assert_eq!(probe.drag_stopped, 1);
    assert_eq!(probe.clicks, 0);
    assert!(probe.events[1..].iter().all(|events| events.len() == 1));
    assert!(app.synthetic.is_empty());
    assert!(!ctx.input(|i| i.pointer.any_down()));
}

#[test]
fn click_packets_do_not_merge_or_reorder_key_and_text_runs() {
    let (mut app, ctx, mut probe) = setup();
    request(&mut app, &ctx, "ui.click", json!({"id": "probe.button"}));
    request(&mut app, &ctx, "ui.key", json!({"key": "Cmd+Shift+K"}));
    request(&mut app, &ctx, "ui.type", json!({"text": "typed once"}));
    frame(&mut app, &ctx, 3.0, &mut probe);
    assert_eq!(probe.clicks, 1);
    assert_eq!(probe.keys[1], (false, false, false));
    assert!(probe.text.is_empty());
    frame(&mut app, &ctx, 6.0, &mut probe);
    assert_eq!(probe.keys[2], (true, true, false));
    let [Event::Key { key: Key::K, pressed: true, repeat: false, modifiers, .. }, Event::Key { key: Key::K, pressed: false, .. }] = probe.events[2].as_slice()
    else {
        panic!("key press/release must stay together");
    };
    assert!(modifiers.command && modifiers.shift);
    assert!(probe.text.is_empty());
    frame(&mut app, &ctx, 9.0, &mut probe);
    assert_eq!(probe.text, "typed once");
    assert!(matches!(probe.events[3].as_slice(), [Event::Text(_), Event::Key { key: Key::F35, pressed: false, .. }]));
    frame(&mut app, &ctx, 12.0, &mut probe);
    assert_eq!(probe.clicks, 1);
    assert_eq!(probe.text, "typed once");
    assert!(app.synthetic.is_empty());
}

#[test]
fn an_oversized_click_count_is_rejected_without_queued_input() {
    let (mut app, ctx, _) = setup();
    let (req, _) = ControlRequest::new("ui.click", json!({"id": "probe.button", "count": 65}));
    let Outcome::Done(reply) = super::handle(&mut app, &ctx, &req) else { panic!("an oversized packet must return an error") };
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().unwrap().contains("count"));
    assert!(app.synthetic.is_empty());
}
