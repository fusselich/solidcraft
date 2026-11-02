//! The number pad: an on-screen keypad for number and expression fields, as tablet CAD apps
//! have one. iPadOS offers no number keyboard of its own (an iPad shows the full keyboard
//! whatever layout an app asks for), so while a number field has the focus the system keyboard
//! is held back (`keyboard::set_numpad`) and this pad is drawn instead. ABC brings the system
//! keyboard back for that field, for names in an expression (`d1 * 2`).
//!
//! The pad is painted, not built from egui widgets: a widget taking the press would take the
//! focus from the field being typed in. Presses on its keys are taken out of the input
//! ([`Numpad::take_presses`]) and become the text and keys they stand for.

/// What a key does.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Action {
    Text(&'static str),
    Key(egui::Key),
    /// Hand the field to the system keyboard.
    Abc,
}

/// The keys, row by row: label, action, width in key units.
const ROWS: &[&[(&str, Action, f32)]] = &[
    &[("7", Action::Text("7"), 1.0), ("8", Action::Text("8"), 1.0), ("9", Action::Text("9"), 1.0), ("(", Action::Text("("), 1.0), (")", Action::Text(")"), 1.0)],
    &[("4", Action::Text("4"), 1.0), ("5", Action::Text("5"), 1.0), ("6", Action::Text("6"), 1.0), ("×", Action::Text("*"), 1.0), ("÷", Action::Text("/"), 1.0)],
    &[("1", Action::Text("1"), 1.0), ("2", Action::Text("2"), 1.0), ("3", Action::Text("3"), 1.0), ("+", Action::Text("+"), 1.0), ("−", Action::Text("-"), 1.0)],
    &[("0", Action::Text("0"), 1.0), (".", Action::Text("."), 1.0), ("mm", Action::Text(" mm"), 1.0), ("in", Action::Text(" in"), 1.0), ("°", Action::Text("°"), 1.0)],
    &[("ABC", Action::Abc, 1.0), ("", Action::Key(egui::Key::Backspace), 1.0), ("", Action::Key(egui::Key::Enter), 3.0)],
];
/// A key's height and the gap between keys (points).
const KEY: f32 = 52.0;
const GAP: f32 = 6.0;
const KEY_WIDTH: f32 = 64.0;
/// How long (seconds) a change must hold before the pad and keyboard follow it. While a
/// keyboard slides in the layout moves, and a field may miss its number mark for a frame or
/// two; following every frame made the keyboard flicker in and out.
const HOLD: f64 = 0.3;

#[derive(Default)]
pub struct Numpad {
    /// Where each key was drawn last frame (empty while the pad is hidden).
    keys: Vec<(egui::Rect, Action)>,
    /// The pad's outline last frame.
    rect: Option<egui::Rect>,
    /// A press on a key is held until its release.
    held: Option<Action>,
    /// The field ABC handed to the system keyboard: no pad while it keeps the focus.
    abc: Option<egui::Id>,
    /// A press of ABC this frame (acted on in `show`).
    want_abc: bool,
    /// When a number field last had the focus, and when the ABC field last had it (app time).
    numeric_at: f64,
    abc_at: f64,
}

impl Numpad {
    /// Whether the pad stands in for the keyboard now.
    #[cfg(test)]
    pub fn shown(&self) -> bool {
        self.rect.is_some()
    }

    /// Turn presses on the pad's keys into the text and keys they stand for. Run on the input
    /// after the app's own sorting of touch (so pencil and finger presses both arrive as the
    /// pointer's), before egui sees it.
    pub fn take_presses(&mut self, raw: &mut egui::RawInput) {
        if self.rect.is_none() && self.held.is_none() {
            return;
        }
        let key_at = |p: egui::Pos2| self.keys.iter().find(|(r, _)| r.expand(GAP / 2.0).contains(p)).map(|(_, a)| *a);
        let on_pad = |p: egui::Pos2| self.rect.is_some_and(|r| r.contains(p));
        let mut out = Vec::with_capacity(raw.events.len());
        for event in std::mem::take(&mut raw.events) {
            match &event {
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } if on_pad(*pos) => {
                    self.held = key_at(*pos);
                    if self.held.is_none() {
                        // Between keys: swallowed all the same, so the field keeps the focus.
                        self.held = Some(Action::Text(""));
                    }
                }
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers } if self.held.is_some() => {
                    let pressed = self.held.take();
                    if pressed.is_some() && key_at(*pos) == pressed {
                        match pressed {
                            Some(Action::Text(t)) if !t.is_empty() => out.push(egui::Event::Text(t.to_string())),
                            Some(Action::Key(key)) => {
                                for down in [true, false] {
                                    out.push(egui::Event::Key { key, physical_key: Some(key), pressed: down, repeat: false, modifiers: *modifiers });
                                }
                            }
                            Some(Action::Abc) => self.want_abc = true,
                            _ => {}
                        }
                    }
                }
                egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } if self.held.is_some() => {}
                egui::Event::PointerGone if self.held.is_some() => {
                    self.held = None;
                }
                _ => out.push(event),
            }
        }
        raw.events = out;
    }

    /// Draw the pad while a number field (`numeric`) has the focus, inside `area` and clear of
    /// the field. Returns whether the system keyboard should stay away, and whether a number
    /// field has the focus (both steadied over [`HOLD`]).
    pub fn show(&mut self, ctx: &egui::Context, area: egui::Rect, numeric: bool) -> (bool, bool) {
        let now = ctx.input(|i| i.time);
        let focused = ctx.memory(|m| m.focused());
        if numeric && focused.is_some() {
            self.numeric_at = now;
        }
        // A field that has closed (Enter, Escape, a tap elsewhere) ends the pad at once; one that
        // only missed its mark keeps it.
        let numeric = focused.is_some() && (numeric || now - self.numeric_at < HOLD);
        if self.want_abc {
            self.want_abc = false;
            self.abc = focused;
            self.abc_at = now;
        }
        if self.abc.is_some() {
            if self.abc == focused {
                self.abc_at = now;
            } else if now - self.abc_at >= HOLD {
                self.abc = None;
            }
        }
        if !numeric || self.abc.is_some() {
            self.keys.clear();
            self.rect = None;
            return (false, numeric);
        }
        let size = egui::vec2(5.0 * KEY_WIDTH + 6.0 * GAP, ROWS.len() as f32 * KEY + (ROWS.len() as f32 + 1.0) * GAP);
        // Bottom right, or bottom left when the field being typed in would be under it.
        let field = focused.and_then(|id| ctx.read_response(id)).map(|r| r.rect);
        let right = egui::Rect::from_min_size(area.right_bottom() - size - egui::vec2(12.0, 12.0), size);
        let left = egui::Rect::from_min_size(egui::pos2(area.left() + 12.0, area.bottom() - size.y - 12.0), size);
        let pad = if field.is_some_and(|f| f.intersects(right)) { left } else { right };
        let visuals = ctx.global_style().visuals.clone();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("sc_numpad")));
        painter.rect(pad, 12.0, visuals.window_fill, visuals.window_stroke, egui::StrokeKind::Inside);
        self.keys.clear();
        let mut y = pad.top() + GAP;
        for row in ROWS {
            let mut x = pad.left() + GAP;
            for (label, action, units) in *row {
                let w = units * KEY_WIDTH + (units - 1.0) * GAP;
                let r = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, KEY));
                let held = self.held == Some(*action);
                let fill = match action {
                    Action::Key(egui::Key::Enter) => visuals.selection.bg_fill,
                    _ if held => visuals.widgets.active.bg_fill,
                    _ => visuals.widgets.inactive.weak_bg_fill,
                };
                painter.rect_filled(r, 8.0, fill);
                let ink = if matches!(action, Action::Key(egui::Key::Enter)) { visuals.selection.stroke.color } else { visuals.text_color() };
                match action {
                    // Drawn, not typeset: the app's fonts have no ⌫ or ✓.
                    Action::Key(egui::Key::Backspace) => backspace(&painter, r.center(), ink),
                    Action::Key(egui::Key::Enter) => check(&painter, r.center(), ink),
                    _ => {
                        painter.text(r.center(), egui::Align2::CENTER_CENTER, *label, egui::FontId::proportional(22.0), ink);
                    }
                }
                self.keys.push((r, *action));
                x += w + GAP;
            }
            y += KEY + GAP;
        }
        self.rect = Some(pad);
        (true, true)
    }
}

/// The delete key's symbol: a tag pointing left with a cross in it.
fn backspace(painter: &egui::Painter, c: egui::Pos2, ink: egui::Color32) {
    let stroke = egui::Stroke::new(2.0, ink);
    let (w, h) = (26.0, 18.0);
    let left = c.x - w / 2.0;
    let points = vec![
        egui::pos2(left, c.y),
        egui::pos2(left + 8.0, c.y - h / 2.0),
        egui::pos2(left + w, c.y - h / 2.0),
        egui::pos2(left + w, c.y + h / 2.0),
        egui::pos2(left + 8.0, c.y + h / 2.0),
    ];
    painter.add(egui::Shape::closed_line(points, stroke));
    let x = egui::pos2(c.x + 4.0, c.y);
    let d = 4.5;
    painter.line_segment([x + egui::vec2(-d, -d), x + egui::vec2(d, d)], stroke);
    painter.line_segment([x + egui::vec2(-d, d), x + egui::vec2(d, -d)], stroke);
}

/// The confirm key's symbol: a tick.
fn check(painter: &egui::Painter, c: egui::Pos2, ink: egui::Color32) {
    let points = vec![c + egui::vec2(-11.0, 0.0), c + egui::vec2(-3.0, 8.0), c + egui::vec2(12.0, -9.0)];
    painter.add(egui::Shape::line(points, egui::Stroke::new(3.0, ink)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(at: egui::Pos2, down: bool) -> egui::Event {
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: down, modifiers: egui::Modifiers::NONE }
    }

    fn pad() -> (Numpad, egui::Context) {
        let ctx = egui::Context::default();
        let mut pad = Numpad::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.memory_mut(|m| m.request_focus(egui::Id::new("field")));
        });
        out.textures_delta.clear();
        let mut out = ctx.run_ui(egui::RawInput { time: Some(0.0), ..Default::default() }, |ui| {
            ui.memory_mut(|m| m.request_focus(egui::Id::new("field")));
            pad.show(ui.ctx(), egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0)), true);
        });
        out.textures_delta.clear();
        (pad, ctx)
    }

    fn key(pad: &Numpad, label_action: Action) -> egui::Pos2 {
        pad.keys.iter().find(|(_, a)| *a == label_action).map(|(r, _)| r.center()).unwrap_or_default()
    }

    #[test]
    fn a_tap_on_a_key_types_it_and_reaches_nothing_else() {
        let (mut p, _ctx) = pad();
        assert!(p.shown());
        let at = key(&p, Action::Text("7"));
        let mut raw = egui::RawInput { events: vec![egui::Event::PointerMoved(at), press(at, true)], ..Default::default() };
        p.take_presses(&mut raw);
        assert!(raw.events.iter().all(|e| !matches!(e, egui::Event::PointerButton { .. })), "the press never reaches the field");
        let mut raw = egui::RawInput { events: vec![press(at, false), egui::Event::PointerGone], ..Default::default() };
        p.take_presses(&mut raw);
        // The finger leaving afterwards may pass: it presses nothing.
        assert_eq!(raw.events.first(), Some(&egui::Event::Text("7".into())));
        assert!(raw.events.iter().all(|e| !matches!(e, egui::Event::PointerButton { .. })));
    }

    #[test]
    fn enter_and_backspace_are_keys() {
        let (mut p, _ctx) = pad();
        let at = key(&p, Action::Key(egui::Key::Enter));
        let mut raw = egui::RawInput { events: vec![press(at, true), press(at, false)], ..Default::default() };
        p.take_presses(&mut raw);
        assert!(raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Enter, pressed: true, .. })));
    }

    #[test]
    fn a_press_off_the_pad_passes_and_no_number_field_hides_it() {
        let (mut p, ctx) = pad();
        let off = egui::pos2(5.0, 5.0);
        let mut raw = egui::RawInput { events: vec![press(off, true)], ..Default::default() };
        p.take_presses(&mut raw);
        assert_eq!(raw.events.len(), 1);
        // Held a moment through a missed mark, then gone.
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        let field = egui::Id::new("field");
        let mut out = ctx.run_ui(egui::RawInput { time: Some(0.1), ..Default::default() }, |ui| {
            ui.memory_mut(|m| m.request_focus(field));
            assert!(p.show(ui.ctx(), area, false).0, "one frame without the mark keeps the pad");
        });
        out.textures_delta.clear();
        let mut out = ctx.run_ui(egui::RawInput { time: Some(1.0), ..Default::default() }, |ui| {
            ui.memory_mut(|m| m.request_focus(field));
            assert!(!p.show(ui.ctx(), area, false).0, "a field that stays without the mark loses the pad");
        });
        out.textures_delta.clear();
        assert!(!p.shown());
    }
}
