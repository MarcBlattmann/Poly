//! Input routing (ADR 0107): the seat's pointer and keyboard, from
//! [`InputEvent`]s to `wl_pointer` and `wl_keyboard` events, with the
//! window management the frame does (focus on click, the close button,
//! moving a window by its title bar).
//!
//! As wayland.xml describes the seat:
//!
//! - **Pointer focus** is the surface under the pointer: `wl_pointer.enter`
//!   (surface-local position) when it changes, `leave` for the old one,
//!   `motion` while it stays, each sequence ended by `frame` (version 5).
//!   While a button is held the focus stays where the press was (an
//!   implicit grab), so a drag out of a window still reaches it. Buttons
//!   go to the focus with a serial; the wheel as `axis_source` (wheel),
//!   `axis_discrete` and `axis`, then `frame`.
//! - **Keyboard focus** follows the active toplevel: `wl_keyboard.leave`
//!   and `enter` (with the keys down) and `modifiers` when it moves; `key`
//!   events with a serial; `modifiers` whenever they change. A new
//!   `wl_keyboard` gets the keymap first (`xkb_v1`, a file descriptor the
//!   [`Host`] provides) and `repeat_info`. The modifier bits are those of
//!   `poly_input::xkb`, whose keymaps the compositor hands out.
//! - **Window management**: a left press on a window's close button asks
//!   it to close; on its title bar or border it focuses and raises the
//!   window and starts moving it until the button is released
//!   (`xdg_toplevel.move` starts the same); on its content it focuses it
//!   and goes to the client as well.
//! - The **cursor** is drawn by the compositor (the scene's software
//!   cursor) wherever the pointer is; `wl_pointer.set_cursor` surfaces are
//!   not shown yet.

use super::*;
use crate::compose::TITLE_HEIGHT;
use crate::protocol::{wl_keyboard, wl_pointer};
use crate::seat::BTN_LEFT;
use poly_input::xkb;

/// Repeat rate (keys per second) and delay (ms) clients are told: Poly's
/// event layer repeats after 250 ms every 33 ms (ADR 0103).
pub const REPEAT_RATE: i32 = 30;
pub const REPEAT_DELAY: i32 = 250;

/// Where the pointer is: a client's surface shown in a scene window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Focus {
    pub client: ClientId,
    pub surface: ObjectId,
    pub window: WindowId,
}

/// What a held button does instead of reaching a client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grab {
    /// Moving `window`; its content's position is the pointer's plus
    /// (`dx`, `dy`).
    Move { window: WindowId, dx: i32, dy: i32 },
}

/// The seat's input state.
#[derive(Debug, Default)]
pub(crate) struct InputState {
    pub pointer_focus: Option<Focus>,
    pub buttons: Vec<u32>,
    pub grab: Option<Grab>,
    pub keys: Vec<u32>,
    /// Modifier bits held, and locked (Caps Lock, Num Lock).
    pub depressed: u32,
    pub locked: u32,
    pub keyboard_focus: Option<(ClientId, ObjectId)>,
    /// The modifiers the focused client last heard.
    pub sent_mods: Option<(u32, u32)>,
    /// The compositor took the keyboard (its own launcher is open).
    pub keyboard_suspended: bool,
    /// An internal window a button was pressed on (it gets the motion and
    /// the release), and the one under the pointer (ADR 0117).
    pub internal_grab: Option<WindowId>,
    pub internal_hover: Option<WindowId>,
    /// The last left press on a title bar and its time: a second within
    /// [`DOUBLE_CLICK_MS`] maximizes or restores.
    pub last_title_press: Option<(WindowId, u32)>,
    /// A move of a maximized or tiled window started here: it is restored
    /// once the pointer is [`UNSNAP_DISTANCE`] away.
    pub unsnap_at: Option<(i32, i32)>,
}

/// Two presses on a title bar this close (ms) are a double click.
pub const DOUBLE_CLICK_MS: u32 = 400;
/// How far (logical pixels) a maximized or tiled window is dragged before
/// it comes off its place.
pub const UNSNAP_DISTANCE: i32 = 8;

impl Server {
    /// An input event: window management, then the clients.
    pub fn handle_input(&mut self, event: InputEvent) {
        let (ow, oh) = self.scene.logical_size();
        let clamp = |v: Fixed, max: i32| Fixed(v.0.clamp(0, (max - 1).max(0) * 256));
        match event {
            InputEvent::PointerMotion { time, dx, dy } => {
                let (x, y) = self.seat.pointer;
                self.seat.pointer = (clamp(Fixed(x.0 + dx.0), ow), clamp(Fixed(y.0 + dy.0), oh));
                self.pointer_moved(time);
            }
            InputEvent::PointerMotionAbsolute { time, x, y } => {
                self.seat.pointer = (clamp(x, ow), clamp(y, oh));
                self.pointer_moved(time);
            }
            InputEvent::PointerButton { time, button, pressed } => self.pointer_button(time, button, pressed),
            InputEvent::PointerAxis { time, horizontal, value } => self.pointer_axis(time, horizontal, value),
            InputEvent::Key { time, key, pressed } => self.key(time, key, pressed),
        }
        self.sync_keyboard_focus();
    }

    /// Show or hide the software cursor (at the pointer).
    pub fn set_cursor_visible(&mut self, visible: bool) {
        let (x, y) = self.seat.pointer;
        self.scene.set_cursor(visible.then(|| (x.floor(), y.floor())));
    }

    /// Take the keyboard from the clients (`true`: the focused one hears
    /// `leave`) or give it back.
    pub fn set_keyboard_suspended(&mut self, suspended: bool) {
        self.seat.input.keyboard_suspended = suspended;
        self.sync_keyboard_focus();
    }

    /// The modifier bits held and locked (`poly_input::xkb::mods`).
    pub fn modifiers(&self) -> (u32, u32) {
        (self.seat.input.depressed, self.seat.input.locked)
    }

    /// The client and surface with keyboard focus.
    pub fn keyboard_focus(&self) -> Option<(ClientId, ObjectId)> {
        self.seat.input.keyboard_focus
    }

    /// The client and surface under the pointer (with pointer focus).
    pub fn pointer_focus(&self) -> Option<(ClientId, ObjectId)> {
        self.seat.input.pointer_focus.map(|f| (f.client, f.surface))
    }

    /// Whether a window is being moved.
    pub fn is_moving(&self) -> bool {
        self.seat.input.grab.is_some()
    }

    /// The objects of `interface` client `c` has.
    fn devices(&mut self, c: ClientId, interface: &str) -> Vec<ObjectId> {
        let Some(client) = self.clients.get(&c) else { return Vec::new() };
        client.objects.iter().filter(|(_, o)| o.interface.name == interface).map(|(id, _)| ObjectId(*id)).collect()
    }

    /// Whether client `c` still has object `id`.
    fn exists(&self, c: ClientId, id: ObjectId) -> bool {
        self.clients.get(&c).is_some_and(|cl| cl.get(id).is_some())
    }

    /// The client surface in window `w`.
    fn focus_of(&self, w: WindowId) -> Option<Focus> {
        let (client, surface) = owner_of(self.scene.window(w)?.owner());
        self.exists(client, surface).then_some(Focus { client, surface, window: w })
    }

    /// The pointer's position relative to window `w`'s surface.
    fn local(&self, w: WindowId) -> (Fixed, Fixed) {
        let (wx, wy) = self.scene.window(w).map_or((0, 0), |w| w.position());
        let (x, y) = self.seat.pointer;
        (Fixed(x.0 - wx * 256), Fixed(y.0 - wy * 256))
    }

    fn pointer_moved(&mut self, time: u32) {
        let (x, y) = (self.seat.pointer.0.floor(), self.seat.pointer.1.floor());
        if self.scene.cursor().is_some() {
            self.scene.set_cursor(Some((x, y)));
        }
        if let Some(Grab::Move { window, mut dx, dy }) = self.seat.input.grab {
            if let Some((px, py)) = self.seat.input.unsnap_at
                && (x - px).abs().max((y - py).abs()) >= UNSNAP_DISTANCE
            {
                // Dragged off its place: floating again, under the pointer.
                self.seat.input.unsnap_at = None;
                if let Some((nx, _)) = self.unsnap_for_drag(window, px) {
                    dx = nx - px;
                    self.seat.input.grab = Some(Grab::Move { window, dx, dy });
                }
            }
            if self.seat.input.unsnap_at.is_none() {
                self.move_window(window, x + dx, y + dy);
            }
            return;
        }
        if let Some(w) = self.seat.input.internal_grab {
            let (wx, wy) = self.scene.window(w).map_or((0, 0), |win| win.position());
            self.internal_events.push(InternalEvent::Motion { window: w, x: x - wx, y: y - wy });
            return;
        }
        let hit = self.scene.hit_test(x, y);
        // Internal windows hear the pointer as events (ADR 0117).
        let over_internal = match hit {
            Hit::Surface { window, x: lx, y: ly } if self.internal.contains_key(&window) => Some((window, lx, ly)),
            _ => None,
        };
        let hover = over_internal.map(|(w, ..)| w);
        if let Some(old) = self.seat.input.internal_hover
            && hover != Some(old)
        {
            self.internal_events.push(InternalEvent::Leave(old));
        }
        self.seat.input.internal_hover = hover;
        if let Some((window, lx, ly)) = over_internal {
            self.internal_events.push(InternalEvent::Motion { window, x: lx, y: ly });
        }
        let target = if self.seat.input.buttons.is_empty() {
            match hit {
                Hit::Surface { window, .. } => self.focus_of(window),
                _ => None,
            }
        } else {
            self.seat.input.pointer_focus.filter(|f| self.scene.window(f.window).is_some())
        };
        if target != self.seat.input.pointer_focus {
            self.set_pointer_focus(target);
            return;
        }
        if let Some(f) = target {
            let (sx, sy) = self.local(f.window);
            for p in self.devices(f.client, wl_pointer::NAME) {
                let client = self.client(f.client);
                client.send(p, wl_pointer::Event::Motion { time, surface_x: sx, surface_y: sy });
                client.send(p, wl_pointer::Event::Frame);
            }
        }
    }

    /// Move the pointer focus: `leave` for the old surface, `enter` for the
    /// new one.
    fn set_pointer_focus(&mut self, target: Option<Focus>) {
        if let Some(old) = self.seat.input.pointer_focus.take()
            && self.exists(old.client, old.surface)
        {
            let serial = self.next_serial();
            for p in self.devices(old.client, wl_pointer::NAME) {
                let client = self.client(old.client);
                client.send(p, wl_pointer::Event::Leave { serial, surface: old.surface });
                client.send(p, wl_pointer::Event::Frame);
            }
        }
        self.seat.input.pointer_focus = target;
        if let Some(f) = target {
            let serial = self.next_serial();
            let (sx, sy) = self.local(f.window);
            for p in self.devices(f.client, wl_pointer::NAME) {
                let client = self.client(f.client);
                client.send(p, wl_pointer::Event::Enter { serial, surface: f.surface, surface_x: sx, surface_y: sy });
                client.send(p, wl_pointer::Event::Frame);
            }
        }
    }

    fn pointer_button(&mut self, time: u32, button: u32, pressed: bool) {
        let buttons = &mut self.seat.input.buttons;
        if pressed && !buttons.contains(&button) {
            buttons.push(button);
        } else if !pressed {
            buttons.retain(|&b| b != button);
        }
        if self.seat.input.grab.is_some() {
            if self.seat.input.buttons.is_empty() {
                self.seat.input.grab = None;
                self.seat.input.unsnap_at = None;
                self.pointer_moved(time);
            }
            return;
        }
        let (x, y) = (self.seat.pointer.0.floor(), self.seat.pointer.1.floor());
        if let Some(w) = self.seat.input.internal_grab {
            // The press was on an internal window: so is the rest.
            let (wx, wy) = self.scene.window(w).map_or((0, 0), |win| win.position());
            self.internal_events.push(InternalEvent::Button { window: w, x: x - wx, y: y - wy, button, pressed });
            if self.seat.input.buttons.is_empty() {
                self.seat.input.internal_grab = None;
                self.pointer_moved(time);
            }
            return;
        }
        if pressed {
            match self.scene.hit_test(x, y) {
                Hit::Close(w) if button == BTN_LEFT => {
                    self.close_window(w);
                    return;
                }
                Hit::Maximize(w) if button == BTN_LEFT => {
                    self.focus_window(w);
                    self.toggle_maximized(w);
                    return;
                }
                Hit::Minimize(w) if button == BTN_LEFT => {
                    self.minimize_window(w);
                    return;
                }
                Hit::TitleBar(w) if button == BTN_LEFT => {
                    self.focus_window(w);
                    let double = self
                        .seat
                        .input
                        .last_title_press
                        .take()
                        .is_some_and(|(pw, pt)| pw == w && time.wrapping_sub(pt) <= DOUBLE_CLICK_MS);
                    if double {
                        self.toggle_maximized(w);
                    } else {
                        self.seat.input.last_title_press = Some((w, time));
                        self.start_move(w);
                    }
                    return;
                }
                Hit::Close(w) | Hit::Maximize(w) | Hit::Minimize(w) | Hit::TitleBar(w) | Hit::Border(w) => {
                    self.focus_window(w);
                    if button == BTN_LEFT {
                        self.start_move(w);
                    }
                    return;
                }
                Hit::Surface { window, x: lx, y: ly } if self.internal.contains_key(&window) => {
                    self.focus_window(window);
                    self.set_pointer_focus(None);
                    self.seat.input.internal_grab = Some(window);
                    self.internal_events.push(InternalEvent::Button { window, x: lx, y: ly, button, pressed });
                    return;
                }
                Hit::Surface { window, .. } => {
                    self.focus_window(window);
                    // A press on another surface than the focus (it
                    // appeared under a still pointer) moves the focus first.
                    let target = self.focus_of(window);
                    if target.map(|f| f.surface) != self.seat.input.pointer_focus.map(|f| f.surface) {
                        self.set_pointer_focus(target);
                    }
                }
                Hit::Desktop => {}
            }
        }
        if let Some(f) = self.seat.input.pointer_focus {
            let serial = self.next_serial();
            let state = if pressed { wl_pointer::ButtonState::PRESSED } else { wl_pointer::ButtonState::RELEASED };
            for p in self.devices(f.client, wl_pointer::NAME) {
                let client = self.client(f.client);
                client.send(p, wl_pointer::Event::Button { serial, time, button, state: state.0 });
                client.send(p, wl_pointer::Event::Frame);
            }
        }
        if !pressed && self.seat.input.buttons.is_empty() {
            // The implicit grab ends: the focus is what is under the pointer.
            self.pointer_moved(time);
        }
    }

    fn pointer_axis(&mut self, time: u32, horizontal: bool, value: Fixed) {
        if let Some(window) = self.seat.input.internal_hover {
            self.internal_events.push(InternalEvent::Axis { window, horizontal, value: value.0 / 256 });
            return;
        }
        let Some(f) = self.seat.input.pointer_focus else { return };
        let axis = if horizontal { wl_pointer::Axis::HORIZONTAL_SCROLL } else { wl_pointer::Axis::VERTICAL_SCROLL };
        // One wheel step is 15 logical pixels (the evdev reader's unit).
        let steps = value.0 / (15 * 256);
        for p in self.devices(f.client, wl_pointer::NAME) {
            let client = self.client(f.client);
            client.send(p, wl_pointer::Event::AxisSource { axis_source: wl_pointer::AxisSource::WHEEL.0 });
            if steps != 0 {
                client.send(p, wl_pointer::Event::AxisDiscrete { axis: axis.0, discrete: steps });
            }
            client.send(p, wl_pointer::Event::Axis { time, axis: axis.0, value });
            client.send(p, wl_pointer::Event::Frame);
        }
    }

    /// Start moving window `w` with the pointer (while a button is held).
    pub(super) fn start_move(&mut self, w: WindowId) {
        if self.seat.input.buttons.is_empty() {
            return;
        }
        let Some(win) = self.scene.window(w) else { return };
        let (wx, wy) = win.position();
        let (x, y) = (self.seat.pointer.0.floor(), self.seat.pointer.1.floor());
        let snapped = win.is_maximized();
        // The client no longer gets the pointer while it moves.
        self.set_pointer_focus(None);
        self.seat.input.grab = Some(Grab::Move { window: w, dx: wx - x, dy: wy - y });
        // A maximized or tiled window stays until the pointer really moves.
        self.seat.input.unsnap_at = snapped.then_some((x, y));
    }

    /// Put window `w`'s content at (`x`, `y`), keeping its title bar inside
    /// the work area (below a shell's strip, and reachable).
    fn move_window(&mut self, w: WindowId, x: i32, mut y: i32) {
        let area = self.scene.work_area();
        let Some(win) = self.scene.window(w) else { return };
        let (wx, wy) = win.position();
        let outer = self.scene.frame_rect(w).unwrap_or_else(|| win.extent());
        let top = outer.y - wy + y;
        if top < area.y {
            y += area.y - top;
        }
        if top > area.bottom() - TITLE_HEIGHT {
            y -= top - (area.bottom() - TITLE_HEIGHT);
        }
        if (x, y) != (wx, wy) {
            self.scene.set_position(w, x, y);
        }
    }

    fn key(&mut self, time: u32, key: u32, pressed: bool) {
        let input = &mut self.seat.input;
        if pressed {
            if input.keys.contains(&key) {
                // The device's own repeat: clients repeat themselves.
                return;
            }
            input.keys.push(key);
        } else {
            if !input.keys.contains(&key) {
                return;
            }
            input.keys.retain(|&k| k != key);
        }
        let code = u16::try_from(key).unwrap_or(0);
        if pressed && let Some(m) = xkb::modifier_bit(code).filter(|m| m.locks) {
            input.locked ^= m.bit;
        }
        input.depressed = input
            .keys
            .iter()
            .filter_map(|&k| xkb::modifier_bit(u16::try_from(k).unwrap_or(0)))
            .filter(|m| !m.locks)
            .fold(0, |a, m| a | m.bit);
        if !input.keyboard_suspended
            && let Some(window) = self.active_internal
        {
            // An internal window has the keyboard (ADR 0117).
            self.internal_events.push(InternalEvent::Key { window, key, pressed });
            return;
        }
        let Some((c, surface)) = input.keyboard_focus else { return };
        if !self.exists(c, surface) {
            return;
        }
        let serial = self.next_serial();
        let state = if pressed { wl_keyboard::KeyState::PRESSED } else { wl_keyboard::KeyState::RELEASED };
        for k in self.devices(c, wl_keyboard::NAME) {
            self.client(c).send(k, wl_keyboard::Event::Key { serial, time, key, state: state.0 });
        }
        self.send_modifiers(c, None);
    }

    /// `wl_keyboard.modifiers` to client `c`'s keyboards (or only to
    /// `only`) if they changed since it last heard (or for `only`, always).
    fn send_modifiers(&mut self, c: ClientId, only: Option<ObjectId>) {
        let now = (self.seat.input.depressed, self.seat.input.locked);
        if only.is_none() && self.seat.input.sent_mods == Some(now) {
            return;
        }
        self.seat.input.sent_mods = Some(now);
        let serial = self.next_serial();
        let targets = only.map_or_else(|| self.devices(c, wl_keyboard::NAME), |k| alloc::vec![k]);
        for k in targets {
            self.client(c).send(
                k,
                wl_keyboard::Event::Modifiers {
                    serial,
                    mods_depressed: now.0,
                    mods_latched: 0,
                    mods_locked: now.1,
                    group: 0,
                },
            );
        }
    }

    /// `wl_keyboard.enter` with the keys down, then the modifiers, to
    /// client `c`'s keyboards (or only `only`).
    fn keyboard_enter(&mut self, c: ClientId, surface: ObjectId, only: Option<ObjectId>) {
        let serial = self.next_serial();
        let keys = super::xdg::u32_array(&self.seat.input.keys);
        let targets = only.map_or_else(|| self.devices(c, wl_keyboard::NAME), |k| alloc::vec![k]);
        for k in &targets {
            self.client(c).send(*k, wl_keyboard::Event::Enter { serial, surface, keys: keys.clone() });
        }
        for k in targets {
            self.send_modifiers(c, Some(k));
        }
    }

    /// Give the keyboard to the active toplevel's surface (or nobody), if it
    /// is not there already.
    pub(super) fn sync_keyboard_focus(&mut self) {
        let target = if self.seat.input.keyboard_suspended {
            None
        } else {
            self.active.and_then(|(c, tl)| self.toplevel(c, tl).map(|t| (c, t.surface)))
        };
        if target == self.seat.input.keyboard_focus {
            return;
        }
        if let Some((c, surface)) = self.seat.input.keyboard_focus.take()
            && self.exists(c, surface)
        {
            let serial = self.next_serial();
            for k in self.devices(c, wl_keyboard::NAME) {
                self.client(c).send(k, wl_keyboard::Event::Leave { serial, surface });
            }
        }
        self.seat.input.keyboard_focus = target;
        self.seat.input.sent_mods = None;
        if let Some((c, surface)) = target {
            self.keyboard_enter(c, surface, None);
        }
    }

    /// A client made a `wl_keyboard`: its keymap, the repeat rate, and
    /// `enter` if it has the focus.
    pub(super) fn keyboard_created(&mut self, c: ClientId, keyboard: ObjectId) {
        if let Some((fd, size)) = self.host().keymap() {
            let format = wl_keyboard::KeymapFormat::XKB_V1.0;
            self.client(c).send(keyboard, wl_keyboard::Event::Keymap { format, fd, size });
        }
        self.client(c).send(keyboard, wl_keyboard::Event::RepeatInfo { rate: REPEAT_RATE, delay: REPEAT_DELAY });
        if let Some((fc, surface)) = self.seat.input.keyboard_focus
            && fc == c
        {
            self.keyboard_enter(c, surface, Some(keyboard));
        }
    }

    /// The keymap changed (another layout, ADR 0117): every `wl_keyboard`
    /// gets the new one from the host ([`Host::keymap`]); the focused
    /// client hears the modifiers again.
    pub fn keymap_changed(&mut self) {
        let clients: Vec<ClientId> = self.clients.keys().copied().collect();
        for c in clients {
            for k in self.devices(c, wl_keyboard::NAME) {
                if let Some((fd, size)) = self.host().keymap() {
                    let format = wl_keyboard::KeymapFormat::XKB_V1.0;
                    self.client(c).send(k, wl_keyboard::Event::Keymap { format, fd, size });
                }
            }
        }
        if let Some((c, _)) = self.seat.input.keyboard_focus {
            self.seat.input.sent_mods = None;
            self.send_modifiers(c, None);
        }
    }

    /// A client left: forget its focus.
    pub(super) fn input_client_gone(&mut self, c: ClientId) {
        let input = &mut self.seat.input;
        if input.pointer_focus.is_some_and(|f| f.client == c) {
            input.pointer_focus = None;
        }
        if input.keyboard_focus.is_some_and(|(fc, _)| fc == c) {
            input.keyboard_focus = None;
        }
        if let Some(Grab::Move { window, .. }) = input.grab
            && self.scene.window(window).is_none()
        {
            self.seat.input.grab = None;
        }
    }
}
