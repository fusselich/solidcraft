//! Apple Pencil and fingers on a tablet, sorted the way tablet CAD apps do it: once a pencil has
//! touched the screen, the pencil is the mouse (it selects, sketches and drags) and fingers move
//! the camera. One finger orbits, two pan, pinch and twist (`navigation::handle`), a finger's tap
//! still selects and a long press opens the context menu. While the pencil is down, fingers and
//! the palm resting on the glass are ignored.
//!
//! The host turns this on ([`Pencil::enabled`], the iPad app). Before a pencil is used nothing
//! changes, so a finger alone works as before. A pencil is told from a finger by its force: on an
//! iPad only the pencil reports one.
//!
//! It works on the input before egui sees it ([`crate::SolveApp::raw_input_hook`]): egui-winit's
//! mouse emulation of the first touch is dropped and the pointer is emulated here instead, from
//! the pencil, or from a finger's tap.
use egui::{Event, Pos2, TouchId, TouchPhase, Vec2};

/// A finger that moves further than this (points) drags rather than taps.
const SLOP: f32 = 10.0;
/// A finger held still this long (seconds) opens the context menu.
const LONG_PRESS: f64 = 0.5;

#[derive(Default)]
pub struct Pencil {
    /// The host has a stylus: sort touches into pencil and finger.
    pub enabled: bool,
    /// A pencil has touched the screen: from now on it draws and fingers navigate.
    pub active: bool,
    /// The pencil touch in progress.
    pencil: Option<TouchId>,
    /// Fingers egui knows are down: id, device, last position.
    fingers: Vec<(TouchId, egui::TouchDeviceId, Pos2)>,
    /// Fingers ignored until they lift (they were down with the pencil: a palm).
    rejected: Vec<TouchId>,
    /// The one-finger gesture in progress.
    finger: Option<Finger>,
    /// The touch whose emulated pointer events egui-winit sends next, and which of them pass.
    owner: Option<(TouchId, Pass)>,
    /// Events held back for the next frame (a tap's release).
    later: Vec<Event>,
    /// One-finger travel still to orbit by (points) and where the finger landed.
    pub(crate) orbit: Vec2,
    pub(crate) orbit_at: Option<Pos2>,
    /// A long press asked for the context menu here.
    pub(crate) menu: Option<Pos2>,
    /// A finger is held still or a release is held back: the next frame must come soon.
    pub(crate) busy: bool,
    /// The keyboard's modifiers, as last seen.
    modifiers: egui::Modifiers,
}

/// Which of egui-winit's emulated pointer events for a touch reach egui.
#[derive(Clone, Copy, PartialEq)]
enum Pass {
    /// All of them: a finger on a panel (buttons, scrolling lists).
    All,
    /// Only where the pointer is: a finger on the view. The camera gestures need the pointer
    /// over the view (`navigation::handle`), but its press would select or drag.
    Moves,
    /// None: the pencil (its pointer is emulated here) and a rejected palm.
    Nothing,
}

struct Finger {
    id: TouchId,
    start: Pos2,
    last: Pos2,
    at: f64,
    /// It travelled past [`SLOP`]: an orbit, not a tap.
    moved: bool,
    /// A second finger joined: the two-finger gesture, not a tap or an orbit.
    multi: bool,
    /// The long press fired.
    held: bool,
    /// It landed on a panel, not the view: egui's own mouse emulation drives it (buttons,
    /// scrolling lists), with its press passed on.
    on_ui: bool,
}

impl Pencil {
    /// Open the view's context menu here (the pencil's squeeze, at its tip).
    pub fn open_menu(&mut self, at: Pos2) {
        self.menu = Some(at);
    }

    /// Sort this frame's touches. `on_view` says whether a point is over the 3D view (not a panel).
    pub(crate) fn filter(&mut self, raw: &mut egui::RawInput, on_view: impl Fn(Pos2) -> bool) {
        if !self.enabled {
            return;
        }
        let now = raw.time.unwrap_or(0.0);
        // RawInput carries no modifiers of its own: the keyboard's (an iPad's Shift for a
        // multi-select) come with its key events.
        if let Some(m) = raw.events.iter().rev().find_map(|e| match e {
            Event::Key { modifiers, .. } | Event::PointerButton { modifiers, .. } => Some(*modifiers),
            _ => None,
        }) {
            self.modifiers = m;
        }
        let mods = self.modifiers;
        let mut out = std::mem::take(&mut self.later);
        for event in std::mem::take(&mut raw.events) {
            match event {
                Event::Touch { device_id, id, phase, pos, force } => {
                    let pencil = force.is_some() || self.pencil == Some(id);
                    if pencil && !self.active {
                        self.active = true;
                        self.lift_fingers(&mut out);
                    }
                    if !self.active {
                        self.owner = None;
                        self.track_finger(id, device_id, phase, pos);
                        out.push(event);
                    } else if pencil {
                        self.owner = Some((id, Pass::Nothing));
                        self.pencil_touch(id, phase, pos, mods, &mut out);
                    } else {
                        self.finger_touch(event, now, mods, &on_view, &mut out);
                    }
                }
                Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone if self.owner.is_some() => {
                    let pass = match (self.owner.map(|(_, p)| p), &event) {
                        (Some(Pass::All), _) => true,
                        (Some(Pass::Moves), Event::PointerMoved(_)) => true,
                        // A tap's click is still to come (`later`): the pointer stays for it.
                        (Some(Pass::Moves), Event::PointerGone) => self.later.is_empty(),
                        _ => false,
                    };
                    if pass {
                        out.push(event);
                    }
                }
                _ => {
                    self.owner = None;
                    out.push(event);
                }
            }
        }
        // A finger held still opens the context menu where it is.
        if let Some(f) = &mut self.finger {
            if !f.moved && !f.multi && !f.held && !f.on_ui && now - f.at >= LONG_PRESS {
                f.held = true;
                self.menu = Some(f.start);
            }
        }
        raw.events = out;
        self.busy = !self.later.is_empty() || self.finger.as_ref().is_some_and(|f| !f.held && !f.moved);
    }

    /// Before the pencil's first touch fingers drove the pointer: let its press go.
    fn lift_fingers(&mut self, out: &mut Vec<Event>) {
        for (id, device_id, pos) in self.fingers.drain(..) {
            out.push(Event::Touch { device_id, id, phase: TouchPhase::Cancel, pos, force: None });
            out.push(Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            out.push(Event::PointerGone);
            self.rejected.push(id);
        }
    }

    fn track_finger(&mut self, id: TouchId, device_id: egui::TouchDeviceId, phase: TouchPhase, pos: Pos2) {
        self.fingers.retain(|f| f.0 != id);
        if matches!(phase, TouchPhase::Start | TouchPhase::Move) {
            self.fingers.push((id, device_id, pos));
        }
        if matches!(phase, TouchPhase::End | TouchPhase::Cancel) {
            self.rejected.retain(|r| *r != id);
        }
    }

    fn pencil_touch(&mut self, id: TouchId, phase: TouchPhase, pos: Pos2, modifiers: egui::Modifiers, out: &mut Vec<Event>) {
        let primary = egui::PointerButton::Primary;
        match phase {
            TouchPhase::Start => {
                if self.pencil.is_some() {
                    return;
                }
                self.pencil = Some(id);
                // Fingers and the palm already down are let go and ignored until they lift.
                if let Some(f) = self.finger.take() {
                    if f.on_ui {
                        out.push(Event::PointerButton { pos: f.last, button: primary, pressed: false, modifiers });
                        out.push(Event::PointerGone);
                    }
                }
                for (id, device_id, pos) in self.fingers.drain(..) {
                    out.push(Event::Touch { device_id, id, phase: TouchPhase::Cancel, pos, force: None });
                    self.rejected.push(id);
                }
                out.push(Event::PointerMoved(pos));
                out.push(Event::PointerButton { pos, button: primary, pressed: true, modifiers });
            }
            TouchPhase::Move => out.push(Event::PointerMoved(pos)),
            TouchPhase::End | TouchPhase::Cancel => {
                if self.pencil == Some(id) {
                    self.pencil = None;
                    out.push(Event::PointerButton { pos, button: primary, pressed: false, modifiers });
                    out.push(Event::PointerGone);
                }
            }
        }
    }

    fn finger_touch(&mut self, event: Event, now: f64, modifiers: egui::Modifiers, on_view: &impl Fn(Pos2) -> bool, out: &mut Vec<Event>) {
        let Event::Touch { device_id, id, phase, pos, .. } = event else { return };
        if self.rejected.contains(&id) || (self.pencil.is_some() && phase == TouchPhase::Start) {
            if phase == TouchPhase::Start {
                self.rejected.push(id);
            }
            if matches!(phase, TouchPhase::End | TouchPhase::Cancel) {
                self.rejected.retain(|r| *r != id);
            }
            self.owner = Some((id, Pass::Nothing));
            return;
        }
        self.track_finger(id, device_id, phase, pos);
        let mut on_ui = false;
        match phase {
            TouchPhase::Start => {
                if let Some(f) = &mut self.finger {
                    f.multi = true;
                } else if self.fingers.len() == 1 {
                    on_ui = !on_view(pos);
                    self.finger = Some(Finger { id, start: pos, last: pos, at: now, moved: false, multi: false, held: false, on_ui });
                }
            }
            TouchPhase::Move => {
                if let Some(f) = self.finger.as_mut().filter(|f| f.id == id) {
                    on_ui = f.on_ui;
                    if !f.moved && (pos - f.start).length() > SLOP {
                        f.moved = true;
                    }
                    if f.moved && !f.multi && !f.held && !f.on_ui && self.fingers.len() == 1 {
                        self.orbit += pos - f.last;
                        self.orbit_at = Some(f.start);
                    }
                    f.last = pos;
                }
            }
            TouchPhase::End | TouchPhase::Cancel => {
                if let Some(f) = self.finger.take_if(|f| f.id == id) {
                    on_ui = f.on_ui;
                    // A tap on the view selects what it landed on, as a click there.
                    if phase == TouchPhase::End && !f.on_ui && !f.moved && !f.multi && !f.held {
                        let button = egui::PointerButton::Primary;
                        out.push(Event::PointerMoved(f.start));
                        out.push(Event::PointerButton { pos: f.start, button, pressed: true, modifiers });
                        self.later.push(Event::PointerButton { pos: f.start, button, pressed: false, modifiers });
                        self.later.push(Event::PointerGone);
                    }
                }
            }
        }
        self.owner = Some((id, if on_ui { Pass::All } else { Pass::Moves }));
        out.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(id: u64, phase: TouchPhase, x: f32, pencil: bool) -> Event {
        Event::Touch {
            device_id: egui::TouchDeviceId(0),
            id: TouchId::from(id),
            phase,
            pos: egui::pos2(x, 100.0),
            force: pencil.then_some(0.5),
        }
    }

    /// egui-winit's emulation of the first touch's mouse.
    fn press(x: f32, down: bool) -> Event {
        Event::PointerButton { pos: egui::pos2(x, 100.0), button: egui::PointerButton::Primary, pressed: down, modifiers: egui::Modifiers::NONE }
    }

    fn run(p: &mut Pencil, time: f64, events: Vec<Event>) -> Vec<Event> {
        let mut raw = egui::RawInput { time: Some(time), events, ..Default::default() };
        p.filter(&mut raw, |_| true);
        raw.events
    }

    fn presses(events: &[Event]) -> usize {
        events.iter().filter(|e| matches!(e, Event::PointerButton { pressed: true, .. })).count()
    }

    #[test]
    fn without_a_pencil_nothing_changes() {
        let mut p = Pencil { enabled: true, ..Default::default() };
        let events = vec![touch(1, TouchPhase::Start, 10.0, false), Event::PointerMoved(egui::pos2(10.0, 100.0)), press(10.0, true)];
        assert_eq!(run(&mut p, 0.0, events.clone()), events);
        assert!(!p.active);
    }

    #[test]
    fn once_the_pencil_is_used_a_finger_drag_orbits_and_presses_nothing() {
        let mut p = Pencil { enabled: true, ..Default::default() };
        let out = run(&mut p, 0.0, vec![touch(9, TouchPhase::Start, 50.0, true), press(50.0, true)]);
        assert!(p.active);
        assert_eq!(presses(&out), 1, "the pencil presses once (its own, not egui-winit's as well)");
        run(&mut p, 0.1, vec![touch(9, TouchPhase::End, 50.0, true), press(50.0, false)]);
        let out = run(&mut p, 1.0, vec![touch(1, TouchPhase::Start, 10.0, false), press(10.0, true)]);
        assert_eq!(presses(&out), 0, "a finger landing on the view presses nothing");
        run(&mut p, 1.1, vec![touch(1, TouchPhase::Move, 60.0, false)]);
        assert!(p.orbit.x > 0.0, "the finger's travel orbits");
        let out = run(&mut p, 1.2, vec![touch(1, TouchPhase::End, 60.0, false), press(60.0, false)]);
        assert_eq!(presses(&out), 0, "a drag is no tap");
    }

    #[test]
    fn a_finger_tap_clicks_and_a_long_press_opens_the_menu() {
        let mut p = Pencil { enabled: true, active: true, ..Default::default() };
        run(&mut p, 0.0, vec![touch(1, TouchPhase::Start, 10.0, false)]);
        let out = run(&mut p, 0.1, vec![touch(1, TouchPhase::End, 10.0, false)]);
        assert_eq!(presses(&out), 1, "a tap presses");
        let next = run(&mut p, 0.12, Vec::new());
        assert!(next.iter().any(|e| matches!(e, Event::PointerButton { pressed: false, .. })), "and lets go the next frame");
        run(&mut p, 1.0, vec![touch(2, TouchPhase::Start, 10.0, false)]);
        run(&mut p, 1.6, Vec::new());
        assert!(p.menu.is_some(), "holding still opens the menu");
        let out = run(&mut p, 1.7, vec![touch(2, TouchPhase::End, 10.0, false)]);
        assert_eq!(presses(&out), 0, "a long press is no tap");
    }

    #[test]
    fn a_finger_on_the_view_still_moves_the_pointer_so_two_finger_gestures_work() {
        let mut p = Pencil { enabled: true, active: true, ..Default::default() };
        let at = egui::pos2(10.0, 100.0);
        let out = run(&mut p, 0.0, vec![touch(1, TouchPhase::Start, 10.0, false), Event::PointerMoved(at), press(10.0, true)]);
        assert!(out.contains(&Event::PointerMoved(at)), "the pointer follows the finger onto the view");
        assert_eq!(presses(&out), 0, "but presses nothing");
        let out = run(&mut p, 0.1, vec![touch(2, TouchPhase::Start, 90.0, false), touch(1, TouchPhase::Move, 30.0, false), Event::PointerMoved(egui::pos2(30.0, 100.0))]);
        assert_eq!(out.iter().filter(|e| matches!(e, Event::Touch { .. })).count(), 2, "both fingers reach egui's multi-touch");
        assert!(out.contains(&Event::PointerMoved(egui::pos2(30.0, 100.0))));
        assert_eq!(p.orbit, Vec2::ZERO, "two fingers do not orbit");
    }

    #[test]
    fn the_palm_is_ignored_while_the_pencil_is_down() {
        let mut p = Pencil { enabled: true, active: true, ..Default::default() };
        run(&mut p, 0.0, vec![touch(9, TouchPhase::Start, 50.0, true)]);
        let out = run(&mut p, 0.1, vec![touch(1, TouchPhase::Start, 200.0, false), touch(1, TouchPhase::Move, 300.0, false)]);
        assert!(out.iter().all(|e| !matches!(e, Event::Touch { .. })), "the palm's touches never reach egui");
        assert_eq!(p.orbit, Vec2::ZERO, "nor orbit");
        // A palm that landed first is cancelled when the pencil comes down.
        let mut p = Pencil { enabled: true, active: true, ..Default::default() };
        run(&mut p, 0.0, vec![touch(1, TouchPhase::Start, 200.0, false)]);
        let out = run(&mut p, 0.1, vec![touch(9, TouchPhase::Start, 50.0, true)]);
        assert!(out.iter().any(|e| matches!(e, Event::Touch { phase: TouchPhase::Cancel, .. })));
        assert_eq!(presses(&out), 1, "the pencil still draws");
    }
}
