# Monitor view options

The View menu and each monitor's wrench menu set how the Source and Program monitors show their
picture. It is all UI state (`MonitorView` in `crates/ui-egui/src/state.rs`, serde): agents read
it with `ui.inspect` (`ui.program`, `ui.source`) and set it with `ui.set {"program": {…}}`, or run
the `view.*` UI commands. Commands act on `params.monitor` (`"program"` / `"source"`), else on the
focused monitor (Program by default). Code: `crates/ui-egui/src/panels/monitor_view.rs`.

| Command | Menu | Effect |
|---|---|---|
| `view.playbackRes.<full\|half\|quarter\|eighth\|sixteenth>` | View ▸ Playback Resolution | render scale while playing |
| `view.pausedRes.<…>` | View ▸ Paused Resolution | render scale while stopped (default Full) |
| `view.highQualityPlayback` | View ▸ High Quality Playback | play at the paused resolution when it is higher |
| `view.display.<composite\|alpha\|red\|green\|blue>` | View ▸ Display Mode | composite, or one channel as greyscale (colour channels premultiplied) |
| `view.display.multicam` | Display Mode ▸ Multi-Camera | the Program's multi-camera view (`multicam.toggleView`) |
| `view.display.audioWaveform`, `view.display.videoAndWaveform` | Display Mode ▸ Audio Waveform / Video and Audio Waveform Split | Source Monitor: the clip's waveform (click to move the source playhead); audio-only clips always show it |
| `view.display.comparison` | Display Mode ▸ Comparison View | Program: the reference frame (left) beside the current frame; the reference starts at the playhead and is stepped or reset with the buttons under it, or `view.compare.setReference {time\|seconds}` |
| `view.magnification.<fit\|10\|25\|50\|75\|100\|150\|200\|400\|800\|1600>` | View ▸ Magnification and the zoom dropdown | 100% = one frame pixel per screen pixel; scroll or drag with the Hand tool to pan |
| `view.showRulers`, `view.showGuides`, `view.lockGuides`, `view.clearGuides` | View | toggles take `{"enabled": bool}` |
| `view.addGuide` | View ▸ Add Guide… | dialog, or `{"orientation": "vertical\|horizontal", "position": px}` |
| `view.snapInProgramMonitor` | View ▸ Snap in Program Monitor | graphic moves snap to the frame edges / centre and the guides (6 pt) |
| `view.safeMargins` | Guide Templates ▸ Safe Margins | action- and title-safe boxes |
| `view.guideTemplates.save`, `.manage`, `.apply`, `.delete` | Guide Templates ▸ Save Guides as Template… / Manage Guides… | templates are stored in the user preferences (`guides.templates`) |

Guides are in frame pixels. Drag out of the top ruler for a horizontal guide or the left ruler for
a vertical one; drag a guide to move it (unless locked) and drop it outside the picture to remove it.

Automation ids: `<monitor>.zoom`, `<monitor>.zoom.<level>`, `<monitor>.settings.<command suffix>`
(wrench menu items, e.g. `program.settings.display.alpha`), `<monitor>.ruler.top|left`,
`<monitor>.guide.<n>`, `program.compare.reference|prev|next|set`, `source.waveform`, and in the
dialogs `guides.add.vertical|horizontal|position|ok|cancel`, `guides.save.name|ok|cancel`,
`guides.manage.row.<n>|apply|delete|close`.

## Program fullscreen preview

View ▸ Program Monitor Full Screen, the Program transport's fullscreen button, or F11 shows the
complete Program picture across the window and requests native fullscreen. Escape, F11, the
corner fullscreen button, or a double-click returns to the editor. Playback keys act on Program
while the preview is open, including when Source had focus. Entering preview stops Source playback;
existing Program playback continues.

The preview fits the composite picture without rulers, guides, editing handles, comparison or
multi-camera controls. The editor's layout, focus, zoom, channel mode and guides are preserved,
and an editor that was already fullscreen stays fullscreen on return. An in-progress dynamic
trim is committed before preview so playback cannot keep changing a hidden edit. With no open sequence,
the command is disabled; closing the active sequence during preview returns to the editor.

Agents invoke `view.programFullScreen` and inspect `ui.program_fullscreen` (null when inactive).
Automation ids: `program.fullscreen`, `program.fullscreen.picture`, `program.fullscreen.exit`.

## Source playback and navigation

The Source monitor’s Play button and Space/J/K/L transport play its open item. Source playback uses
that item’s frame rate and audio, leaves the Program playhead unchanged, and stays inside a restricted
subclip’s bounds. A sequence opened in Source plays its own picture and mix. Audio-only items start
without waiting for video frames. A stalled audio device falls back to the same wall clock used by
Program playback. Source transport does not start mixer automation, multi-camera cuts or voice-over
recording.

While Source has focus, Left/Right, Shift+Left/Right, Home/End and Go to In/Out navigate Source. Frame
steps use Preferences ▸ Playback ▸ Step forward/back many, and stop Source playback before seeking.
Opening another Source item or closing its panel stops the previous Source playback. Opening an item
restores a closed Source panel and leaves a differently maximized panel so the Source is visible.

Agents can use `ui.playback` with `target: "source"`; its default target remains Program. `ui.inspect`
reports the active playback target as `{monitor: "source"|"program", item: id}`. Monitor transport
widgets keep their existing automation ids.
