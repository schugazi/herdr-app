//! Windows: overlays a mod opens with `kind: "window"`. They stay until
//! closed, move where they're dropped (a press anywhere the mod doesn't keep
//! for itself drags), and the one pressed last is on top. With a `key` a
//! window remembers its spot, and opening that key again replaces it; with
//! `keep` it also reopens on the next start, in the layout that opened it.
//! A layout switch closes them (they belong to their layout, like its mods).
//! Fork: one opened with `resizable: true` resizes from its left, right and
//! bottom edges and its two bottom corners (the top is its title bar's, for
//! moving), and with a `key` remembers its size too.

use super::*;

/// How far the pointer moves before a press becomes a drag.
const DRAG_START: f32 = 4.0;
/// How much of a window stays on screen however far it's dragged, px.
const KEEP_SHOWN: f32 = 48.0;
/// A resizable window's grab band along its left, right and bottom edges, px.
const GRIP: f32 = 6.0;
/// The smallest a window resizes to, px as designed.
const MIN_SIZE: (f32, f32) = (240.0, 120.0);

/// The side a resize's width follows: its left edge moves the window's left
/// side (its right stays put), its right edge the right side.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Side {
    Left,
    Right,
    Neither,
}

/// A resize: which edges follow the pointer, the window's outer top left and
/// size when pressed, and the app's width then (for KEEP_SHOWN).
#[derive(Clone, Copy)]
struct Resize {
    side: Side,
    bottom: bool,
    origin: Point<Pixels>,
    start: Size<Pixels>,
    view_width: Pixels,
}

pub struct WinDrag {
    pub id: String,
    /// The pointer's offset from the window's top left.
    grab: Point<Pixels>,
    down: Point<Pixels>,
    pointer: Point<Pixels>,
    moving: bool,
    /// A resize instead of a move.
    resize: Option<Resize>,
}

impl WinDrag {
    /// Where a resize puts the window now (top left, size), neither side below
    /// MIN_SIZE (also the one not dragged, so a saved size always passes
    /// `size_of`). From the left the right side stays where it was, and the
    /// left side stops KEEP_SHOWN short of the app's right edge; from the
    /// right, the right side stops KEEP_SHOWN past the app's left edge. So the
    /// keep-on-screen clamp in render_window never moves a window mid-resize.
    fn frame(&self) -> Option<(Point<Pixels>, Size<Pixels>)> {
        let r = self.resize?;
        let grow = self.pointer - self.down;
        let (min_w, keep) = (settings::z(MIN_SIZE.0), px(KEEP_SHOWN));
        let right = r.origin.x + r.start.width;
        let (x, w) = match r.side {
            Side::Left => {
                let x = (r.origin.x + grow.x).min(right - min_w).min(r.view_width - keep);
                (x, right - x)
            }
            Side::Right => (r.origin.x, (r.start.width + grow.x).max(min_w).max(keep - r.origin.x)),
            Side::Neither => (r.origin.x, r.start.width.max(min_w)),
        };
        let h = if r.bottom { r.start.height + grow.y } else { r.start.height }.max(settings::z(MIN_SIZE.1));
        Some((point(x, r.origin.y), size(w, h)))
    }
}

// ---- windows.json: spots and sizes by key, and the kept windows of each layout

fn path() -> PathBuf {
    file("windows.json")
}

fn read() -> Value {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

fn edit(f: impl FnOnce(&mut Value)) {
    let mut v = read();
    f(&mut v);
    if let Ok(s) = serde_json::to_string_pretty(&v) {
        let _ = crate::persist::write_atomic(&path(), s + "\n");
    }
}

pub fn place_of(key: &str) -> Option<Point<Pixels>> {
    let p = read().get("places")?.get(key)?.as_array()?.clone();
    Some(point(px(p.first()?.as_f64()? as f32), px(p.get(1)?.as_f64()? as f32)))
}

fn remember(key: &str, at: Point<Pixels>) {
    edit(|v| v["places"][key] = json!([f32::from(at.x).round(), f32::from(at.y).round()]));
}

/// A resized window's size, px as designed (the zoom applies when drawn).
pub fn size_of(key: &str) -> Option<Size<f32>> {
    let s = read().get("sizes")?.get(key)?.as_array()?.clone();
    let (w, h) = (s.first()?.as_f64()? as f32, s.get(1)?.as_f64()? as f32);
    (w >= MIN_SIZE.0 && h >= MIN_SIZE.1).then(|| size(w, h))
}

fn remember_size(key: &str, s: Size<f32>) {
    edit(|v| v["sizes"][key] = json!([s.width.round(), s.height.round()]));
}

pub fn keep(layout: &str, key: &str, spec: &Value) {
    edit(|v| v["kept"][layout][key] = spec.clone());
}

pub fn forget(layout: &str, key: &str) {
    if read().get("kept").and_then(|k| k.get(layout)).and_then(|l| l.get(key)).is_none() {
        return;
    }
    edit(|v| {
        if let Some(l) = v["kept"].get_mut(layout).and_then(Value::as_object_mut) {
            l.remove(key);
        }
    });
}

fn kept(layout: &str) -> Vec<Value> {
    read().get("kept").and_then(|k| k.get(layout)).and_then(Value::as_object).map(|l| l.values().cloned().collect()).unwrap_or_default()
}

impl AppWindow {
    /// The windows a layout keeps, reopened (at start, or on switching to it).
    pub(super) fn open_kept_windows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for spec in kept(&self.layout_name) {
            self.open_overlay(mods::new_overlay_id(), spec, window, cx);
        }
    }

    /// Leaving a layout: its windows close (the kept ones come back with it).
    pub(super) fn close_windows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The layout's windows go with it; the app's own (What's new) stay.
        let ids: Vec<String> = self.overlays.iter().filter(|o| o.kind == OverlayKind::Window && !o.frame).map(|o| o.id.clone()).collect();
        for id in ids {
            self.remove_overlay(&id, window, cx);
        }
    }

    /// Where a window being resized shows now, top left and size (they replace
    /// its own while the press lasts).
    pub(super) fn resizing(&self, id: &str) -> Option<(Point<Pixels>, Size<Pixels>)> {
        self.win_drag.as_ref().filter(|d| d.id == id && d.moving).and_then(WinDrag::frame)
    }

    /// A window at its spot: where it was dropped, else centered once its size
    /// is known (it stays invisible until then), always partly on screen.
    pub(super) fn render_window(&self, o: &Overlay, body: Stateful<Div>, viewport: Size<Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let measured = self.bounds.borrow().get(&o.slot).map(|b| b.size);
        let live = self.resizing(&o.id);
        // Mid-resize the live size, not last frame's, so the clamp below agrees with `frame()`.
        let size = live.map(|(_, s)| s).or(measured).unwrap_or_default();
        let at = match (&self.win_drag, o.place, live) {
            (_, _, Some((p, _))) => p,
            (Some(d), _, _) if d.id == o.id && d.moving && d.resize.is_none() => d.pointer - d.grab,
            (_, Some(p), _) => p,
            _ => point((viewport.width - size.width) / 2.0, (viewport.height - size.height) / 2.0),
        };
        let keep = px(KEEP_SHOWN);
        let x = at.x.min(viewport.width - keep).max(keep - size.width);
        let y = at.y.min(viewport.height - keep).max(px(0.0));
        let measure = {
            let (bounds, slot, known) = (self.bounds.clone(), o.slot.clone(), measured.is_some());
            canvas(
                move |b, window, _| {
                    if !known {
                        window.request_animation_frame();
                    }
                    bounds.borrow_mut().insert(slot, b);
                },
                |_, _, _, _| {},
            )
            // Anchored: after the body in block flow, an unset inset would put it below.
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        };
        let (raise, press) = (o.id.clone(), o.id.clone());
        // Over the content's edges, drawn after it so they get the press first.
        let grip = |name: &str, side: Side, bottom: bool, cursor: CursorStyle| {
            let id = o.id.clone();
            div()
                .id(SharedString::from(format!("ov-rz-{name}-{}", o.id)))
                .absolute()
                .occlude()
                .cursor(cursor)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.resize_press(&id, ev.position, side, bottom, window.viewport_size().width, cx);
                    }),
                )
        };
        let g = settings::z(GRIP);
        let grips: Vec<AnyElement> = if o.resizable {
            vec![
                grip("l", Side::Left, false, CursorStyle::ResizeLeftRight).top_0().left_0().bottom(g * 2.).w(g).into_any_element(),
                grip("r", Side::Right, false, CursorStyle::ResizeLeftRight).top_0().right_0().bottom(g * 2.).w(g).into_any_element(),
                grip("b", Side::Neither, true, CursorStyle::ResizeUpDown).left(g * 2.).bottom_0().right(g * 2.).h(g).into_any_element(),
                grip("bl", Side::Left, true, CursorStyle::ResizeUpRightDownLeft).left_0().bottom_0().w(g * 2.).h(g * 2.).into_any_element(),
                grip("br", Side::Right, true, CursorStyle::ResizeUpLeftDownRight).right_0().bottom_0().w(g * 2.).h(g * 2.).into_any_element(),
            ]
        } else {
            Vec::new()
        };
        let body = body
            .relative()
            .children(grips)
            // Pressed anywhere, even on the mod's own controls: to the front.
            .capture_any_mouse_down(cx.listener(move |this, _, _, cx| this.raise_window(&raise, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.window_press(&press, ev.position, cx);
                }),
            );
        // Measured on this borderless wrapper, not inside the body's border (which
        // GPUI snaps to device pixels): the spot and size are the outer ones.
        div().absolute().left(x).top(y).when(measured.is_none(), |d| d.opacity(0.0)).child(body).child(measure).into_any_element()
    }

    fn raise_window(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(i) = self.overlays.iter().position(|o| o.id == id) else { return };
        if i + 1 < self.overlays.len() {
            let o = self.overlays.remove(i);
            self.overlays.push(o);
            cx.notify();
        }
    }

    fn window_press(&mut self, id: &str, at: Point<Pixels>, cx: &mut Context<Self>) {
        // A terminal in the window keeps its presses (selecting text).
        let on_terminal = self.overlays.iter().find(|o| o.id == id).and_then(|o| o.layout.as_ref()).is_some_and(|l| {
            let bounds = self.bounds.borrow();
            l.visible_blocks().iter().any(|n| matches!(n.content(), Some(Content::Terminal { .. })) && bounds.get(&n.id).is_some_and(|b| b.contains(&at)))
        });
        if on_terminal {
            return;
        }
        let Some(b) = self.outer_bounds(id) else { return };
        self.win_drag = Some(WinDrag { id: id.to_owned(), grab: at - b.origin, down: at, pointer: at, moving: false, resize: None });
        cx.notify();
    }

    /// Where a window is drawn now, border included.
    fn outer_bounds(&self, id: &str) -> Option<Bounds<Pixels>> {
        let o = self.overlays.iter().find(|o| o.id == id)?;
        self.bounds.borrow().get(&o.slot).copied()
    }

    /// A press on a resizable window's edge: the window is pinned where it is
    /// drawn now (a centered one too), and the pressed edges follow the pointer.
    fn resize_press(&mut self, id: &str, at: Point<Pixels>, side: Side, bottom: bool, view_width: Pixels, cx: &mut Context<Self>) {
        // The grip occludes the body, whose capture handler raises a pressed window.
        self.raise_window(id, cx);
        let Some(b) = self.outer_bounds(id) else { return };
        if let Some(o) = self.overlays.iter_mut().find(|o| o.id == id) {
            o.place = Some(b.origin);
        }
        let resize = Resize { side, bottom, origin: b.origin, start: b.size, view_width };
        self.win_drag = Some(WinDrag { id: id.to_owned(), grab: at - b.origin, down: at, pointer: at, moving: false, resize: Some(resize) });
        cx.notify();
    }

    fn window_move(&mut self, at: Point<Pixels>, pressed: bool, cx: &mut Context<Self>) {
        let Some(d) = &mut self.win_drag else { return };
        if !pressed {
            self.win_drag = None;
            cx.notify();
            return;
        }
        d.pointer = at;
        let dist = f32::from((at.x - d.down.x).abs()).max(f32::from((at.y - d.down.y).abs()));
        if d.moving || dist >= DRAG_START {
            d.moving = true;
            cx.notify();
        }
    }

    /// Ends a press: a drag leaves the window there, a resize at that size
    /// (each remembered under its key); returns true so the release doesn't
    /// also click what's under it.
    fn window_release(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(d) = self.win_drag.take() else { return false };
        cx.notify();
        if !d.moving {
            return false;
        }
        let Some(o) = self.overlays.iter_mut().find(|o| o.id == d.id) else { return true };
        if let Some((p, s)) = d.frame() {
            let zoom = settings::zoom();
            let designed = size(f32::from(s.width) / zoom, f32::from(s.height) / zoom);
            o.size = designed;
            o.fit = false;
            o.place = Some(p);
            if let Some(k) = &o.key {
                remember_size(k, designed);
                remember(k, p);
            }
        } else {
            let at = d.pointer - d.grab;
            o.place = Some(at);
            if let Some(k) = &o.key {
                remember(k, at);
            }
        }
        true
    }

    /// While a window is pressed: follows the pointer anywhere in the app.
    pub(super) fn window_tracker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.win_drag.as_ref()?;
        let this = cx.entity().downgrade();
        Some(
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let t = this.clone();
                    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            let _ = t.update(cx, |this, cx| this.window_move(ev.position, ev.pressed_button == Some(MouseButton::Left), cx));
                        }
                    });
                    let t = this.clone();
                    window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture && ev.button == MouseButton::Left && t.update(cx, |this, cx| this.window_release(cx)).unwrap_or(false) {
                            cx.stop_propagation();
                        }
                    });
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }
}
