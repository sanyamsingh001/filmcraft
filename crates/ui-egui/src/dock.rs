//! Premiere-style docking: a tree of splits and tab groups. Panels are rounded frames separated by
//! thin gutters; each group has a tab strip (active tab bright, with a panel menu "≡"); the focused
//! panel gets a blue outline. Gutters drag to resize; workspaces are serialized trees.

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::icons::{self, Icon};
use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelKind {
    Project,
    MediaBrowser,
    Libraries,
    Info,
    Effects,
    Markers,
    History,
    Source,
    EffectControls,
    AudioClipMixer,
    Metadata,
    Program,
    Timeline,
    Tools,
    AudioMeters,
    AudioTrackMixer,
    LumetriColor,
    LumetriScopes,
    EssentialGraphics,
    EssentialSound,
    Properties,
    Text,
    Events,
    Progress,
    ReferenceMonitor,
    Timecode,
}

impl PanelKind {
    pub const ALL: [PanelKind; 26] = [
        PanelKind::Project,
        PanelKind::MediaBrowser,
        PanelKind::Libraries,
        PanelKind::Info,
        PanelKind::Effects,
        PanelKind::Markers,
        PanelKind::History,
        PanelKind::Source,
        PanelKind::EffectControls,
        PanelKind::AudioClipMixer,
        PanelKind::Metadata,
        PanelKind::Program,
        PanelKind::Timeline,
        PanelKind::Tools,
        PanelKind::AudioMeters,
        PanelKind::AudioTrackMixer,
        PanelKind::LumetriColor,
        PanelKind::LumetriScopes,
        PanelKind::EssentialGraphics,
        PanelKind::EssentialSound,
        PanelKind::Properties,
        PanelKind::Text,
        PanelKind::Events,
        PanelKind::Progress,
        PanelKind::ReferenceMonitor,
        PanelKind::Timecode,
    ];
    pub fn title(self) -> &'static str {
        match self {
            PanelKind::Project => "Project",
            PanelKind::MediaBrowser => "Media Browser",
            PanelKind::Libraries => "Libraries",
            PanelKind::Info => "Info",
            PanelKind::Effects => "Effects",
            PanelKind::Markers => "Markers",
            PanelKind::History => "History",
            PanelKind::Source => "Source",
            PanelKind::EffectControls => "Effect Controls",
            PanelKind::AudioClipMixer => "Audio Clip Mixer",
            PanelKind::Metadata => "Metadata",
            PanelKind::Program => "Program",
            PanelKind::Timeline => "Timeline",
            PanelKind::Tools => "Tools",
            PanelKind::AudioMeters => "Audio Meters",
            PanelKind::AudioTrackMixer => "Audio Track Mixer",
            PanelKind::LumetriColor => "Lumetri Color",
            PanelKind::LumetriScopes => "Lumetri Scopes",
            PanelKind::EssentialGraphics => "Essential Graphics",
            PanelKind::EssentialSound => "Essential Sound",
            PanelKind::Properties => "Properties",
            PanelKind::Text => "Text",
            PanelKind::Events => "Events",
            PanelKind::Progress => "Progress",
            PanelKind::ReferenceMonitor => "Reference Monitor",
            PanelKind::Timecode => "Timecode",
        }
    }
    pub fn id(self) -> String {
        format!("{self:?}")
    }
    pub fn from_name(s: &str) -> Option<PanelKind> {
        let n = s.to_ascii_lowercase().replace([' ', '_', '-'], "");
        Self::ALL.iter().copied().find(|p| format!("{p:?}").to_ascii_lowercase() == n || p.title().to_ascii_lowercase().replace(' ', "") == n)
    }
    /// Window-menu shortcut (Premiere: Shift+1..9).
    pub fn window_shortcut(self) -> Option<&'static str> {
        match self {
            PanelKind::Project => Some("Shift+1"),
            PanelKind::Source => Some("Shift+2"),
            PanelKind::Timeline => Some("Shift+3"),
            PanelKind::Program => Some("Shift+4"),
            PanelKind::EffectControls => Some("Shift+5"),
            PanelKind::AudioClipMixer => Some("Shift+9"),
            PanelKind::Effects => Some("Shift+7"),
            PanelKind::MediaBrowser => Some("Shift+8"),
            PanelKind::AudioTrackMixer => Some("Shift+6"),
            _ => None,
        }
    }
    /// Narrow chrome-less panels (no tab strip; a slim grip instead).
    pub fn compact(self) -> bool {
        matches!(self, PanelKind::Tools | PanelKind::AudioMeters)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SplitSize {
    Ratio(f32),
    /// First child has a fixed size in points.
    FixedA(f32),
    /// Second child has a fixed size in points.
    FixedB(f32),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DockNode {
    /// `vertical`: children stacked top/bottom; else side by side.
    Split {
        vertical: bool,
        size: SplitSize,
        a: Box<DockNode>,
        b: Box<DockNode>,
    },
    Tabs {
        panels: Vec<PanelKind>,
        active: usize,
    },
}

fn tabs(p: &[PanelKind], active: usize) -> DockNode {
    DockNode::Tabs { panels: p.to_vec(), active }
}
fn hsplit(size: SplitSize, a: DockNode, b: DockNode) -> DockNode {
    DockNode::Split { vertical: false, size, a: Box::new(a), b: Box::new(b) }
}
fn vsplit(size: SplitSize, a: DockNode, b: DockNode) -> DockNode {
    DockNode::Split { vertical: true, size, a: Box::new(a), b: Box::new(b) }
}

pub const WORKSPACES: [&str; 9] = ["Editing", "Assembly", "Color", "Effects", "Audio", "Captions and Graphics", "Learning", "Review", "All Panels"];

/// The user's workspaces file, beside `preferences.json` in the data directory.
pub const WORKSPACES_FILE: &str = "workspaces.json";

/// Window ▸ Workspaces: the user's own workspaces and saved changes to the built-in ones, and the
/// workspace in use (reopened at launch).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspacePrefs {
    pub saved: Vec<SavedWorkspace>,
    pub current: String,
}

/// A named layout. `layout` is a [`DockNode`] kept as JSON, so one unreadable entry (a panel a
/// newer version added) doesn't lose the others.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedWorkspace {
    pub name: String,
    pub layout: serde_json::Value,
}

impl WorkspacePrefs {
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, path)
    }
}

/// A workspace's command-id key: `Captions and Graphics` → `captionsandgraphics`.
pub fn slug(name: &str) -> String {
    name.to_ascii_lowercase().replace(' ', "")
}

pub fn is_builtin(name: &str) -> bool {
    WORKSPACES.contains(&name)
}

/// Every workspace, built-in and the user's own, in one alphabetical list (Premiere's Window ▸
/// Workspaces).
pub fn names(prefs: &WorkspacePrefs) -> Vec<String> {
    let mut v: Vec<String> = WORKSPACES.iter().map(|s| s.to_string()).collect();
    for s in &prefs.saved {
        if !v.contains(&s.name) {
            v.push(s.name.clone());
        }
    }
    v.sort_by_key(|n| n.to_lowercase());
    v
}

/// The workspace called `query` (any case, spaces optional).
pub fn find(prefs: &WorkspacePrefs, query: &str) -> Option<String> {
    names(prefs).into_iter().find(|n| slug(n) == slug(query))
}

/// The saved layout of a workspace: the user's saved version, else the built-in default.
pub fn saved_layout(prefs: &WorkspacePrefs, name: &str) -> DockNode {
    prefs.saved.iter().find(|s| s.name == name).and_then(|s| serde_json::from_value(s.layout.clone()).ok()).unwrap_or_else(|| workspace(name))
}

/// The default layout of a named workspace.
pub fn workspace(name: &str) -> DockNode {
    use PanelKind::*;
    use SplitSize::*;
    let bottom_editing = || hsplit(FixedA(64.0), tabs(&[Tools], 0), hsplit(FixedB(116.0), tabs(&[Timeline], 0), tabs(&[AudioMeters], 0)));
    match name {
        "Assembly" => vsplit(
            Ratio(0.52),
            hsplit(
                Ratio(0.46),
                tabs(&[Project, MediaBrowser, Libraries, Info, Effects, Markers, History], 0),
                hsplit(Ratio(0.5), tabs(&[Source, EffectControls, AudioClipMixer, Metadata], 0), tabs(&[Program], 0)),
            ),
            bottom_editing(),
        ),
        "Color" => hsplit(
            FixedB(340.0),
            vsplit(
                Ratio(0.58),
                hsplit(Ratio(0.42), tabs(&[LumetriScopes, Source, EffectControls], 0), tabs(&[Program], 0)),
                hsplit(Ratio(0.3), tabs(&[Project, Info, Markers, History], 0), bottom_editing()),
            ),
            tabs(&[LumetriColor, Effects], 0),
        ),
        "Effects" => hsplit(
            FixedB(320.0),
            vsplit(
                Ratio(0.5),
                hsplit(Ratio(0.5), tabs(&[Source, EffectControls, AudioClipMixer], 1), tabs(&[Program], 0)),
                hsplit(Ratio(0.3), tabs(&[Project, MediaBrowser, Info, Markers, History], 0), bottom_editing()),
            ),
            vsplit(Ratio(0.5), tabs(&[Effects], 0), tabs(&[EffectControls, EssentialGraphics], 1)),
        ),
        "Audio" => hsplit(
            FixedB(320.0),
            vsplit(
                Ratio(0.5),
                hsplit(Ratio(0.5), tabs(&[AudioTrackMixer, Source, AudioClipMixer, EffectControls], 0), tabs(&[Program], 0)),
                hsplit(Ratio(0.3), tabs(&[Project, MediaBrowser, Effects, Markers, History], 0), bottom_editing()),
            ),
            tabs(&[EssentialSound], 0),
        ),
        "Captions and Graphics" => hsplit(
            FixedB(330.0),
            vsplit(
                Ratio(0.5),
                hsplit(Ratio(0.4), tabs(&[Text, Source, EffectControls], 0), tabs(&[Program], 0)),
                hsplit(Ratio(0.3), tabs(&[Project, MediaBrowser, Libraries], 0), bottom_editing()),
            ),
            tabs(&[EssentialGraphics, Properties], 0),
        ),
        "All Panels" => vsplit(
            Ratio(0.5),
            hsplit(
                Ratio(0.5),
                tabs(&[Source, EffectControls, AudioClipMixer, Metadata, LumetriScopes, Text], 0),
                hsplit(Ratio(0.6), tabs(&[Program], 0), tabs(&[LumetriColor, EssentialGraphics, EssentialSound, Properties, AudioTrackMixer], 0)),
            ),
            hsplit(Ratio(0.26), tabs(&[Project, MediaBrowser, Libraries, Info, Effects, Markers, History], 0), bottom_editing()),
        ),
        // Editing (default), Learning, Review — Premiere 26 factory layout:
        // top: Source group | Program | Properties; bottom: Project group | Tools | Timeline | Meters.
        _ => vsplit(
            Ratio(0.57),
            hsplit(
                FixedB(450.0),
                hsplit(Ratio(0.5), tabs(&[Source, EffectControls, AudioClipMixer, Metadata], 0), tabs(&[Program], 0)),
                tabs(&[Properties, EssentialGraphics, Text], 0),
            ),
            hsplit(
                Ratio(0.29),
                tabs(&[Project, MediaBrowser, Libraries, Info, Effects, Markers, History], 0),
                hsplit(FixedA(64.0), tabs(&[Tools], 0), hsplit(FixedB(116.0), tabs(&[Timeline], 0), tabs(&[AudioMeters], 0))),
            ),
        ),
    }
}

impl DockNode {
    /// Every panel in the tree.
    pub fn panels(&self, out: &mut Vec<PanelKind>) {
        match self {
            DockNode::Split { a, b, .. } => {
                a.panels(out);
                b.panels(out);
            }
            DockNode::Tabs { panels, .. } => out.extend(panels.iter().copied()),
        }
    }
    pub fn contains(&self, p: PanelKind) -> bool {
        let mut v = Vec::new();
        self.panels(&mut v);
        v.contains(&p)
    }
    /// Make `p` the active tab of its group. Returns false if not present.
    pub fn activate(&mut self, p: PanelKind) -> bool {
        match self {
            DockNode::Split { a, b, .. } => a.activate(p) || b.activate(p),
            DockNode::Tabs { panels, active } => {
                if let Some(i) = panels.iter().position(|x| *x == p) {
                    *active = i;
                    true
                } else {
                    false
                }
            }
        }
    }
    pub fn is_visible(&self, p: PanelKind) -> bool {
        match self {
            DockNode::Split { a, b, .. } => a.is_visible(p) || b.is_visible(p),
            DockNode::Tabs { panels, active } => panels.get(*active) == Some(&p),
        }
    }
    /// Close a panel (remove its tab; empty groups collapse their split).
    pub fn close(&mut self, p: PanelKind) {
        if let DockNode::Split { a, b, .. } = self {
            a.close(p);
            b.close(p);
            let empty = |n: &DockNode| matches!(n, DockNode::Tabs { panels, .. } if panels.is_empty());
            if empty(a) {
                *self = (**b).clone();
            } else if empty(b) {
                *self = (**a).clone();
            }
        } else if let DockNode::Tabs { panels, active } = self {
            panels.retain(|x| *x != p);
            *active = (*active).min(panels.len().saturating_sub(1));
        }
    }
    /// Activate exactly the panel instance under this tab strip. Global show-panel commands
    /// intentionally keep using `activate`, which finds the first existing instance.
    pub fn activate_at(&mut self, panel: PanelKind, path: &str) -> bool {
        if path.len() > 64 {
            return false;
        }
        let Some(DockNode::Tabs { panels, active }) = self.at_path_mut(path) else { return false };
        let Some(index) = panels.iter().position(|p| *p == panel) else { return false };
        *active = index;
        true
    }

    /// Close exactly one tab instance, preserving the active neighbor and collapsing its group.
    pub fn close_at(&mut self, panel: PanelKind, path: &str) -> bool {
        if path.len() > 64 {
            return false;
        }
        let Some(DockNode::Tabs { panels, active }) = self.at_path_mut(path) else { return false };
        let Some(index) = panels.iter().position(|p| *p == panel) else { return false };
        panels.remove(index);
        *active = if *active > index { active.saturating_sub(1) } else { (*active).min(panels.len().saturating_sub(1)) };
        self.collapse_empty_at(path.as_bytes());
        true
    }

    /// The active instance chosen by the keyboard maximize command. Bound both depth and work
    /// when resolving a saved layout; a stale explicit menu path is never replaced with this.
    fn active_path(&self, panel: PanelKind) -> Option<String> {
        fn find(node: &DockNode, panel: PanelKind, path: &mut String, remaining: &mut usize) -> Option<String> {
            if path.len() > 64 || *remaining == 0 {
                return None;
            }
            *remaining = remaining.saturating_sub(1);
            match node {
                DockNode::Tabs { panels, active } => (panels.get(*active) == Some(&panel)).then(|| path.clone()),
                DockNode::Split { a, b, .. } => {
                    for (child, direction) in [(a, 'a'), (b, 'b')] {
                        path.push(direction);
                        let found = find(child, panel, path, remaining);
                        path.pop();
                        if found.is_some() {
                            return found;
                        }
                    }
                    None
                }
            }
        }
        find(self, panel, &mut String::new(), &mut 1024)
    }

    /// Move an existing panel between tab groups, or reorder it in its group. Paths refer to the
    /// layout before the move; collapsing an empty source group happens after both edits.
    pub fn move_panel(&mut self, panel: PanelKind, from: &str, to: &str, index: usize) -> bool {
        const MAX_PATH: usize = 64;
        if from.len() > MAX_PATH || to.len() > MAX_PATH {
            return false;
        }
        let Some(DockNode::Tabs { panels, .. }) = self.at_path(from) else { return false };
        let Some(source_index) = panels.iter().position(|p| *p == panel) else { return false };
        let Some(DockNode::Tabs { panels: target, .. }) = self.at_path(to) else { return false };
        if from != to && (target.len() >= PanelKind::ALL.len() || target.contains(&panel)) {
            return false;
        }
        let mut insertion = index.min(target.len());
        if from == to {
            if source_index < insertion {
                insertion = insertion.saturating_sub(1);
            }
            let Some(DockNode::Tabs { panels, active }) = self.at_path_mut(from) else { return false };
            let p = panels.remove(source_index);
            panels.insert(insertion, p);
            *active = insertion;
            return true;
        }
        // Both paths were validated, and changing a tab list cannot invalidate the other path.
        if let Some(DockNode::Tabs { panels, active }) = self.at_path_mut(from) {
            panels.remove(source_index);
            *active = if *active > source_index { active.saturating_sub(1) } else { (*active).min(panels.len().saturating_sub(1)) };
        }
        if let Some(DockNode::Tabs { panels, active }) = self.at_path_mut(to) {
            panels.insert(insertion, panel);
            *active = insertion;
        }
        self.collapse_empty_at(from.as_bytes());
        true
    }

    fn at_path(&self, path: &str) -> Option<&DockNode> {
        let mut node = self;
        for direction in path.bytes() {
            node = match (node, direction) {
                (DockNode::Split { a, .. }, b'a') => a,
                (DockNode::Split { b, .. }, b'b') => b,
                _ => return None,
            };
        }
        Some(node)
    }

    fn at_path_mut(&mut self, path: &str) -> Option<&mut DockNode> {
        let mut node = self;
        for direction in path.bytes() {
            node = match (node, direction) {
                (DockNode::Split { a, .. }, b'a') => a,
                (DockNode::Split { b, .. }, b'b') => b,
                _ => return None,
            };
        }
        Some(node)
    }

    // Only the validated source path (at most 64 levels) can acquire an empty leaf.
    fn collapse_empty_at(&mut self, path: &[u8]) {
        let Some((&direction, rest)) = path.split_first() else { return };
        let DockNode::Split { a, b, .. } = self else { return };
        match direction {
            b'a' => a.collapse_empty_at(rest),
            b'b' => b.collapse_empty_at(rest),
            _ => return,
        }
        let empty = |n: &DockNode| matches!(n, DockNode::Tabs { panels, .. } if panels.is_empty());
        if empty(a) {
            *self = std::mem::replace(&mut **b, tabs(&[], 0));
        } else if empty(b) {
            *self = std::mem::replace(&mut **a, tabs(&[], 0));
        }
    }

    /// Put the Timeline panel back where the workspaces keep it (left of the audio meters) after
    /// it was closed, so opening a sequence always has somewhere to show it.
    pub fn restore_timeline(&mut self) {
        if self.contains(PanelKind::Timeline) {
            return;
        }
        fn beside_meters(n: &mut DockNode) -> bool {
            match n {
                DockNode::Split { a, b, .. } => beside_meters(a) || beside_meters(b),
                DockNode::Tabs { panels, .. } if panels.as_slice() == [PanelKind::AudioMeters] => {
                    *n = hsplit(SplitSize::FixedB(116.0), tabs(&[PanelKind::Timeline], 0), tabs(&[PanelKind::AudioMeters], 0));
                    true
                }
                DockNode::Tabs { .. } => false,
            }
        }
        if !beside_meters(self) {
            self.open_near(PanelKind::Timeline, PanelKind::Project);
        }
    }
    /// Add a panel as a tab next to `near` (or into the first group).
    pub fn open_near(&mut self, p: PanelKind, near: PanelKind) {
        if self.contains(p) {
            self.activate(p);
            return;
        }
        fn add(n: &mut DockNode, p: PanelKind, near: PanelKind) -> bool {
            match n {
                DockNode::Split { a, b, .. } => add(a, p, near) || add(b, p, near),
                DockNode::Tabs { panels, active } => {
                    if panels.contains(&near) {
                        panels.push(p);
                        *active = panels.len() - 1;
                        true
                    } else {
                        false
                    }
                }
            }
        }
        if !add(self, p, near) && !add(self, p, PanelKind::Project) {
            add(self, p, PanelKind::Program);
        }
    }
}

/// One laid-out tab group.
pub struct Group {
    pub path: String,
    pub rect: Rect,
    pub content: Rect,
    pub panels: Vec<PanelKind>,
    pub active: usize,
}

/// Actions produced by interacting with the dock chrome.
#[derive(Debug, Clone, PartialEq)]
pub enum DockAction {
    Activate(PanelKind, String),
    Focus(PanelKind),
    Close(PanelKind, String),
    PanelMenu(PanelKind, egui::Pos2, String),
    /// A sequence tab of the Timeline panel was clicked / closed (project item id).
    OpenSequence(u64),
    CloseSequence(u64),
    /// A sequence tab was dragged past a neighbour: its new place among the sequence tabs.
    MoveSequence(u64, usize),
    /// Drop a panel into a tab group at an insertion slot in the original layout.
    MovePanel {
        panel: PanelKind,
        from: String,
        to: String,
        index: usize,
    },
}

/// Size of a split's first child in `avail` points: the requested size, keeping both children at
/// least 20 points when there is room (and never more than `avail`). Must not panic for any size:
/// a tiny window (or browser canvas) gives splits less than 40 points.
fn split_first(size: SplitSize, avail: f32) -> f32 {
    let want = match size {
        SplitSize::Ratio(r) => avail * r,
        SplitSize::FixedA(px) => px,
        SplitSize::FixedB(px) => avail - px,
    };
    let lo = 20.0_f32.min(avail * 0.5);
    let hi = (avail - 20.0).max(lo);
    want.clamp(lo, hi)
}

/// Lay out the tree into group rects (with `gap` gutters) and handle gutter dragging.
pub fn layout(ui: &mut egui::Ui, node: &mut DockNode, rect: Rect, t: &Tokens, path: &str, out: &mut Vec<Group>, reg: &mut crate::automation::Registry) {
    match node {
        DockNode::Tabs { panels, active } => {
            let tab_h = if panels.len() == 1 && panels[0].compact() { 8.0 } else { t.tab_h };
            let content = Rect::from_min_max(pos2(rect.min.x, rect.min.y + tab_h), rect.max);
            out.push(Group { path: path.to_string(), rect, content, panels: panels.clone(), active: *active });
        }
        DockNode::Split { vertical, size, a, b } => {
            let g = t.gap;
            let total = if *vertical { rect.height() } else { rect.width() };
            let avail = (total - g).max(0.0);
            let first = split_first(*size, avail);
            let (ra, gutter, rb) = if *vertical {
                (
                    Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + first)),
                    Rect::from_min_max(pos2(rect.min.x, rect.min.y + first), pos2(rect.max.x, rect.min.y + first + g)),
                    Rect::from_min_max(pos2(rect.min.x, rect.min.y + first + g), rect.max),
                )
            } else {
                (
                    Rect::from_min_max(rect.min, pos2(rect.min.x + first, rect.max.y)),
                    Rect::from_min_max(pos2(rect.min.x + first, rect.min.y), pos2(rect.min.x + first + g, rect.max.y)),
                    Rect::from_min_max(pos2(rect.min.x + first + g, rect.min.y), rect.max),
                )
            };
            // gutter drag (a slightly larger hit area than the visible gap)
            let hit = gutter.expand2(if *vertical { vec2(0.0, 3.0) } else { vec2(3.0, 0.0) });
            let id = egui::Id::new(("dock-gutter", path.to_string()));
            let resp = ui.interact(hit, id, Sense::drag());
            reg.add(&format!("dock.gutter.{path}"), hit, "gutter");
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(if *vertical { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal });
            }
            if resp.dragged() {
                let d = if *vertical { resp.drag_delta().y } else { resp.drag_delta().x };
                let nf = (first + d).clamp(40.0, (avail - 40.0).max(40.0));
                *size = match *size {
                    SplitSize::Ratio(_) => SplitSize::Ratio(nf / avail.max(1.0)),
                    SplitSize::FixedA(_) => SplitSize::FixedA(nf),
                    SplitSize::FixedB(_) => SplitSize::FixedB(avail - nf),
                };
            }
            if resp.dragged() || resp.hovered() {
                ui.painter().rect_filled(gutter, 0.0, t.focus.gamma_multiply(if resp.dragged() { 0.9 } else { 0.4 }));
            }
            layout(ui, a, ra, t, &format!("{path}a"), out, reg);
            layout(ui, b, rb, t, &format!("{path}b"), out, reg);
        }
    }
}

/// Draw a group's frame + tab strip. Returns actions (tab clicks, panel menu, focus).
/// The sequences open in the Timeline panel, in tab order: `(project item id, name)`.
#[derive(Debug, Clone, Default)]
pub struct SeqTabs {
    pub open: Vec<(u64, String)>,
    pub active: Option<u64>,
}

/// The temporary full-size frame has no tree path. Keep the menu's original path while
/// maximized; keyboard maximize resolves its already active instance once. A stale stored path
/// stays stale, so a close can fail safely instead of removing every copy of the panel.
pub fn panel_action_path(ctx: &egui::Context, dock: &DockNode, panel: PanelKind, path: String, maximized: bool) -> String {
    if !maximized {
        return path;
    }
    if let Some((p, original)) = ctx.data(|d| d.get_temp::<(PanelKind, String)>(egui::Id::new("dock-maximized-group")))
        && p == panel
    {
        return original;
    }
    dock.active_path(panel).unwrap_or_else(|| "invalid".into())
}

pub fn remember_maximized_group(ctx: &egui::Context, panel: PanelKind, path: String) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("dock-maximized-group"), (panel, path)));
}

pub fn forget_maximized_group(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<(PanelKind, String)>(egui::Id::new("dock-maximized-group")));
}

#[derive(Clone)]
struct PanelDrag {
    panel: PanelKind,
    from: String,
    last_frame: u64,
}

fn panel_drag(ctx: &egui::Context) -> Option<PanelDrag> {
    if ctx.input(|i| !i.focused || i.key_pressed(egui::Key::Escape) || (!i.pointer.primary_down() && !i.pointer.button_released(egui::PointerButton::Primary)))
    {
        return None;
    }
    let id = egui::Id::new("dock-panel-drag");
    let mut drag = ctx.data(|d| d.get_temp::<PanelDrag>(id))?;
    let frame = ctx.cumulative_frame_nr();
    // Fullscreen monitors and other modes hide the dock. A drag must not survive a frame in
    // which its chrome did not run, including a release delivered while the dock was hidden.
    if frame.saturating_sub(drag.last_frame) > 1 {
        cancel_panel_drag(ctx);
        return None;
    }
    drag.last_frame = frame;
    ctx.data_mut(|d| d.insert_temp(id, drag.clone()));
    Some(drag)
}

pub fn cancel_panel_drag(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<PanelDrag>(egui::Id::new("dock-panel-drag")));
}

/// Run once after every group's chrome: an earlier group must not consume another group's drop.
pub fn finish_panel_drag(ctx: &egui::Context) {
    if ctx.input(|i| !i.pointer.primary_down() || !i.focused || i.key_pressed(egui::Key::Escape)) {
        cancel_panel_drag(ctx);
    }
}

fn panel_drop(ui: &egui::Ui, g: &Group, strip: Rect, rects: &[(usize, Rect)], t: &Tokens, actions: &mut Vec<DockAction>) {
    let Some(drag) = panel_drag(ui.ctx()) else { return };
    if drag.from != g.path && g.panels.contains(&drag.panel) {
        return;
    }
    let Some(pointer) = ui.input(|i| i.pointer.latest_pos()).filter(|p| strip.contains(*p)) else { return };
    let before = rects.iter().find(|(_, r)| pointer.x < r.center().x);
    let (index, x) = match before {
        Some((index, r)) => (*index, r.min.x - 4.0),
        None => rects.last().map_or((g.panels.len(), strip.min.x + 12.0), |(index, r)| (index.saturating_add(1), r.max.x + 4.0)),
    };
    let x = x.clamp(strip.min.x, strip.max.x);
    ui.painter().line_segment([pos2(x, strip.min.y + 4.0), pos2(x, strip.max.y - 4.0)], Stroke::new(2.0, t.focus));
    if ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
        actions.push(DockAction::MovePanel { panel: drag.panel, from: drag.from, to: g.path.clone(), index });
    }
}

#[allow(clippy::too_many_arguments)] // The dock passes its existing frame, sequence and automation state explicitly.
pub fn draw_group_chrome(
    ui: &mut egui::Ui,
    g: &Group,
    focused: PanelKind,
    panel_moves: bool,
    seqs: &SeqTabs,
    t: &Tokens,
    reg: &mut crate::automation::Registry,
) -> Vec<DockAction> {
    let mut actions = Vec::new();
    let painter = ui.painter().clone();
    painter.rect_filled(g.rect, t.radius, t.panel_bg);
    let active_panel = g.panels.get(g.active).copied();
    let compact = g.panels.len() == 1 && g.panels[0].compact();
    if compact {
        // grip dots
        let c = pos2(g.rect.center().x, g.rect.min.y + 4.0);
        for dx in [-4.0, 0.0, 4.0] {
            painter.circle_filled(c + vec2(dx, 0.0), 1.0, t.text_faint);
        }
        let strip = Rect::from_min_size(g.rect.min, vec2(g.rect.width(), 8.0));
        if panel_moves {
            panel_drop(ui, g, strip, &[], t, &mut actions);
        }
    } else {
        let strip = Rect::from_min_size(g.rect.min, vec2(g.rect.width(), t.tab_h));
        // the Timeline panel shows one tab per open sequence (its own tab names the sequence)
        struct Tab {
            panel: PanelKind,
            panel_index: usize,
            seq: Option<u64>,
            title: String,
            active: bool,
        }
        let mut tabs: Vec<Tab> = Vec::new();
        for (i, p) in g.panels.iter().enumerate() {
            let active = i == g.active;
            if *p == PanelKind::Timeline && active && !seqs.open.is_empty() {
                tabs.extend(seqs.open.iter().map(|(id, name)| Tab {
                    panel: *p,
                    panel_index: i,
                    seq: Some(*id),
                    title: name.clone(),
                    active: seqs.active == Some(*id),
                }));
            } else if *p == PanelKind::Timeline {
                let name = seqs.open.iter().find(|(id, _)| seqs.active == Some(*id)).map(|(_, n)| n.clone());
                tabs.push(Tab { panel: *p, panel_index: i, seq: None, title: name.unwrap_or_else(|| "Timeline: (no sequences)".into()), active });
            } else {
                tabs.push(Tab { panel: *p, panel_index: i, seq: None, title: p.title().to_string(), active });
            }
        }
        let activate = |tab: &Tab, actions: &mut Vec<DockAction>| match tab.seq {
            Some(id) => actions.push(DockAction::OpenSequence(id)),
            None => actions.push(DockAction::Activate(tab.panel, g.path.clone())),
        };
        let active_at = tabs.iter().position(|tab| tab.active).unwrap_or(0);
        let text_y = strip.min.y + 16.0;
        // Which tabs fit. The active tab is always shown: tabs before it are left out until it
        // fits, then as many after it as there is room for. Tabs that are left out are reached
        // through the » list at the right end of the strip.
        let widths: Vec<f32> = tabs
            .iter()
            .map(|tab| {
                let label = painter.layout_no_wrap(tab.title.clone(), Tokens::ui(12.0), t.tab_text).size().x;
                label + 16.0 + if tab.active { 20.0 } else { 0.0 } + if tab.active && tab.seq.is_some() { 16.0 } else { 0.0 }
            })
            .collect();
        let span = |from: usize, to: usize| widths.get(from..=to).map_or(0.0, |w| w.iter().sum::<f32>() + 8.0 * w.len() as f32);
        let room = strip.width() - 12.0;
        let all_fit = span(0, tabs.len().saturating_sub(1)) <= room;
        let room = if all_fit { room } else { room - 24.0 };
        let mut first = 0;
        while first < active_at && span(first, active_at) > room {
            first += 1;
        }
        let mut last = active_at.max(first);
        while last + 1 < tabs.len() && span(first, last + 1) <= room {
            last += 1;
        }
        let list_id = egui::Id::new(("tab-list", g.path.clone()));
        let more = (!all_fit).then(|| Rect::from_min_size(pos2(strip.max.x - 22.0, strip.min.y + 6.0), vec2(18.0, 20.0)));
        let mut list_opened = false;
        if let Some(r) = more {
            let resp = ui.interact(r, egui::Id::new(("tab-overflow", g.path.clone())), Sense::click());
            let name = if g.panels.contains(&PanelKind::Timeline) { "timeline.tabs.more".to_string() } else { format!("panel.tabs.more.{}", g.path) };
            reg.add(&name, r, "more tabs");
            let c = if resp.hovered() { t.tab_text_active } else { t.tab_text };
            icons::paint(&painter, r.shrink(4.0).translate(vec2(-2.0, 0.0)), Icon::ChevronRight, c);
            icons::paint(&painter, r.shrink(4.0).translate(vec2(2.0, 0.0)), Icon::ChevronRight, c);
            if resp.clicked() {
                let open = ui.ctx().data(|d| d.get_temp::<bool>(list_id)).unwrap_or(false);
                ui.ctx().data_mut(|d| d.insert_temp(list_id, !open));
                list_opened = !open;
            }
        }
        let mut x = strip.min.x + 12.0;
        // sequence tabs as drawn (for dragging one past its neighbours), and the one being dragged
        let mut seq_rects: Vec<(u64, Rect)> = Vec::new();
        let mut panel_rects: Vec<(usize, Rect)> = Vec::new();
        let mut dragged: Option<u64> = None;
        for (i, tab) in tabs.iter().enumerate().take(last + 1).skip(first) {
            let p = &tab.panel;
            let is_active = tab.active;
            let galley = painter.layout_no_wrap(tab.title.clone(), Tokens::ui(12.0), if is_active { t.tab_text_active } else { t.tab_text });
            let close_w = if is_active && tab.seq.is_some() { 16.0 } else { 0.0 };
            let w = widths.get(i).copied().unwrap_or(0.0);
            let tab_rect = Rect::from_min_size(pos2(x, strip.min.y), vec2(w, t.tab_h));
            // a sequence tab keeps its identity when the tabs are reordered under the pointer
            let resp = match tab.seq {
                Some(id) => ui.interact(tab_rect, egui::Id::new(("tab-seq", g.path.clone(), id)), Sense::click_and_drag()),
                None => ui.interact(tab_rect, egui::Id::new(("tab", g.path.clone(), *p)), Sense::click_and_drag()),
            };
            panel_rects.push((tab.panel_index, tab_rect));
            if panel_moves && tab.seq.is_none() && resp.drag_started_by(egui::PointerButton::Primary) {
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new("dock-panel-drag"), PanelDrag { panel: *p, from: g.path.clone(), last_frame: ui.ctx().cumulative_frame_nr() })
                });
            }
            if let Some(id) = tab.seq {
                seq_rects.push((id, tab_rect));
                if resp.dragged() {
                    dragged = Some(id);
                }
            }
            if let Some(id) = tab.seq {
                reg.add(&format!("timeline.tab.{id}"), tab_rect, &tab.title);
            }
            if tab.seq.is_none() || is_active {
                reg.add(&format!("panel.tab.{}", p.id()), tab_rect, p.title());
                reg.add(&format!("panel.tab.{}.group.{}", p.id(), g.path), tab_rect, p.title());
            }
            let label_x = tab_rect.min.x + 8.0 + close_w;
            let label_w = galley.size().x;
            let col = if is_active || resp.hovered() { t.tab_text_active } else { t.tab_text };
            painter.galley_with_override_text_color(pos2(label_x, text_y - galley.size().y / 2.0), galley, col);
            let mut closed = false;
            if let Some(id) = tab.seq.filter(|_| is_active) {
                // × closes the sequence's tab (the sequence stays in the project)
                let cr = Rect::from_center_size(pos2(tab_rect.min.x + 10.0, text_y), vec2(8.0, 8.0));
                let cresp = ui.interact(cr.expand(4.0), egui::Id::new(("tab-close", g.path.clone(), id)), Sense::click());
                reg.add(&format!("timeline.tab.{id}.close"), cr.expand(4.0), "close sequence");
                let cc = if cresp.hovered() { t.tab_text_active } else { t.tab_text };
                painter.line_segment([cr.left_top(), cr.right_bottom()], Stroke::new(1.2, cc));
                painter.line_segment([cr.right_top(), cr.left_bottom()], Stroke::new(1.2, cc));
                if cresp.clicked() {
                    actions.push(DockAction::CloseSequence(id));
                    closed = true;
                }
            }
            if is_active {
                let mr = Rect::from_center_size(pos2(label_x + label_w + 12.0, text_y), vec2(12.0, 10.0));
                let mresp = ui.interact(mr.expand(3.0), egui::Id::new(("tab-menu", g.path.clone())), Sense::click());
                reg.add(&format!("panel.menu.{}", p.id()), mr, "panel menu");
                reg.add(&format!("panel.menu.{}.group.{}", p.id(), g.path), mr, "panel menu");
                let mc = if mresp.hovered() { t.tab_text_active } else { t.tab_text };
                icons::paint(&painter, Rect::from_center_size(mr.center(), vec2(16.0, 16.0)), Icon::Hamburger, mc);
                // 1 pt underline spanning label + ≡, 23 pt below the frame top
                let uy = strip.min.y + 23.0;
                painter.line_segment([pos2(label_x, uy), pos2(mr.max.x, uy)], Stroke::new(1.0, t.tab_text_active));
                if mresp.clicked() {
                    actions.push(DockAction::PanelMenu(*p, mr.left_bottom(), g.path.clone()));
                }
            }
            if (resp.clicked() || (tab.seq.is_some() && resp.drag_started())) && !closed {
                activate(tab, &mut actions);
                actions.push(DockAction::Focus(*p));
            }
            // a right-click shows the tab and opens its panel menu (as in Premiere)
            if resp.secondary_clicked() {
                activate(tab, &mut actions);
                actions.push(DockAction::Focus(*p));
                let at = resp.interact_pointer_pos().unwrap_or(tab_rect.left_bottom());
                actions.push(DockAction::PanelMenu(*p, at, g.path.clone()));
            }
            if resp.middle_clicked() {
                // a sequence tab closes its sequence; the Timeline panel itself stays
                match tab.seq.or(seqs.active.filter(|_| *p == PanelKind::Timeline)) {
                    Some(id) => actions.push(DockAction::CloseSequence(id)),
                    None => actions.push(DockAction::Close(*p, g.path.clone())),
                }
            }
            x += w + 8.0;
        }
        if panel_moves {
            panel_drop(ui, g, strip, &panel_rects, t, &mut actions);
        }
        // Dragging a sequence tab sideways: it changes places with a neighbour when the pointer
        // passes that neighbour's middle.
        if let Some(id) = dragged
            && let Some(px) = ui.input(|i| i.pointer.latest_pos()).map(|p| p.x)
            && let Some(at) = seq_rects.iter().position(|(s, _)| *s == id)
            && let Some(index) = seqs.open.iter().position(|(s, _)| *s == id)
        {
            let before = at.checked_sub(1).and_then(|k| seq_rects.get(k));
            let after = seq_rects.get(at + 1);
            if before.is_some_and(|(_, r)| px < r.center().x) {
                actions.push(DockAction::MoveSequence(id, index.saturating_sub(1)));
            } else if after.is_some_and(|(_, r)| px > r.center().x) {
                actions.push(DockAction::MoveSequence(id, index + 1));
            }
        }
        // the » list: every tab of the group, the shown one ticked
        let list_open = ui.ctx().data(|d| d.get_temp::<bool>(list_id)).unwrap_or(false);
        if let Some(r) = more.filter(|_| list_open) {
            let mut picked = None;
            let area = egui::Area::new(list_id.with("area")).order(egui::Order::Foreground).pivot(Align2::RIGHT_TOP).fixed_pos(r.right_bottom()).show(
                ui.ctx(),
                |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(160.0);
                        // many open sequences: the list scrolls instead of leaving the window
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for tab in &tabs {
                                let label = if tab.active { format!("✓ {}", tab.title) } else { tab.title.clone() };
                                let resp = ui.button(label);
                                match tab.seq {
                                    Some(id) => reg.add(&format!("timeline.tabs.list.{id}"), resp.rect, &tab.title),
                                    None => reg.add(&format!("panel.tabs.list.{}", tab.panel.id()), resp.rect, &tab.title),
                                }
                                if resp.clicked() {
                                    picked = Some(tab);
                                }
                            }
                        });
                    });
                },
            );
            if let Some(tab) = picked {
                activate(tab, &mut actions);
                actions.push(DockAction::Focus(tab.panel));
            }
            if picked.is_some() || (!list_opened && area.response.clicked_elsewhere()) {
                ui.ctx().data_mut(|d| d.remove::<bool>(list_id));
            }
        } else if list_open {
            ui.ctx().data_mut(|d| d.remove::<bool>(list_id));
        }
    }
    // focus outline
    if active_panel == Some(focused) {
        painter.rect_stroke(g.rect, 0.0, Stroke::new(1.0, t.focus), StrokeKind::Inside);
    }
    // clicking anywhere in the panel focuses it
    if let Some(p) = active_panel
        && ui.rect_contains_pointer(g.rect)
        && ui.input(|i| i.pointer.any_pressed())
    {
        actions.push(DockAction::Focus(p));
    }
    actions
}

/// Placeholder body for panels that are not implemented yet.
pub fn placeholder(ui: &mut egui::Ui, rect: Rect, t: &Tokens, text: &str) {
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, Tokens::ui(12.0), t.text_faint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_first_never_panics_on_tiny_areas() {
        for avail in [0.0, 1.0, 15.0, 20.0, 35.0, 39.9, 40.0, 100.0, 2000.0] {
            for size in [
                SplitSize::Ratio(0.0),
                SplitSize::Ratio(0.5),
                SplitSize::Ratio(1.0),
                SplitSize::FixedA(300.0),
                SplitSize::FixedB(300.0),
                SplitSize::FixedA(0.0),
            ] {
                let f = split_first(size, avail);
                assert!((0.0..=avail).contains(&f), "{size:?} in {avail}: {f}");
            }
        }
        assert_eq!(split_first(SplitSize::FixedA(300.0), 1000.0), 300.0);
        assert_eq!(split_first(SplitSize::FixedB(300.0), 1000.0), 700.0);
        assert_eq!(split_first(SplitSize::Ratio(0.0), 1000.0), 20.0);
    }

    #[test]
    fn move_panel_reorders_preserves_source_active_and_collapses_empty_groups() {
        use PanelKind::*;
        let mut d = hsplit(SplitSize::Ratio(0.37), tabs(&[Project, Effects, Info], 2), tabs(&[Program], 0));
        assert!(d.move_panel(Effects, "a", "b", usize::MAX));
        assert_eq!(d.at_path("a"), Some(&tabs(&[Project, Info], 1)));
        assert_eq!(d.at_path("b"), Some(&tabs(&[Program, Effects], 1)));
        assert!(d.move_panel(Effects, "b", "b", 0));
        assert_eq!(d.at_path("b"), Some(&tabs(&[Effects, Program], 0)));
        assert!(d.move_panel(Effects, "b", "b", usize::MAX));
        assert_eq!(d.at_path("b"), Some(&tabs(&[Program, Effects], 1)));
        assert!(d.move_panel(Project, "a", "b", 0));
        assert!(d.move_panel(Info, "a", "b", 1));
        assert_eq!(d, tabs(&[Project, Info, Program, Effects], 1));
        assert_eq!(serde_json::from_str::<DockNode>(&serde_json::to_string(&d).unwrap()).unwrap(), d);
    }

    #[test]
    fn scoped_activation_and_close_keep_other_panel_copies_and_reject_stale_paths() {
        use PanelKind::*;
        let mut d = hsplit(SplitSize::Ratio(0.37), tabs(&[EffectControls, Source], 1), tabs(&[EffectControls, EssentialGraphics], 1));
        assert!(d.activate_at(EffectControls, "b"));
        assert_eq!(d.at_path("a"), Some(&tabs(&[EffectControls, Source], 1)));
        assert_eq!(d.at_path("b"), Some(&tabs(&[EffectControls, EssentialGraphics], 0)));
        assert!(d.close_at(EffectControls, "b"));
        assert_eq!(d, hsplit(SplitSize::Ratio(0.37), tabs(&[EffectControls, Source], 1), tabs(&[EssentialGraphics], 0)));
        let original = d.clone();
        assert!(!d.close_at(EffectControls, "b"));
        assert!(!d.close_at(EffectControls, &"a".repeat(65)));
        assert!(!d.activate_at(EffectControls, "x"));
        assert_eq!(d, original);
        assert!(d.close_at(EffectControls, "a"));
        assert_eq!(d.at_path("a"), Some(&tabs(&[Source], 0)));
    }

    #[test]
    fn maximized_menu_path_survives_hidden_frames_and_stays_stale_after_a_move() {
        use PanelKind::*;
        let ctx = egui::Context::default();
        let mut d = hsplit(SplitSize::Ratio(0.37), tabs(&[EffectControls, Source], 1), tabs(&[EffectControls, EssentialGraphics], 0));
        remember_maximized_group(&ctx, EffectControls, "b".into());
        assert_eq!(panel_action_path(&ctx, &d, EffectControls, String::new(), true), "b");
        // A fullscreen round trip does not draw dock chrome, so its original path remains.
        assert_eq!(panel_action_path(&ctx, &d, EffectControls, String::new(), true), "b");
        assert!(!d.move_panel(EffectControls, "b", "a", 0), "do not create a duplicate within a group");
        d = tabs(&[EffectControls, Program], 0);
        let path = panel_action_path(&ctx, &d, EffectControls, String::new(), true);
        assert_eq!(path, "b");
        assert!(!d.close_at(EffectControls, &path));
        assert_eq!(d, tabs(&[EffectControls, Program], 0));
        forget_maximized_group(&ctx);
        assert_eq!(panel_action_path(&ctx, &d, EffectControls, String::new(), true), "");
    }

    #[test]
    fn move_panel_validates_paths_and_keeps_duplicate_panel_instances_distinct() {
        use PanelKind::*;
        let mut d = hsplit(
            SplitSize::Ratio(0.4),
            tabs(&[EffectControls, Source], 1),
            hsplit(SplitSize::FixedB(300.0), tabs(&[EffectControls], 0), tabs(&[Program], 0)),
        );
        let original = d.clone();
        for (panel, from, to) in [(Effects, "a", "bb"), (Source, "x", "bb"), (Source, "a", "b"), (EffectControls, "a", "ba")] {
            assert!(!d.move_panel(panel, from, to, usize::MAX));
            assert_eq!(d, original);
        }
        assert!(!d.move_panel(Source, &"a".repeat(65), "bb", 0));
        assert_eq!(d, original);
        assert!(d.move_panel(EffectControls, "ba", "bb", 0));
        assert_eq!(d.at_path("a"), Some(&tabs(&[EffectControls, Source], 1)));
        assert_eq!(d.at_path("b"), Some(&tabs(&[EffectControls, Program], 0)));
        let mut full = hsplit(SplitSize::Ratio(0.5), tabs(&[Source], 0), tabs(&[Program; 26], 0));
        let original = full.clone();
        assert!(!full.move_panel(Source, "a", "b", 0));
        assert_eq!(full, original);
    }

    #[test]
    fn workspaces_contain_core_panels() {
        for w in WORKSPACES {
            let d = workspace(w);
            assert!(d.contains(PanelKind::Timeline), "{w}");
            assert!(d.contains(PanelKind::Program), "{w}");
        }
    }

    #[test]
    fn close_and_open() {
        let mut d = workspace("Editing");
        d.close(PanelKind::Metadata);
        assert!(!d.contains(PanelKind::Metadata));
        d.open_near(PanelKind::LumetriColor, PanelKind::Program);
        assert!(d.is_visible(PanelKind::LumetriColor));
        let s = serde_json::to_string(&d).unwrap();
        let back: DockNode = serde_json::from_str(&s).unwrap();
        assert_eq!(back, d);
    }
}
