//! A simulated i8042 controller with a keyboard on its first port, for the
//! unit tests. It follows the documented behaviour (status bits, controller
//! commands, configuration byte, keyboard commands and replies) and can be
//! told to misbehave: no controller, a mute one, failing self-tests, no
//! keyboard, a keyboard that asks for resends or refuses commands.

extern crate std;

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::vec::Vec;

/// What the driver did, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Command(u8),
    Data(u8),
}

#[derive(Debug)]
pub struct Keyboard {
    /// Answer this many bytes with RESEND before handling any.
    pub resend_first: u32,
    /// Keyboard commands answered with RESEND every time.
    pub refuse: Vec<u8>,
    /// What the self-test after a reset reports.
    pub bat: u8,
    /// Bytes the keyboard sends right before its next reply (keys typed
    /// during bring-up).
    pub stray: Vec<u8>,
    pub scanning: bool,
    pub set: u8,
    pub leds: u8,
    /// The command whose parameter byte comes next.
    param_for: Option<u8>,
}

impl Default for Keyboard {
    fn default() -> Keyboard {
        Keyboard {
            resend_first: 0,
            refuse: Vec::new(),
            bat: BAT_OK,
            stray: Vec::new(),
            scanning: true,
            set: 2,
            leds: 0,
            param_for: None,
        }
    }
}

/// A mouse on the second port.
#[derive(Debug)]
pub struct Mouse {
    /// Understands the IntelliMouse wheel knock (then ID 3).
    pub wheel: bool,
    /// ... and the five-button knock (then ID 4).
    pub five_buttons: bool,
    pub id: u8,
    pub reporting: bool,
    pub rate: u8,
    pub resolution: u8,
    /// The last three sample rates set.
    rates: Vec<u8>,
    /// What the self-test after a reset reports.
    pub bat: u8,
    /// Commands answered with RESEND every time.
    pub refuse: Vec<u8>,
    param_for: Option<u8>,
}

impl Default for Mouse {
    fn default() -> Mouse {
        Mouse {
            wheel: true,
            five_buttons: false,
            id: 0,
            reporting: false,
            rate: 100,
            resolution: 2,
            rates: Vec::new(),
            bat: BAT_OK,
            refuse: Vec::new(),
            param_for: None,
        }
    }
}

#[derive(Debug)]
pub struct State {
    /// Nothing decodes the ports: reads give 0xFF, writes vanish.
    pub absent: bool,
    /// The controller takes bytes but never answers.
    pub mute: bool,
    pub config: u8,
    /// Bytes for the driver, with "from the second port".
    pub output: VecDeque<(u8, bool)>,
    pub self_test_reply: u8,
    /// The configuration byte becomes this after the self-test.
    pub config_after_self_test: Option<u8>,
    pub port_test_reply: u8,
    /// The second port's test result; `None`: no second port (no reply).
    pub aux_port_test_reply: Option<u8>,
    pub keyboard: Option<Keyboard>,
    pub mouse: Option<Mouse>,
    /// Status reports "input full" for this many reads after each write;
    /// `u32::MAX` keeps it full for good.
    pub busy_reads: u32,
    busy_left: u32,
    /// The controller command whose data byte comes next.
    pending: Option<u8>,
    pub log: Vec<Op>,
}

#[derive(Debug)]
pub struct Sim {
    pub state: RefCell<State>,
    clock: Cell<u64>,
}

impl Sim {
    /// A healthy controller (firmware left translation and the keyboard
    /// interrupt on) with a keyboard.
    pub fn new() -> Sim {
        Sim {
            state: RefCell::new(State {
                absent: false,
                mute: false,
                config: CONFIG_KEYBOARD_IRQ | CONFIG_SYSTEM_FLAG | CONFIG_TRANSLATE,
                output: VecDeque::new(),
                self_test_reply: SELF_TEST_OK,
                config_after_self_test: None,
                port_test_reply: PORT_TEST_OK,
                aux_port_test_reply: Some(PORT_TEST_OK),
                keyboard: Some(Keyboard::default()),
                mouse: Some(Mouse::default()),
                busy_reads: 0,
                busy_left: 0,
                pending: None,
                log: Vec::new(),
            }),
            clock: Cell::new(0),
        }
    }

    pub fn with(f: impl FnOnce(&mut State)) -> Sim {
        let sim = Sim::new();
        f(&mut sim.state.borrow_mut());
        sim
    }

    pub fn keyboard<R>(&self, f: impl FnOnce(&mut Keyboard) -> R) -> R {
        f(self.state.borrow_mut().keyboard.as_mut().expect("no keyboard"))
    }

    pub fn mouse<R>(&self, f: impl FnOnce(&mut Mouse) -> R) -> R {
        f(self.state.borrow_mut().mouse.as_mut().expect("no mouse"))
    }

    /// Whether IRQ 12 would be raised now.
    pub fn aux_irq(&self) -> bool {
        let s = self.state.borrow();
        s.config & CONFIG_AUX_IRQ != 0 && s.output.front().is_some_and(|&(_, aux)| aux)
    }

    pub fn log(&self) -> Vec<Op> {
        self.state.borrow().log.clone()
    }

    pub fn config(&self) -> u8 {
        self.state.borrow().config
    }

    /// The keyboard sends `bytes` (if its port is enabled and it scans).
    pub fn type_bytes(&self, bytes: &[u8]) {
        let mut s = self.state.borrow_mut();
        if s.keyboard_port_enabled() && s.keyboard.as_ref().is_some_and(|k| k.scanning) {
            s.output.extend(bytes.iter().map(|&b| (b, false)));
        }
    }

    /// The second port sends a byte.
    pub fn aux_byte(&self, byte: u8) {
        self.state.borrow_mut().output.push_back((byte, true));
    }

    /// Whether IRQ 1 would be raised now.
    pub fn keyboard_irq(&self) -> bool {
        let s = self.state.borrow();
        s.config & CONFIG_KEYBOARD_IRQ != 0 && s.output.front().is_some_and(|&(_, aux)| !aux)
    }

    pub fn elapsed_ns(&self) -> u64 {
        self.clock.get()
    }
}

impl State {
    fn keyboard_port_enabled(&self) -> bool {
        self.config & CONFIG_KEYBOARD_CLOCK_OFF == 0
    }

    fn reply(&mut self, byte: u8) {
        if !self.mute {
            self.output.push_back((byte, false));
        }
    }

    fn aux_port_enabled(&self) -> bool {
        self.config & CONFIG_AUX_CLOCK_OFF == 0
    }

    fn aux_reply(&mut self, byte: u8) {
        if !self.mute {
            self.output.push_back((byte, true));
        }
    }

    fn controller_command(&mut self, cmd: u8) {
        match cmd {
            CMD_READ_CONFIG => self.reply(self.config),
            CMD_WRITE_CONFIG | CMD_WRITE_KEYBOARD_OUTPUT | CMD_WRITE_AUX | CMD_WRITE_AUX_OUTPUT => {
                self.pending = Some(cmd)
            }
            CMD_DISABLE_AUX => self.config |= CONFIG_AUX_CLOCK_OFF,
            CMD_ENABLE_AUX => self.config &= !CONFIG_AUX_CLOCK_OFF,
            CMD_TEST_AUX_PORT => {
                if let Some(r) = self.aux_port_test_reply {
                    self.reply(r);
                }
            }
            CMD_SELF_TEST => {
                self.reply(self.self_test_reply);
                if let Some(c) = self.config_after_self_test {
                    self.config = c;
                }
            }
            CMD_TEST_KEYBOARD_PORT => self.reply(self.port_test_reply),
            CMD_DISABLE_KEYBOARD => self.config |= CONFIG_KEYBOARD_CLOCK_OFF,
            CMD_ENABLE_KEYBOARD => self.config &= !CONFIG_KEYBOARD_CLOCK_OFF,
            _ => {}
        }
    }

    fn data(&mut self, byte: u8) {
        match self.pending.take() {
            Some(CMD_WRITE_CONFIG) => self.config = byte,
            Some(CMD_WRITE_KEYBOARD_OUTPUT) => self.reply(byte),
            Some(CMD_WRITE_AUX_OUTPUT) => self.aux_reply(byte),
            Some(CMD_WRITE_AUX) => self.mouse_receives(byte),
            _ => self.keyboard_receives(byte),
        }
    }

    fn mouse_receives(&mut self, byte: u8) {
        use crate::ps2mouse as m;
        if !self.aux_port_enabled() {
            return;
        }
        let Some(mouse) = self.mouse.as_mut() else { return };
        let mut replies = Vec::new();
        if let Some(cmd) = mouse.param_for.take() {
            match cmd {
                m::SET_SAMPLE_RATE => {
                    mouse.rate = byte;
                    mouse.rates.push(byte);
                    if mouse.rates.len() > 3 {
                        mouse.rates.remove(0);
                    }
                    if mouse.rates == m::WHEEL_KNOCK && mouse.wheel {
                        mouse.id = m::ID_WHEEL;
                    }
                    if mouse.rates == m::FIVE_BUTTON_KNOCK && mouse.five_buttons && mouse.id == m::ID_WHEEL {
                        mouse.id = m::ID_FIVE_BUTTONS;
                    }
                }
                m::SET_RESOLUTION => mouse.resolution = byte,
                _ => {}
            }
            replies.push(ACK);
        } else if mouse.refuse.contains(&byte) {
            replies.push(RESEND);
        } else {
            match byte {
                m::RESET => {
                    let bat = mouse.bat;
                    let refuse = core::mem::take(&mut mouse.refuse);
                    *mouse =
                        Mouse { wheel: mouse.wheel, five_buttons: mouse.five_buttons, bat, refuse, ..Mouse::default() };
                    replies.extend([ACK, bat, 0x00]);
                }
                m::SET_SAMPLE_RATE | m::SET_RESOLUTION => {
                    mouse.param_for = Some(byte);
                    replies.push(ACK);
                }
                m::GET_ID => replies.extend([ACK, mouse.id]),
                m::ENABLE_REPORTING => {
                    mouse.reporting = true;
                    replies.push(ACK);
                }
                m::DISABLE_REPORTING | m::SET_DEFAULTS | m::SET_SCALING_1_1 => {
                    mouse.reporting &= byte != m::DISABLE_REPORTING;
                    replies.push(ACK);
                }
                _ => replies.push(RESEND),
            }
        }
        for r in replies {
            self.aux_reply(r);
        }
    }

    fn keyboard_receives(&mut self, byte: u8) {
        if !self.keyboard_port_enabled() {
            return;
        }
        let Some(k) = self.keyboard.as_mut() else { return };
        let mut replies: Vec<u8> = core::mem::take(&mut k.stray);
        if k.resend_first > 0 {
            k.resend_first -= 1;
            replies.push(RESEND);
        } else if let Some(cmd) = k.param_for.take() {
            match cmd {
                KBD_SET_LEDS => k.leds = byte,
                KBD_SCANCODE_SET => k.set = byte,
                _ => {}
            }
            replies.push(ACK);
        } else if k.refuse.contains(&byte) {
            replies.push(RESEND);
        } else {
            match byte {
                KBD_RESET => {
                    k.set = 2;
                    k.scanning = true;
                    k.leds = 0;
                    replies.extend([ACK, k.bat]);
                }
                KBD_SET_LEDS | KBD_SCANCODE_SET => {
                    k.param_for = Some(byte);
                    replies.push(ACK);
                }
                KBD_ENABLE_SCANNING => {
                    k.scanning = true;
                    replies.push(ACK);
                }
                KBD_DISABLE_SCANNING => {
                    k.scanning = false;
                    replies.push(ACK);
                }
                KBD_ECHO => replies.push(KBD_ECHO),
                _ => replies.push(RESEND),
            }
        }
        for r in replies {
            self.reply(r);
        }
    }
}

impl Io for Sim {
    fn read_status(&self) -> u8 {
        let mut s = self.state.borrow_mut();
        if s.absent {
            return 0xff;
        }
        let mut status = 0;
        if let Some(&(_, aux)) = s.output.front() {
            status |= STATUS_OUTPUT_FULL;
            if aux {
                status |= STATUS_AUX_DATA;
            }
        }
        if s.busy_left > 0 {
            status |= STATUS_INPUT_FULL;
            if s.busy_left != u32::MAX {
                s.busy_left -= 1;
            }
        }
        status
    }

    fn read_data(&self) -> u8 {
        let mut s = self.state.borrow_mut();
        if s.absent {
            return 0xff;
        }
        s.output.pop_front().map_or(0, |(b, _)| b)
    }

    fn write_command(&self, command: u8) {
        let mut s = self.state.borrow_mut();
        if s.absent {
            return;
        }
        s.log.push(Op::Command(command));
        s.busy_left = s.busy_reads;
        s.controller_command(command);
    }

    fn write_data(&self, byte: u8) {
        let mut s = self.state.borrow_mut();
        if s.absent {
            return;
        }
        s.log.push(Op::Data(byte));
        s.busy_left = s.busy_reads;
        s.data(byte);
    }

    fn now_ns(&self) -> u64 {
        // Every look at the clock costs 10 µs, so timeouts end.
        self.clock.set(self.clock.get() + 10_000);
        self.clock.get()
    }
}
