//! Headless UI tests of the M3.11 menu long tail: Find, Create Search Bin (and its Project panel
//! rows), Project Settings, Scene Edit Detection, Simplify Sequence, Automate to Sequence,
//! Normalize Mix Track, Dynamic Audio Waveforms, Reveal Log Files and the System Compatibility
//! Report, all driven by automation id through the control channel.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_engine::project::ItemId;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn new() -> Self {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let (tx, rx) = channel();
        let mut app = FilmcraftApp::new(s).with_control(rx);
        // never hand paths to the real OS opener from a test
        app.hooks.open_path = Some(Box::new(|_, _| Ok(())));
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx };
        d.frames(4);
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

    fn call(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                return v;
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let v = self.call(method, params.clone());
        assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
        v["result"].clone()
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    fn menu(&mut self, id: &str) -> Value {
        let r = self.ok("ui.menu.invoke", json!({"id": id}));
        self.frames(3);
        r
    }

    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(3);
    }

    fn has(&mut self, id: &str) -> bool {
        let v = self.ok("ui.elements", json!({"prefix": id}));
        v.as_array().unwrap().iter().any(|e| e["id"] == json!(id))
    }

    fn app(&mut self) -> &mut FilmcraftApp {
        self.harness.state_mut()
    }

    fn item_named(&mut self, name: &str) -> u64 {
        self.app().session.project.items.values().find(|i| i.name == name).unwrap().id.0
    }

    fn dialog_open(&mut self) -> bool {
        self.app().ui.extras.dialog.is_some()
    }
}

#[test]
fn menu_items_toggles_and_help() {
    let mut d = Driver::new();
    let menus = d.ok("ui.menu.list", json!({}));
    let find = |id: &str| menus.as_array().unwrap().iter().find(|m| m["id"] == id).cloned().unwrap_or_else(|| panic!("{id} not in menus"));
    assert_eq!(find("view.dynamicAudioWaveforms")["checked"], true);
    assert_eq!(find("help.systemCompatibilityReport")["path"], json!(["Help"]));
    assert_eq!(find("file.mediaProperties")["path"], json!(["File", "Get Media File Properties for"]));
    assert_eq!(find("clip.sceneEditDetection")["label"], "Scene Edit Detection…");
    // Dynamic Audio Waveforms toggles
    d.menu("view.dynamicAudioWaveforms");
    assert!(!d.app().ui.extras.dynamic_waveforms);
    let menus = d.ok("ui.menu.list", json!({}));
    assert_eq!(menus.as_array().unwrap().iter().find(|m| m["id"] == "view.dynamicAudioWaveforms").unwrap()["checked"], false);
    // System Compatibility Report
    assert_eq!(d.menu("help.systemCompatibilityReport")["dialog"], "systemReport");
    assert!(d.has("systemReport.ok"));
    d.click("systemReport.ok");
    assert!(!d.dialog_open());
    // Reveal Log Files writes this session's log and reveals it
    let r = d.menu("help.revealLogFiles");
    let path = r["path"].as_str().unwrap().to_string();
    assert!(std::fs::read_to_string(&path).unwrap().contains("\"os\""));
    assert_eq!(d.app().ui.extras.opened.last(), Some(&path));
    let _ = std::fs::remove_file(&path);
    // Import from Media Browser needs a Media Browser selection
    assert_eq!(d.call("ui.menu.invoke", json!({"id": "file.importFromMediaBrowser"}))["ok"], false);
}

#[test]
fn import_image_sequence_from_the_file_menu_and_media_browser() {
    let dir = std::env::temp_dir().join(format!("fc-ui-imgseq-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let paths: Vec<String> = (1..=5)
        .map(|n| {
            let p = dir.join(format!("plate_{n:03}.png"));
            image::RgbaImage::from_pixel(16, 9, image::Rgba([n as u8 * 40, 0, 0, 255])).save(&p).unwrap();
            p.to_string_lossy().to_string()
        })
        .collect();
    let mut d = Driver::new();
    let menus = d.ok("ui.menu.list", json!({}));
    let item = menus.as_array().unwrap().iter().find(|m| m["id"] == "file.importImageSequence").cloned().expect("File ▸ Import Image Sequence…");
    assert_eq!(item["path"], json!(["File"]));
    // File ▸ Import Image Sequence… with the first frame chosen
    let r = d.ok("ui.menu.invoke", json!({"id": "file.importImageSequence", "params": {"path": paths[0]}}));
    assert_eq!(r["imageSequences"][0]["frames"], 5);
    let id = ItemId(r["items"][0].as_u64().unwrap());
    let kind = d.app().session.project.item(id).unwrap().as_media().unwrap().info.kind;
    assert_eq!(kind, filmcraft_media::MediaKind::ImageSequence);
    // Media Browser ▸ Import as Image Sequence (the selection, from frame 3)
    d.exec("mediaBrowser.select", json!({"paths": [paths[2]]}));
    let r = d.ok("ui.menu.invoke", json!({"id": "file.importFromMediaBrowser", "params": {"imageSequence": true}}));
    assert_eq!(r["imageSequences"][0]["frames"], 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn find_dialog_and_search_bins() {
    let mut d = Driver::new();
    d.ok("ui.set", json!({"focused": "Project"}));
    assert_eq!(d.menu("edit.find")["dialog"], "find");
    for id in
        ["find.scope.project", "find.row.0.column", "find.row.0.operator", "find.row.0.text", "find.row.1.text", "find.match.any", "find.ok", "find.cancel"]
    {
        assert!(d.has(id), "{id}");
    }
    d.ok(
        "ui.set",
        json!({"menuDialog": {"rows": [{"column": "Name", "operator": "beginsWith", "text": "Neon"}, {"column": "Name", "operator": "contains", "text": ""}]}}),
    );
    d.click("find.ok");
    let neon = d.item_named("Neon_Loop.mov");
    assert_eq!(d.app().session.state.project_selection, vec![ItemId(neon)]);
    assert!(d.dialog_open(), "Find stays open until Done");
    d.click("find.cancel");
    assert!(!d.dialog_open());
    // timeline Find from the Timeline panel
    d.ok("ui.set", json!({"focused": "Timeline"}));
    d.menu("edit.find");
    assert_eq!(d.app().ui.extras.dialog.as_ref().unwrap().params["scope"], "timeline");
    d.ok("ui.set", json!({"menuDialog": {"rows": [{"column": "Name", "operator": "contains", "text": "forest"}]}}));
    d.click("find.ok");
    assert_eq!(d.app().session.state.selection.len(), 1);
    d.click("find.cancel");

    // Create Search Bin, shown in the Project panel's list view with its live contents
    d.exec("project.view.set", json!({"view": "list"}));
    d.ok("ui.panel.show", json!({"panel": "Project"}));
    d.menu("file.newSearchBin");
    assert!(d.has("searchBin.text") && d.has("searchBin.column"));
    d.ok("ui.set", json!({"menuDialog": {"text": "ocean", "name": "Ocean shots"}}));
    d.click("searchBin.ok");
    let bin = d.app().session.project.search_bins[0].id.0;
    d.frames(3);
    assert!(d.has(&format!("project.searchBin.{bin}")));
    d.click(&format!("project.searchBin.{bin}"));
    let ocean = d.item_named("Ocean_Sunset.mp4");
    assert!(d.app().ui.expanded_bins.contains(&bin));
    let items = d.ok("ui.elements", json!({"prefix": format!("project.item.{ocean}")}));
    assert!(!items.as_array().unwrap().is_empty(), "listed under the expanded search bin: {items}");
}

#[test]
fn project_settings_dialog() {
    let mut d = Driver::new();
    assert_eq!(d.menu("file.projectSettings.scratchDisks")["dialog"], "projectSettings");
    assert_eq!(d.app().ui.extras.dialog.as_ref().unwrap().params["tab"], "scratchDisks");
    for id in ["projectSettings.scratch.captured", "projectSettings.scratch.videoPreviews.browse", "projectSettings.scratch.autoSave.same"] {
        assert!(d.has(id), "{id}");
    }
    d.click("projectSettings.tab.general");
    for id in ["projectSettings.renderer", "projectSettings.videoDisplay", "projectSettings.titleSafe.horizontal", "projectSettings.captureFormat"] {
        assert!(d.has(id), "{id}");
    }
    d.ok(
        "ui.set",
        json!({"menuDialog": {"titleSafe": [15.0, 12.0], "renderer": filmcraft_engine::project_tools::RENDERER_SOFTWARE, "captured": "/tmp/fc-captured"}}),
    );
    d.click("projectSettings.tab.ingest");
    assert!(d.has("projectSettings.ingest.enabled"));
    d.click("projectSettings.ok");
    assert!(!d.dialog_open());
    let st = d.app().session.project.settings.clone();
    assert_eq!(st.title_safe, (15.0, 12.0));
    assert_eq!(st.renderer, filmcraft_engine::project_tools::RENDERER_SOFTWARE);
    assert_eq!(st.scratch.captured.as_deref(), Some("/tmp/fc-captured"));
    assert!(filmcraft_ui_egui::panels::menu_dialogs::software_renderer(d.app()));
}

#[test]
fn clip_and_sequence_dialogs() {
    let mut d = Driver::new();
    // Scene Edit Detection needs a selected video clip
    assert_eq!(d.call("ui.menu.invoke", json!({"id": "clip.sceneEditDetection"}))["ok"], false);
    let c = d.app().session.active_sequence().unwrap().video_tracks[0].items[0].id.0;
    d.exec("timeline.select", json!({"clips": [c]}));
    d.menu("clip.sceneEditDetection");
    for id in ["sceneDetect.applyCuts", "sceneDetect.createSubclips", "sceneDetect.generateMarkers", "sceneDetect.sensitivity", "sceneDetect.ok"] {
        assert!(d.has(id), "{id}");
    }
    d.click("sceneDetect.cancel");
    assert!(!d.dialog_open());
    // Normalize Mix Track
    d.menu("sequence.normalizeMixTrack");
    d.ok("ui.set", json!({"menuDialog": {"db": -6.0}}));
    d.click("normalizeMix.ok");
    assert!(!d.dialog_open());
    assert!(d.app().session.active_sequence().unwrap().master_volume_db != 0.0);
    // Simplify Sequence
    let n = d.app().session.project.items.len();
    d.menu("sequence.simplify");
    assert!(d.has("simplify.keep.audio") && d.has("simplify.closeGaps"));
    d.click("simplify.closeGaps");
    d.click("simplify.ok");
    assert_eq!(d.app().session.project.items.len(), n + 1);
    let active = d.app().session.state.active_sequence.unwrap();
    assert_eq!(d.app().session.project.item(active).unwrap().name, "Main Edit (Simplified)");
    // Automate to Sequence into a new sequence
    d.exec("file.newSequence", json!({"name": "Auto"}));
    let ids: Vec<u64> = ["Ocean_Sunset.mp4", "Misty_Forest.mp4"].iter().map(|n| d.item_named(n)).collect();
    d.exec("project.select", json!({"items": ids}));
    assert_eq!(d.menu("clip.automateToSequence")["dialog"], "automate");
    d.click("automate.method.overwrite");
    d.click("automate.ordering.selection");
    d.click("automate.ok");
    assert!(!d.dialog_open());
    let q = d.app().session.active_sequence().unwrap().clone();
    assert_eq!(q.video_tracks[0].items.iter().map(|i| i.item.0).collect::<Vec<_>>(), ids);
    // Get Media File Properties for ▸ Selection shows the probe info
    d.exec("project.select", json!({"items": [ids[0]]}));
    assert_eq!(d.menu("file.mediaProperties")["dialog"], "properties");
    assert_eq!(d.app().ui.extras.dialog.as_ref().unwrap().info[0]["video"]["width"], 1920);
    d.click("properties.ok");
    assert!(!d.dialog_open());
}

/// #29: File ▸ New ▸ Color Matte… asks for the color (and name, size, duration), and an existing
/// matte's color can be changed afterwards (double-click in the Project panel, or
/// `project.matteColor`), which used to leave every matte stuck at its first, dark grey color.
#[test]
fn color_matte_dialog_picks_and_changes_the_color() {
    let mut d = Driver::new();
    let matte_color = |d: &mut Driver, id: u64| match &d.app().session.project.item(ItemId(id)).unwrap().kind {
        filmcraft_engine::project::ItemKind::Media(m) => match &m.media {
            filmcraft_engine::project::MediaRef::Generator(filmcraft_media::Generator::ColorMatte { color }) => filmcraft_color::to_hex(*color),
            g => panic!("not a matte: {g:?}"),
        },
        _ => panic!("not media"),
    };
    assert_eq!(d.menu("file.newColorMatte")["dialog"], "colorMatte");
    for id in ["colorMatte.color", "colorMatte.name", "colorMatte.width", "colorMatte.height", "colorMatte.seconds", "colorMatte.ok"] {
        assert!(d.has(id), "{id}");
    }
    d.ok("ui.set", json!({"menuDialog": {"color": "#ff8000", "name": "Orange"}}));
    d.click("colorMatte.ok");
    assert!(!d.dialog_open());
    let orange = d.item_named("Orange");
    assert_eq!(matte_color(&mut d, orange), "#ff8000");
    // change it: the dialog starts from the current color
    d.exec("project.select", json!({"items": [orange]}));
    assert_eq!(d.menu("project.matteColor")["dialog"], "matteColor");
    assert_eq!(d.app().ui.extras.dialog.as_ref().unwrap().params["color"], "#ff8000");
    assert!(d.has("matteColor.color"));
    d.ok("ui.set", json!({"menuDialog": {"color": "#2040c0"}}));
    d.click("matteColor.ok");
    assert_eq!(matte_color(&mut d, orange), "#2040c0");
    // without a Color Matte selected there is nothing to change
    let seq = d.app().session.state.active_sequence.unwrap().0;
    d.exec("project.select", json!({"items": [seq]}));
    assert_eq!(d.call("ui.menu.invoke", json!({"id": "project.matteColor"}))["ok"], json!(false));
    assert!(!d.dialog_open());
}

#[test]
fn reveal_in_finder_opens_an_imported_clip() {
    let dir = std::env::temp_dir().join(format!("fc-ui-reveal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("clip.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255])).save(&path).unwrap();
    let mut d = Driver::new();
    let item = d.exec("file.import", json!({"paths": [path.to_string_lossy()]}))["items"][0].as_u64().unwrap();
    assert!(!d.app().session.is_enabled("file.reveal"));
    d.exec("project.select", json!({"items": [item]}));
    assert!(d.app().session.is_enabled("file.reveal"));
    let r = d.ok("ui.menu.invoke", json!({"id": "file.reveal"}));
    assert_eq!(r["paths"][0], path.to_string_lossy().as_ref());
    assert_eq!(d.app().ui.extras.opened.last().map(String::as_str), Some(path.to_string_lossy().as_ref()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A project saved for the first time is listed at the top of Home's Recent list and of File ▸
/// Open Recent, both of which reopen it; Clear Recent Files empties the list (#1461).
#[test]
fn saved_projects_are_listed_in_home_and_open_recent() {
    let dir = std::env::temp_dir().join(format!("fc-ui-recent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("first cut.fcproj").to_string_lossy().to_string();
    let b = dir.join("second cut.fcproj").to_string_lossy().to_string();
    let mut d = Driver::new();
    d.exec("file.saveAs", json!({"path": a}));
    d.exec("file.newProject", json!({"name": "Second"}));
    d.exec("file.saveAs", json!({"path": b}));
    let menus = d.ok("ui.menu.list", json!({}));
    let recent: Vec<&Value> = menus.as_array().unwrap().iter().filter(|m| m["path"] == json!(["File", "Open Recent"])).collect();
    let labels: Vec<&str> = recent.iter().map(|m| m["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["second cut.fcproj", "first cut.fcproj", "Clear Recent Files"]);
    assert_eq!(recent[0]["id"], "file.openRecent.0");
    // File ▸ Open Recent reopens the older one, which moves to the top
    d.menu("file.openRecent.1");
    assert_eq!(d.app().session.path.as_deref(), Some(a.as_str()));
    assert_eq!(d.app().session.prefs.general.recent_projects, [a.clone(), b.clone()]);
    // Home lists them too, newest first, and a click opens one
    d.click("header.home");
    let rows = d.ok("ui.elements", json!({"prefix": "import.recent."}));
    let rows: Vec<&str> = rows.as_array().unwrap().iter().map(|e| e["label"].as_str().unwrap()).collect();
    assert_eq!(rows, ["first cut.fcproj", "second cut.fcproj"]);
    d.click("import.recent.1");
    assert_eq!(d.app().session.path.as_deref(), Some(b.as_str()));
    // Clear Recent Files empties both lists
    d.menu("file.clearRecent");
    assert!(d.app().session.prefs.general.recent_projects.is_empty());
    let menus = d.ok("ui.menu.list", json!({}));
    let clear = menus.as_array().unwrap().iter().find(|m| m["id"] == "file.clearRecent").cloned().expect("Clear Recent Files");
    assert_eq!(clear["enabled"], false);
    assert!(!menus.as_array().unwrap().iter().any(|m| m["id"].as_str().is_some_and(|id| id.starts_with("file.openRecent."))));
    let _ = std::fs::remove_dir_all(&dir);
}

mod responsive_header {
    use super::*;
    use filmcraft_ui_egui::{dock::PanelKind, i18n::Language, menus, state::Mode};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    const RIGHT_CONTROLS: [&str; 9] = [
        "header.discord",
        "header.workspaceName",
        "header.workspaces",
        "header.appearance",
        "header.quickExport",
        "header.notifications",
        "header.search",
        "header.volume",
        "header.fullscreen",
    ];

    // Read this completed frame, not Registry::find/query's previous-frame preference.
    fn rect(d: &Driver, id: &str) -> egui::Rect {
        let mut matches = d.harness.state().auto.elements.iter().filter(|e| e.id == id);
        let element = matches.next().unwrap_or_else(|| panic!("current frame is missing {id}"));
        assert!(matches.next().is_none(), "duplicate current element {id}");
        let [x, y, w, h] = element.rect;
        assert!(element.rect.iter().all(|v| v.is_finite()) && w > 0.0 && h > 0.0, "invalid {id}: {:?}", element.rect);
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
    }

    fn present(d: &Driver, id: &str) -> bool {
        d.harness.state().auto.elements.iter().any(|e| e.id == id)
    }

    fn resize(d: &mut Driver, width: f32, height: f32) {
        d.harness.set_size(egui::vec2(width, height));
        d.frames(4);
        // The existing control readiness check waits for timeline geometry to settle.
        d.ok("ui.elements", json!({"prefix": "header."}));
        d.frames(2);
        let screen = d.harness.ctx.content_rect();
        assert!((screen.width() - width).abs() < 0.5 && (screen.height() - height).abs() < 0.5, "{screen:?}");
    }

    fn body_top(d: &Driver) -> f32 {
        // Every dock group's registered drag grip starts at that group's actual top.
        let top =
            d.harness.state().auto.elements.iter().filter(|e| e.id.starts_with("panel.drag.")).map(|e| rect(d, &e.id).top()).fold(f32::INFINITY, f32::min);
        assert!(top.is_finite(), "fixture has no rendered dock groups");
        top
    }

    fn clear_header(d: &Driver) {
        // Resolve every fixture element before the first overlap assertion.
        let menu_rects: Vec<_> = menus::MENUS.iter().map(|name| (format!("menu.{name}"), rect(d, &format!("menu.{name}")))).collect();
        let controls: Vec<_> = RIGHT_CONTROLS.iter().map(|id| (*id, rect(d, id))).collect();
        for (menu_id, menu_rect) in &menu_rects {
            for (control_id, control_rect) in &controls {
                let overlap = menu_rect.intersect(*control_rect);
                assert!(overlap.width() <= 0.0 || overlap.height() <= 0.0, "header overlap: {menu_id} {menu_rect:?} intersects {control_id} {control_rect:?}");
            }
        }
        let screen = d.harness.ctx.content_rect();
        let body = body_top(d);
        for (index, (id, r)) in menu_rects.iter().enumerate() {
            assert!(
                r.left() >= screen.left() && r.right() <= screen.right() && r.top() >= screen.top() && r.bottom() < body,
                "{id} must fit above the real dock body: {r:?}, screen={screen:?}, body={body}"
            );
            for (other_id, other) in &menu_rects[index + 1..] {
                let overlap = r.intersect(*other);
                assert!(overlap.width() <= 0.0 || overlap.height() <= 0.0, "{id} overlaps {other_id}");
            }
        }
    }

    fn menus_below_controls(d: &Driver) -> bool {
        let control_top = RIGHT_CONTROLS.iter().map(|id| rect(d, id).top()).fold(f32::INFINITY, f32::min);
        let control_bottom = RIGHT_CONTROLS.iter().map(|id| rect(d, id).bottom()).fold(f32::NEG_INFINITY, f32::max);
        let menu_rects: Vec<_> = menus::MENUS.iter().map(|name| rect(d, &format!("menu.{name}"))).collect();
        let inline = menu_rects.iter().all(|r| r.top() < control_bottom && r.bottom() > control_top);
        let below = menu_rects.iter().all(|r| r.top() >= control_bottom);
        assert!(inline != below, "menus must occupy one rendered row: {menu_rects:?}, controls={control_top}..{control_bottom}");
        below
    }

    fn click_current(d: &mut Driver, id: &str) {
        let center = rect(d, id).center();
        // This injects real pointer events through raw_input_hook; it never invokes a menu command.
        d.ok("ui.click", json!({"x": center.x, "y": center.y}));
        d.frames(3);
    }

    fn open_menu(d: &mut Driver, title: &str, item: &str) {
        assert!(!present(d, item), "{item} was already visible before opening {title}");
        click_current(d, &format!("menu.{title}"));
        let item_rect = rect(d, item);
        assert!(item_rect.top() >= rect(d, &format!("menu.{title}")).bottom(), "{title} did not open its popup below the menu");
    }

    #[test]
    fn narrow_header_keeps_menus_clear_and_pointer_reachable_after_resize() {
        let mut d = Driver::new();
        let urls = Arc::new(AtomicUsize::new(0));
        let opened = urls.clone();
        {
            let app = d.app();
            app.integrated_titlebar = false;
            app.ui.show_header = true;
            app.ui.show_menu_bar = true;
            app.ui.mode = Mode::Edit;
            app.ui.workspace = "Editing".into();
            app.ui.language = Language::En;
            app.ui.keys.maximized = None;
            app.ui.floating.clear();
            app.hooks.open_url = Some(Box::new(move |_| {
                opened.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }));
        }
        resize(&mut d, 900.0, 560.0);
        assert!(!d.dialog_open(), "fixture must start without a dialog");

        // Intended old-source failure: actual menu/control overlap, after fixture readiness.
        clear_header(&d);
        assert!(menus_below_controls(&d), "900-point fixture must render menus below the controls");
        let narrow_body = body_top(&d);
        assert!(narrow_body - rect(&d, "header.home").center().y > 20.0, "narrow menus need room above the body");
        let project = d.harness.state().session.project.clone();
        let progress_item = format!("menu.{}", menus::panel_command_id(PanelKind::Progress));
        for (title, item) in [("Graphics and Titles", "menu.graphics.resetAllParameters"), ("View", "menu.view.zoomIn"), ("Window", progress_item.as_str())] {
            open_menu(&mut d, title, item);
            d.ok("ui.key", json!({"key": "Escape"}));
            d.frames(3);
            assert!(!present(&d, item), "{title} popup remained open after Escape");
        }
        open_menu(&mut d, "Help", "menu.help.systemCompatibilityReport");
        click_current(&mut d, "menu.help.systemCompatibilityReport");
        assert!(d.dialog_open(), "pointer selection must open the compatibility report");
        rect(&d, "systemReport.ok");
        click_current(&mut d, "systemReport.ok");
        assert!(!d.dialog_open(), "pointer Close must dismiss the report");
        assert_eq!(d.harness.state().session.project, project, "opening menus/report must not edit the project");
        assert_eq!(urls.load(Ordering::Relaxed), 0, "menu clicks must not hit Discord");

        // Resize the same app back to a width where the original single row fits.
        resize(&mut d, 1600.0, 980.0);
        clear_header(&d);
        assert!(!menus_below_controls(&d), "1600-point fixture must retain the inline menus");
        let wide_body = body_top(&d);
        let home = rect(&d, "header.home");
        assert!((wide_body - home.center().y - 20.0).abs() < 0.5, "wide layout must retain the 38-point header plus one-point body gap");
        for name in menus::MENUS {
            assert!((rect(&d, &format!("menu.{name}")).center().y - home.center().y).abs() < 1.0, "wide {name} left the existing row");
        }

        // Bracket one observed transition using rendered rectangles, not the sizing formula.
        let mut two_row_width = 900.0;
        let mut single_row_width = 1600.0;
        for _ in 0..10 {
            if single_row_width - two_row_width <= 1.0 {
                break;
            }
            let width = (two_row_width + single_row_width) / 2.0;
            resize(&mut d, width, 560.0);
            clear_header(&d);
            if menus_below_controls(&d) {
                two_row_width = width;
            } else {
                single_row_width = width;
            }
        }
        assert!(
            single_row_width > two_row_width && single_row_width - two_row_width <= 1.0,
            "rendered transition bracket is too wide: {two_row_width}..{single_row_width}"
        );
        for (width, below) in [(two_row_width, true), (single_row_width, false)] {
            resize(&mut d, width, 560.0);
            clear_header(&d);
            assert_eq!(menus_below_controls(&d), below, "rendered row changed when revisiting width {width}");
        }
        eprintln!("rendered English header transition: two-row width={two_row_width}, single-row width={single_row_width}");

        // A real resize back must use current geometry, including translated label widths.
        d.app().ui.language = Language::Es;
        resize(&mut d, 900.0, 560.0);
        assert_ne!(Language::Es.tr("Graphics and Titles"), "Graphics and Titles", "translation fixture must use a translated label");
        clear_header(&d);
        assert!(body_top(&d) > wide_body, "translated narrow menu needs its second row");
        open_menu(&mut d, "Help", "menu.help.systemCompatibilityReport");
        d.ok("ui.key", json!({"key": "Escape"}));
        d.frames(3);
        assert!(!present(&d, "menu.help.systemCompatibilityReport"));

        // Model the app configuration used with a native menu; this is not an OS-menu test.
        let right_before: Vec<_> = RIGHT_CONTROLS.iter().map(|id| rect(&d, id)).collect();
        d.app().ui.show_menu_bar = false;
        d.frames(4);
        for name in menus::MENUS {
            assert!(!present(&d, &format!("menu.{name}")));
        }
        assert!(
            (body_top(&d) - rect(&d, "header.home").center().y - 20.0).abs() < 0.5,
            "hidden in-window menu must retain the 38-point header plus one-point body gap"
        );
        for (id, before) in RIGHT_CONTROLS.iter().zip(right_before) {
            assert_eq!(rect(&d, id), before, "hiding the in-window menu moved {id}");
        }
        assert_eq!(d.harness.state().session.project, project);
        assert_eq!(urls.load(Ordering::Relaxed), 0);
    }
}
