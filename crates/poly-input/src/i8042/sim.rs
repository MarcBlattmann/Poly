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
    pub keyboard: Option<Keyboard>,
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
                keyboard: Some(Keyboard::default()),
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

    fn controller_command(&mut self, cmd: u8) {
        match cmd {
            CMD_READ_CONFIG => self.reply(self.config),
            CMD_WRITE_CONFIG | CMD_WRITE_KEYBOARD_OUTPUT => self.pending = Some(cmd),
            CMD_DISABLE_AUX => self.config |= CONFIG_AUX_CLOCK_OFF,
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
            _ => self.keyboard_receives(byte),
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
