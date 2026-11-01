//! A simulated HDA controller and codecs for the unit tests, written from
//! the device-side requirements of HDA 1.0a (§3.3 registers, §4.2 reset,
//! §4.4 CORB / RIRB, §3.6 BDL and stream DMA, §7.3 codec verbs), not from
//! the driver code.
//!
//! One shared [`Device`] sits behind the register block ([`Regs`]); tests
//! keep a [`Sim`] handle to set it up and inspect it. Commands are answered
//! as soon as CORBWP moves (while the CORB runs). Stream DMA happens when
//! a test calls [`Sim::run_output`]. Bus addresses are host addresses.

extern crate std;

use crate::regs::{self, sd, sdctl, sdsts};
use core::cell::RefCell;
use poly_pci::Mmio;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::{alloc, vec::Vec};

/// A zeroed, page-aligned allocation standing in for DMA memory.
#[derive(Debug)]
pub struct DmaMemory {
    pub ptr: core::ptr::NonNull<u8>,
    pub len: usize,
    layout: alloc::Layout,
}

impl DmaMemory {
    pub fn new(len: usize) -> DmaMemory {
        let layout = alloc::Layout::from_size_align(len.max(1), 4096).unwrap();
        // SAFETY: the layout has a non-zero size.
        let ptr = core::ptr::NonNull::new(unsafe { alloc::alloc_zeroed(layout) }).unwrap();
        DmaMemory { ptr, len, layout }
    }

    pub fn region(&self) -> crate::DmaRegion {
        crate::DmaRegion { ptr: self.ptr, bus: self.ptr.as_ptr() as u64, len: self.len }
    }

    pub fn bytes(&self) -> Vec<u8> {
        // SAFETY: `len` bytes allocated in `new`, alive while `self` is.
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }.to_vec()
    }
}

impl Drop for DmaMemory {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this layout.
        unsafe { alloc::dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

fn rd<T: Copy>(addr: u64) -> T {
    // SAFETY: the driver handed these addresses to the device; tests keep
    // the memory alive while the device works.
    unsafe { core::ptr::read_volatile(addr as *const T) }
}

fn wr<T: Copy>(addr: u64, v: T) {
    // SAFETY: as for `rd`.
    unsafe { core::ptr::write_volatile(addr as *mut T, v) }
}

// ---- codecs ----------------------------------------------------------------

/// Widget capability words for the builders (§7.3.4.6).
pub mod caps {
    pub const STEREO: u32 = 1 << 0;
    pub const IN_AMP: u32 = 1 << 1;
    pub const OUT_AMP: u32 = 1 << 2;
    pub const AMP_OVERRIDE: u32 = 1 << 3;
    pub const FORMAT_OVERRIDE: u32 = 1 << 4;
    pub const CONN_LIST: u32 = 1 << 8;
    pub const POWER: u32 = 1 << 10;
    pub const fn kind(t: u32) -> u32 {
        t << 20
    }
    pub const DAC: u32 = kind(0) | STEREO | OUT_AMP | POWER;
    pub const MIXER: u32 = kind(2) | STEREO | IN_AMP | OUT_AMP | CONN_LIST;
    pub const SELECTOR: u32 = kind(3) | STEREO | CONN_LIST;
    pub const PIN: u32 = kind(4) | STEREO | OUT_AMP | CONN_LIST | POWER;
}

/// Pin capability bits (§7.3.4.9).
pub mod pincaps {
    pub const HP: u32 = 1 << 3;
    pub const OUT: u32 = 1 << 4;
    pub const IN: u32 = 1 << 5;
    pub const EAPD: u32 = 1 << 16;
}

/// Configuration default words (§7.3.3): connectivity and default device.
pub mod config {
    pub const fn make(connectivity: u32, device: u32) -> u32 {
        (connectivity << 30) | (device << 20) | 0x0101_0010
    }
    pub const LINE_OUT: u32 = make(0, 0);
    pub const SPEAKER: u32 = make(2, 1);
    pub const HP_OUT: u32 = make(0, 2);
    pub const MIC: u32 = make(0, 0xA);
    pub const fn disconnected(device: u32) -> u32 {
        make(1, device)
    }
}

pub const AFG_PCM: u32 = (0b00110 << 16) | 0x070; // 16/20-bit; 32k, 44.1k, 48k
pub const OUT_AMP_CAPS: u32 = 0x8005_3F27;
pub const IN_AMP_CAPS: u32 = 0x8003_1F17;

/// One node of a codec: its parameters, connection list, configuration
/// default, and the state the set verbs change.
#[derive(Clone, Debug, Default)]
pub struct Node {
    pub params: BTreeMap<u8, u32>,
    /// Raw connection list entries (short or long form, range bits
    /// included).
    pub conn: Vec<u16>,
    pub long_form: bool,
    pub config: u32,
    pub power: u8,
    /// (output, left, index) → mute bit 7 | gain 6:0. Absent: muted, 0.
    pub amps: BTreeMap<(bool, bool, u8), u8>,
    pub pin_ctl: u8,
    pub select: u8,
    pub eapd: u8,
    pub format: u16,
    pub stream_chan: u8,
}

impl Node {
    pub fn amp(&self, output: bool, left: bool, index: u8) -> u8 {
        self.amps.get(&(output, left, index)).copied().unwrap_or(0x80)
    }

    /// Both channels unmuted at `gain`.
    pub fn amp_is(&self, output: bool, index: u8, gain: u8) -> bool {
        self.amp(output, true, index) == gain && self.amp(output, false, index) == gain
    }
}

#[derive(Clone, Debug)]
pub struct Codec {
    pub addr: u8,
    pub nodes: BTreeMap<u8, Node>,
    /// Answer commands at all.
    pub responsive: bool,
    /// Unsolicited responses sent before each solicited one (if the
    /// controller accepts them, GCTL.UNSOL).
    pub unsolicited_before_each: u32,
    /// Every command word received.
    pub log: Vec<u32>,
}

impl Codec {
    /// A codec with one audio function group at `afg` and no widgets yet.
    pub fn new(addr: u8, afg: u8) -> Codec {
        let mut c =
            Codec { addr, nodes: BTreeMap::new(), responsive: true, unsolicited_before_each: 0, log: Vec::new() };
        let root = c.nodes.entry(0).or_default();
        root.params.insert(0x00, 0x1AF4_0022);
        root.params.insert(0x04, ((afg as u32) << 16) | 1);
        let a = c.nodes.entry(afg).or_default();
        a.params.insert(0x05, 0x01);
        a.params.insert(0x0A, AFG_PCM);
        a.params.insert(0x0B, 1);
        a.params.insert(0x0D, IN_AMP_CAPS);
        a.params.insert(0x12, OUT_AMP_CAPS);
        a.params.insert(0x04, 0);
        a.power = 3;
        c
    }

    pub fn afg(&self) -> u8 {
        (self.nodes[&0].params[&0x04] >> 16) as u8
    }

    pub fn node(&self, nid: u8) -> &Node {
        &self.nodes[&nid]
    }

    pub fn node_mut(&mut self, nid: u8) -> &mut Node {
        self.nodes.entry(nid).or_default()
    }

    /// Add a widget; the AFG's node count grows to cover it (NIDs are
    /// expected to be contiguous).
    pub fn widget(&mut self, nid: u8, caps: u32, conns: &[u8]) -> &mut Codec {
        let afg = self.afg();
        let n = self.node_mut(nid);
        n.params.insert(0x09, caps);
        n.power = 3;
        n.conn = conns.iter().map(|&c| c as u16).collect();
        n.params.insert(0x0E, conns.len() as u32);
        let a = self.node_mut(afg);
        let cur = a.params[&0x04];
        let (start, count) = ((cur >> 16) & 0xFF, cur & 0xFF);
        let (lo, hi) = if count == 0 {
            (nid as u32, nid as u32)
        } else {
            (start.min(nid as u32), (start + count - 1).max(nid as u32))
        };
        a.params.insert(0x04, (lo << 16) | (hi - lo + 1));
        self
    }

    pub fn dac(&mut self, nid: u8) -> &mut Codec {
        self.widget(nid, caps::DAC, &[])
    }

    pub fn mixer(&mut self, nid: u8, conns: &[u8]) -> &mut Codec {
        self.widget(nid, caps::MIXER, conns)
    }

    pub fn selector(&mut self, nid: u8, conns: &[u8]) -> &mut Codec {
        self.widget(nid, caps::SELECTOR, conns)
    }

    pub fn pin(&mut self, nid: u8, conns: &[u8], pin_caps: u32, config: u32) -> &mut Codec {
        self.widget(nid, caps::PIN, conns);
        let n = self.node_mut(nid);
        n.params.insert(0x0C, pin_caps);
        n.config = config;
        self
    }

    /// Answer one verb (§7.3.3) to `nid`. Unknown nodes and verbs answer 0.
    pub fn handle(&mut self, nid: u8, verb: u32) -> u32 {
        let Some(n) = self.nodes.get_mut(&nid) else { return 0 };
        let top = (verb >> 16) & 0xF;
        if top == 0x7 || top == 0xF {
            let id = (verb >> 8) as u16;
            let pl = verb as u8;
            match id {
                0xF00 => n.params.get(&pl).copied().unwrap_or(0),
                0xF01 => n.select as u32,
                0x701 => {
                    n.select = pl;
                    0
                }
                0xF02 => {
                    let start = pl as usize;
                    let (per, width, mask) = if n.long_form { (2, 16, 0xFFFF) } else { (4, 8, 0xFF) };
                    (0..per).fold(0u32, |acc, k| {
                        acc | ((n.conn.get(start + k).copied().unwrap_or(0) as u32 & mask) << (k * width))
                    })
                }
                0xF05 => ((n.power as u32) << 4) | n.power as u32,
                0x705 => {
                    n.power = pl & 0xF;
                    0
                }
                0xF06 => n.stream_chan as u32,
                0x706 => {
                    n.stream_chan = pl;
                    0
                }
                0xF07 => n.pin_ctl as u32,
                0x707 => {
                    n.pin_ctl = pl;
                    0
                }
                0xF0C => n.eapd as u32,
                0x70C => {
                    n.eapd = pl;
                    0
                }
                0xF1C => n.config,
                _ => 0,
            }
        } else {
            let pl = verb as u16;
            match top {
                0x2 => {
                    n.format = pl;
                    0
                }
                0xA => n.format as u32,
                0x3 => {
                    let idx = ((pl >> 8) & 0xF) as u8;
                    let val = (pl & 0xFF) as u8;
                    for (dir_bit, out) in [(0x8000u16, true), (0x4000, false)] {
                        for (side_bit, left) in [(0x2000u16, true), (0x1000, false)] {
                            if pl & dir_bit != 0 && pl & side_bit != 0 {
                                n.amps.insert((out, left, idx), val);
                            }
                        }
                    }
                    0
                }
                0xB => n.amp(pl & 0x8000 != 0, pl & 0x2000 != 0, (pl & 0xF) as u8) as u32,
                _ => 0,
            }
        }
    }
}

// ---- the controller ----------------------------------------------------------

const SPACE: usize = 0x400;

#[derive(Debug)]
pub struct Device {
    bytes: [u8; SPACE],
    pub codecs: Vec<Codec>,
    pub gcap: u16,
    pub corb_size_caps: u8,
    pub rirb_size_caps: u8,
    /// CRST never reads back 1.
    pub stuck_in_reset: bool,
    /// The CORB read pointer reset bit never reads back 1 (allowed: the
    /// driver must only insist on it clearing).
    pub corbrp_reset_invisible: bool,
    /// Take no command while RINTFL is set, and set it only with the
    /// response interrupt control on (QEMU's controller, observed).
    pub qemu_rirb: bool,
    corb_rp: u16,
    corb_rp_reset: bool,
    rirb_wp: u16,
    responses: u32,
    /// Output stream engine: current BDL entry and offset in it.
    bd_index: u32,
    bd_off: u32,
    /// Bytes the output stream has fetched, in order.
    pub played: Vec<u8>,
    /// BDL entries with IOC completed.
    pub completions: u32,
    /// Times the controller went through reset (CRST 1 → 0).
    pub resets: u32,
}

/// The device side, shared by the test and the register block.
#[derive(Clone, Debug)]
pub struct Sim(pub Rc<RefCell<Device>>);

/// The register block the driver gets.
#[derive(Clone, Debug)]
pub struct Regs(pub Rc<RefCell<Device>>);

impl Sim {
    /// A controller with 4 input and 4 output streams, 64-bit addressing,
    /// CORB/RIRB sizes 2/16/256, out of reset (as firmware leaves it).
    pub fn new(codecs: Vec<Codec>) -> Sim {
        let mut d = Device {
            bytes: [0; SPACE],
            codecs,
            gcap: (4 << 12) | (4 << 8) | 1,
            corb_size_caps: 0x70,
            rirb_size_caps: 0x70,
            stuck_in_reset: false,
            corbrp_reset_invisible: false,
            qemu_rirb: true,
            corb_rp: 0,
            corb_rp_reset: false,
            rirb_wp: 0,
            responses: 0,
            bd_index: 0,
            bd_off: 0,
            played: Vec::new(),
            completions: 0,
            resets: 0,
        };
        d.reset_registers();
        d.bytes[regs::GCTL] = 1;
        let present = d.present();
        d.bytes[regs::STATESTS..regs::STATESTS + 2].copy_from_slice(&present.to_le_bytes());
        Sim(Rc::new(RefCell::new(d)))
    }

    pub fn regs(&self) -> Regs {
        Regs(self.0.clone())
    }

    pub fn dev(&self) -> std::cell::RefMut<'_, Device> {
        self.0.borrow_mut()
    }

    pub fn codec(&self, addr: u8) -> Codec {
        self.0.borrow().codecs.iter().find(|c| c.addr == addr).unwrap().clone()
    }

    pub fn reg8(&self, off: usize) -> u8 {
        self.0.borrow().read(off, 1) as u8
    }
    pub fn reg16(&self, off: usize) -> u16 {
        self.0.borrow().read(off, 2) as u16
    }
    pub fn reg32(&self, off: usize) -> u32 {
        self.0.borrow().read(off, 4)
    }

    /// Let the output stream's DMA fetch `n` bytes, if it runs. Returns the
    /// bytes fetched.
    pub fn run_output(&self, n: usize) -> usize {
        self.0.borrow_mut().run_output(n)
    }

    /// Would the interrupt line be asserted: GIE and an enabled cause?
    pub fn irq(&self) -> bool {
        let d = self.0.borrow();
        let ctl = d.read(regs::INTCTL, 4);
        let sts = d.intsts();
        ctl & regs::int::GLOBAL != 0 && sts & ctl & (regs::int::STREAMS | regs::int::CONTROLLER) != 0
    }
}

impl Device {
    fn present(&self) -> u16 {
        self.codecs.iter().fold(0, |m, c| m | (1 << c.addr))
    }

    fn streams(&self) -> usize {
        let g = self.gcap;
        (((g >> 12) & 0xF) + ((g >> 8) & 0xF) + ((g >> 3) & 0x1F)) as usize
    }

    fn output_index(&self) -> usize {
        ((self.gcap >> 8) & 0xF) as usize
    }

    /// Register defaults (§3.3): everything 0 except the capabilities.
    fn reset_registers(&mut self) {
        self.bytes = [0; SPACE];
        self.bytes[0..2].copy_from_slice(&self.gcap.to_le_bytes());
        self.bytes[regs::VMIN] = 0;
        self.bytes[regs::VMAJ] = 1;
        self.bytes[0x04] = 0x3C; // OUTPAY
        self.bytes[0x06] = 0x1D; // INPAY
        self.bytes[regs::CORBSIZE] = self.corb_size_caps;
        self.bytes[regs::RIRBSIZE] = self.rirb_size_caps;
        self.corb_rp = 0;
        self.corb_rp_reset = false;
        self.rirb_wp = 0;
        self.responses = 0;
        self.bd_index = 0;
        self.bd_off = 0;
    }

    fn crst(&self) -> bool {
        self.bytes[regs::GCTL] & 1 != 0
    }

    fn u32_at(&self, off: usize) -> u32 {
        u32::from_le_bytes(self.bytes[off..off + 4].try_into().unwrap())
    }

    fn intsts(&self) -> u32 {
        let mut v = 0;
        for n in 0..self.streams().min(30) {
            let ctl = self.bytes[regs::sd(n)];
            let sts = self.bytes[regs::sd(n) + sd::STS];
            if (sts & sdsts::BCIS != 0 && ctl & sdctl::IOCE != 0)
                || (sts & sdsts::FIFOE != 0 && ctl & sdctl::FEIE != 0)
                || (sts & sdsts::DESE != 0 && ctl & sdctl::DEIE != 0)
            {
                v |= 1 << n;
            }
        }
        let rs = self.bytes[regs::RIRBSTS];
        let rc = self.bytes[regs::RIRBCTL];
        let statests = u16::from_le_bytes([self.bytes[regs::STATESTS], self.bytes[regs::STATESTS + 1]]);
        let wakeen = u16::from_le_bytes([self.bytes[regs::WAKEEN], self.bytes[regs::WAKEEN + 1]]);
        if (rs & 1 != 0 && rc & 1 != 0)
            || (rs & 4 != 0 && rc & 4 != 0)
            || (self.bytes[regs::CORBSTS] & 1 != 0 && self.bytes[regs::CORBCTL] & 1 != 0)
            || statests & wakeen != 0
        {
            v |= regs::int::CONTROLLER;
        }
        if v != 0 {
            v |= regs::int::GLOBAL;
        }
        v
    }

    fn byte(&self, off: usize) -> u8 {
        match off {
            0x24..=0x27 => (self.intsts() >> (8 * (off - 0x24))) as u8,
            0x4A => self.corb_rp as u8,
            0x4B => {
                if self.corb_rp_reset && !self.corbrp_reset_invisible {
                    0x80
                } else {
                    0
                }
            }
            0x58 => self.rirb_wp as u8,
            0x59 => 0,
            _ => self.bytes[off],
        }
    }

    pub fn read(&self, off: usize, width: usize) -> u32 {
        if off + width > SPACE {
            return u32::MAX;
        }
        (0..width).fold(0, |v, i| v | ((self.byte(off + i) as u32) << (8 * i)))
    }

    fn write_byte(&mut self, off: usize, b: u8) {
        let stream_end = regs::sd(self.streams());
        match off {
            0x00..=0x07 | 0x24..=0x27 | 0x4A | 0x4B | 0x58 | 0x59 => {}
            regs::STATESTS | 0x0F | regs::CORBSTS | regs::RIRBSTS => self.bytes[off] &= !b,
            regs::CORBSIZE | regs::RIRBSIZE => self.bytes[off] = (self.bytes[off] & 0xF0) | (b & 3),
            o if (regs::SD_BASE..stream_end).contains(&o) => {
                let rel = (o - regs::SD_BASE) % regs::SD_STRIDE;
                let ctl = o - rel;
                let in_reset = self.bytes[ctl] & sdctl::SRST != 0;
                match rel {
                    // SDnCTL low byte: SRST always writable.
                    0 => self.bytes[o] = b,
                    _ if in_reset => {}
                    3 => self.bytes[o] &= !(b & sdsts::CLEAR),
                    4..=7 | 0x10 | 0x11 => {}
                    _ => self.bytes[o] = b,
                }
            }
            _ => self.bytes[off] = b,
        }
    }

    pub fn write(&mut self, off: usize, width: usize, val: u32) {
        if off + width > SPACE {
            return;
        }
        let was_up = self.crst();
        // In reset only GCTL is writable (§4.2).
        if !was_up && off != regs::GCTL {
            return;
        }
        let ctl_before: Vec<u8> = (0..self.streams()).map(|n| self.bytes[regs::sd(n)]).collect();
        for i in 0..width {
            self.write_byte(off + i, (val >> (8 * i)) as u8);
        }
        match off {
            regs::GCTL => {
                let up = self.crst();
                if was_up && !up {
                    let keep = self.bytes[regs::GCTL];
                    self.reset_registers();
                    self.bytes[regs::GCTL] = keep;
                    self.resets += 1;
                } else if !was_up && up {
                    if self.stuck_in_reset {
                        self.bytes[regs::GCTL] &= !1;
                    } else {
                        let present = self.present();
                        self.bytes[regs::STATESTS..regs::STATESTS + 2].copy_from_slice(&present.to_le_bytes());
                    }
                }
            }
            regs::CORBWP | regs::CORBCTL | regs::RIRBSTS => self.process_corb(),
            regs::CORBRP => {
                self.corb_rp_reset = val & 0x8000 != 0;
                if self.corb_rp_reset {
                    self.corb_rp = 0;
                }
            }
            regs::RIRBWP if val & 0x8000 != 0 => self.rirb_wp = 0,
            _ => {}
        }
        // Stream resets (§3.3 SDnCTL.SRST): entering reset clears the
        // descriptor and stops its DMA.
        for (n, &before) in ctl_before.iter().enumerate() {
            let base = regs::sd(n);
            let now = self.bytes[base];
            if now & sdctl::SRST != 0 && before & sdctl::SRST == 0 {
                self.bytes[base + 1..base + regs::SD_STRIDE].fill(0);
                self.bytes[base] = sdctl::SRST;
                if n == self.output_index() {
                    self.bd_index = 0;
                    self.bd_off = 0;
                }
            }
        }
    }

    fn ring_entries(sizereg: u8) -> u16 {
        match sizereg & 3 {
            0 => 2,
            1 => 16,
            _ => 256,
        }
    }

    fn process_corb(&mut self) {
        if self.bytes[regs::CORBCTL] & 2 == 0 {
            return;
        }
        let n = Self::ring_entries(self.bytes[regs::CORBSIZE]);
        let wp = self.bytes[regs::CORBWP] as u16 % n;
        let base = self.u32_at(regs::CORBLBASE) as u64 | ((self.u32_at(regs::CORBUBASE) as u64) << 32);
        for _ in 0..n {
            if self.corb_rp == wp || self.corb_blocked() {
                break;
            }
            self.corb_rp = (self.corb_rp + 1) % n;
            let cmd: u32 = rd(base + self.corb_rp as u64 * 4);
            self.command(u32::from_le(cmd));
        }
    }

    fn command(&mut self, cmd: u32) {
        let addr = (cmd >> 28) as u8;
        let nid = (cmd >> 20) as u8;
        let unsol_ok = self.u32_at(regs::GCTL) & regs::gctl::UNSOL != 0;
        let Some(c) = self.codecs.iter_mut().find(|c| c.addr == addr) else { return };
        c.log.push(cmd);
        if !c.responsive {
            return;
        }
        let unsol = if unsol_ok { c.unsolicited_before_each } else { 0 };
        let resp = c.handle(nid, cmd & 0xF_FFFF);
        for i in 0..unsol {
            self.respond(0x0400_0000 | i, addr as u32 | regs::rirb_ex::UNSOL);
        }
        self.respond(resp, addr as u32);
    }

    fn respond(&mut self, resp: u32, ex: u32) {
        if self.bytes[regs::RIRBCTL] & 2 == 0 {
            return;
        }
        let n = Self::ring_entries(self.bytes[regs::RIRBSIZE]);
        let base = self.u32_at(regs::RIRBLBASE) as u64 | ((self.u32_at(regs::RIRBUBASE) as u64) << 32);
        self.rirb_wp = (self.rirb_wp + 1) % n;
        let at = base + self.rirb_wp as u64 * 8;
        wr(at, resp.to_le());
        wr(at + 4, ex.to_le());
        self.responses += 1;
        let cnt = match self.bytes[regs::RINTCNT] {
            0 => 256,
            c => c as u32,
        };
        if self.responses >= cnt && self.bytes[regs::RIRBCTL] & regs::rirbctl::RINTCTL != 0 {
            self.responses = 0;
            self.bytes[regs::RIRBSTS] |= regs::rirbsts::RINTFL;
        }
    }

    /// Like QEMU's controller (observed): with RINTFL set, no more commands
    /// are taken until software clears it; with the response interrupt
    /// control off, responses are counted but RINTFL is never set, so the
    /// controller stops after RINTCNT responses for good.
    fn corb_blocked(&self) -> bool {
        if !self.qemu_rirb {
            return false;
        }
        let cnt = match self.bytes[regs::RINTCNT] {
            0 => 256,
            c => c as u32,
        };
        self.bytes[regs::RIRBSTS] & regs::rirbsts::RINTFL != 0
            || (self.bytes[regs::RIRBCTL] & regs::rirbctl::RINTCTL == 0 && self.responses >= cnt)
    }

    fn run_output(&mut self, n: usize) -> usize {
        let base = regs::sd(self.output_index());
        let ctl = self.bytes[base];
        if ctl & sdctl::RUN == 0 || ctl & sdctl::SRST != 0 {
            return 0;
        }
        let lvi = self.bytes[base + sd::LVI] as u32;
        let cbl = self.u32_at(base + sd::CBL);
        let bdl = self.u32_at(base + sd::BDPL) as u64 | ((self.u32_at(base + sd::BDPU) as u64) << 32);
        let mut lpib = self.u32_at(base + sd::LPIB);
        let mut done = 0usize;
        while done < n {
            let e = bdl + self.bd_index as u64 * 16;
            let addr = u64::from_le(rd(e));
            let len = u32::from_le(rd(e + 8));
            let flags = u32::from_le(rd(e + 12));
            if len == 0 || cbl == 0 {
                self.bytes[base + sd::STS] |= sdsts::DESE;
                break;
            }
            let take = ((n - done) as u32).min(len - self.bd_off.min(len));
            for k in 0..take {
                self.played.push(rd(addr + (self.bd_off + k) as u64));
            }
            self.bd_off += take;
            done += take as usize;
            lpib = (lpib + take) % cbl;
            if self.bd_off >= len {
                if flags & 1 != 0 {
                    self.bytes[base + sd::STS] |= sdsts::BCIS;
                    self.completions += 1;
                }
                self.bd_off = 0;
                self.bd_index = if self.bd_index >= lvi { 0 } else { self.bd_index + 1 };
            }
        }
        self.bytes[base + sd::LPIB..base + sd::LPIB + 4].copy_from_slice(&lpib.to_le_bytes());
        done
    }
}

impl Mmio for Regs {
    fn read8(&self, offset: usize) -> u8 {
        self.0.borrow().read(offset, 1) as u8
    }
    fn read16(&self, offset: usize) -> u16 {
        self.0.borrow().read(offset, 2) as u16
    }
    fn read32(&self, offset: usize) -> u32 {
        self.0.borrow().read(offset, 4)
    }
    fn write8(&self, offset: usize, value: u8) {
        self.0.borrow_mut().write(offset, 1, value as u32)
    }
    fn write16(&self, offset: usize, value: u16) {
        self.0.borrow_mut().write(offset, 2, value as u32)
    }
    fn write32(&self, offset: usize, value: u32) {
        self.0.borrow_mut().write(offset, 4, value)
    }
}
