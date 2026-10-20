//! Against a simulated device that follows the documented behaviour, and
//! ones that do not.

extern crate std;

use super::*;
use core::cell::RefCell;
use std::vec::Vec;

/// How a simulated device misbehaves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Quirk {
    #[default]
    None,
    /// Keeps 8 bits per pixel whatever is written.
    StuckBpp,
    /// Never turns the display on.
    NeverEnables,
    /// Lines shorter than the width.
    ShortLines,
}

#[derive(Debug)]
struct Device {
    id: u16,
    max: Mode,
    vram_64k: u16,
    quirk: Quirk,
    regs: [u16; 16],
    writes: Vec<(u16, u16)>,
}

#[derive(Debug)]
struct Sim(RefCell<Device>);

impl Sim {
    fn new(id: u16) -> Sim {
        Sim(RefCell::new(Device {
            id,
            max: Mode { width: 2560, height: 1600, bpp: 32 },
            vram_64k: 256, // 16 MiB
            quirk: Quirk::None,
            regs: [0; 16],
            writes: Vec::new(),
        }))
    }

    fn quirk(self, q: Quirk) -> Sim {
        self.0.borrow_mut().quirk = q;
        self
    }

    fn writes(&self) -> Vec<(u16, u16)> {
        core::mem::take(&mut self.0.borrow_mut().writes)
    }
}

impl Registers for Sim {
    fn read(&self, i: u16) -> u16 {
        let d = self.0.borrow();
        let caps = d.regs[index::ENABLE as usize] & enable::GETCAPS != 0;
        match i {
            index::ID => d.id,
            index::XRES if caps => d.max.width,
            index::YRES if caps => d.max.height,
            index::BPP if caps => d.max.bpp,
            index::VIDEO_MEMORY_64K => d.vram_64k,
            i if (i as usize) < d.regs.len() => d.regs[i as usize],
            _ => 0xffff,
        }
    }

    fn write(&self, i: u16, v: u16) {
        let mut d = self.0.borrow_mut();
        d.writes.push((i, v));
        let r = |i: u16| i as usize;
        match i {
            index::ENABLE => {
                let was_on = d.regs[r(index::ENABLE)] & enable::ENABLED != 0;
                if d.quirk == Quirk::NeverEnables {
                    return;
                }
                if v & enable::ENABLED != 0 && !was_on {
                    // Switching on sets the mode: lines as wide as the
                    // screen, as many as the memory holds.
                    let (w, bpp) = (d.regs[r(index::XRES)], d.regs[r(index::BPP)]);
                    let virt = if d.quirk == Quirk::ShortLines { w / 2 } else { w };
                    let line = virt as u32 * bpp.div_ceil(8) as u32;
                    d.regs[r(index::VIRT_WIDTH)] = virt;
                    d.regs[r(index::VIRT_HEIGHT)] = (d.vram_64k as u32 * 0x1_0000 / line.max(1)).min(0xffff) as u16;
                    d.regs[r(index::X_OFFSET)] = 0;
                    d.regs[r(index::Y_OFFSET)] = 0;
                }
                d.regs[r(index::ENABLE)] = v;
            }
            index::BPP if d.quirk == Quirk::StuckBpp => d.regs[r(index::BPP)] = 8,
            // Mode registers only change while the display is off.
            index::XRES | index::YRES | index::BPP if d.regs[r(index::ENABLE)] & enable::ENABLED != 0 => {}
            i if (i as usize) < d.regs.len() && i != index::ID => d.regs[r(i)] = v,
            _ => {}
        }
    }
}

const QEMU_ID: u16 = 0xb0c5;

#[test]
fn probe_reads_the_capabilities_and_leaves_the_display_off() {
    let sim = Sim::new(QEMU_ID);
    let caps = probe(&sim).unwrap();
    assert_eq!(caps, Capabilities { version: 5, max: Some(Mode::new(2560, 1600)), vram: Some(16 << 20) });
    assert_eq!(sim.writes(), [(index::ENABLE, enable::GETCAPS), (index::ENABLE, 0)]);
    assert_eq!(current(&sim), None);
}

#[test]
fn probe_rejects_other_devices_and_old_versions() {
    assert_eq!(probe(&Sim::new(0xffff)), Err(Error::NotPresent { id: 0xffff }));
    assert_eq!(probe(&Sim::new(0)), Err(Error::NotPresent { id: 0 }));
    assert_eq!(probe(&Sim::new(0xb0c1)), Err(Error::TooOld { version: 1 }));
    // Version 2 has a linear framebuffer but cannot report its limits.
    let old = Sim::new(0xb0c2);
    assert_eq!(probe(&old), Ok(Capabilities { version: 2, max: None, vram: None }));
    assert!(old.writes().is_empty());
    // A device that reports no memory: unknown.
    let sim = Sim::new(QEMU_ID);
    sim.0.borrow_mut().vram_64k = 0;
    assert_eq!(probe(&sim).unwrap().vram, None);
}

#[test]
fn set_mode_follows_the_protocol() {
    let sim = Sim::new(QEMU_ID);
    let caps = probe(&sim).unwrap();
    sim.writes();
    let info = set_mode(&sim, &caps, Mode::new(1280, 800), 16 << 20).unwrap();
    assert_eq!(info.mode, Mode::new(1280, 800));
    assert_eq!((info.virt_width, info.stride(), info.offset()), (1280, 5120, 0));
    assert_eq!(info.virt_height, ((16u32 << 20) / 5120) as u16);
    // Off, then the mode, then on with the linear framebuffer.
    assert_eq!(
        sim.writes()[..5],
        [
            (index::ENABLE, 0),
            (index::XRES, 1280),
            (index::YRES, 800),
            (index::BPP, 32),
            (index::ENABLE, enable::ENABLED | enable::LFB)
        ]
    );
    assert_eq!(current(&sim), Some(info));
    // A second mode change goes through off again, so the device applies it.
    let info = set_mode(&sim, &caps, Mode::new(640, 480), 16 << 20).unwrap();
    assert_eq!((info.mode, info.stride()), (Mode::new(640, 480), 2560));
    disable(&sim);
    assert_eq!(current(&sim), None);
}

#[test]
fn set_mode_checks_limits_first() {
    let sim = Sim::new(QEMU_ID);
    let caps = probe(&sim).unwrap();
    sim.writes();
    let max = Mode::new(2560, 1600);
    assert_eq!(
        set_mode(&sim, &caps, Mode::new(4096, 800), 64 << 20),
        Err(Error::Unsupported { asked: Mode::new(4096, 800), max })
    );
    let deep = Mode { width: 640, height: 480, bpp: 48 };
    assert_eq!(set_mode(&sim, &caps, deep, 64 << 20), Err(Error::Unsupported { asked: deep, max }));
    // The smaller of the caller's and the device's memory size counts.
    let big = Mode::new(2560, 1600);
    assert_eq!(set_mode(&sim, &caps, big, 8 << 20), Err(Error::TooLarge { needed: 2560 * 1600 * 4, vram: 8 << 20 }));
    sim.0.borrow_mut().vram_64k = 64;
    let caps = probe(&sim).unwrap();
    sim.writes();
    assert_eq!(set_mode(&sim, &caps, big, 64 << 20), Err(Error::TooLarge { needed: 2560 * 1600 * 4, vram: 4 << 20 }));
    assert_eq!(set_mode(&sim, &caps, Mode::new(0, 10), 64 << 20), Err(Error::TooLarge { needed: 0, vram: 4 << 20 }));
    // Nothing was written for any of them.
    assert!(sim.writes().is_empty());
    // Without GETCAPS, only memory limits the mode.
    let old = Sim::new(0xb0c2);
    let caps = probe(&old).unwrap();
    assert!(set_mode(&old, &caps, Mode::new(2560, 1600), 16 << 20).is_ok());
}

#[test]
fn set_mode_notices_a_device_that_did_not_take_it() {
    let mode = Mode::new(800, 600);
    let try_quirk = |q: Quirk| {
        let sim = Sim::new(QEMU_ID).quirk(q);
        let caps = probe(&sim).unwrap();
        set_mode(&sim, &caps, mode, 16 << 20)
    };
    match try_quirk(Quirk::StuckBpp) {
        Err(Error::Rejected { asked, got: Some(got) }) => {
            assert_eq!((asked, got.mode), (mode, Mode { width: 800, height: 600, bpp: 8 }))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(try_quirk(Quirk::NeverEnables), Err(Error::Rejected { asked: mode, got: None }));
    assert!(matches!(try_quirk(Quirk::ShortLines), Err(Error::Rejected { got: Some(g), .. }) if g.virt_width == 400));
}

#[test]
fn errors_explain_themselves() {
    let e = Error::Rejected { asked: Mode::new(800, 600), got: None };
    assert_eq!(std::format!("{e}"), "asked for 800x600x32, the device left the display off");
    let e = Error::NotPresent { id: 0xffff };
    assert_eq!(std::format!("{e}"), "no Bochs VBE interface (ID register reads 0xffff)");
}

#[test]
fn parses_modes() {
    assert_eq!(Mode::parse("1280x800"), Some(Mode::new(1280, 800)));
    assert_eq!(Mode::parse("1920X1080"), Some(Mode::new(1920, 1080)));
    for bad in
        ["", "x", "1280", "1280x", "x800", "0x800", "1280x0", "-1x5", "+5x5", "70000x10", "12 80x800", "1280x800x32"]
    {
        assert_eq!(Mode::parse(bad), None, "{bad:?}");
    }
    assert_eq!(Mode::new(2, 3).bytes(), 24);
}
