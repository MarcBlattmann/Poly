//! Display modes: what a connector offers and what a CRTC is set to.
//!
//! A mode is a `struct drm_mode_modeinfo` ([`ModeInfo`]): the pixel clock
//! in kHz and the horizontal and vertical timings (active, sync start, sync
//! end, total). Where they come from:
//!
//! - **EDID detailed timing descriptors** (VESA E-EDID 1.3/1.4, the 18-byte
//!   descriptors at bytes 54, 72, 90 and 108 of the base block whose pixel
//!   clock is not 0): clock in 10 kHz units (bytes 0-1), horizontal active
//!   and blanking (bytes 2-4), vertical active and blanking (5-7), sync
//!   offsets and widths (8-11), the image size in mm (12-14) and the flags
//!   (17: bit 7 interlaced; with digital separate sync, bits 4-3 = 11, bit
//!   2 is the vertical and bit 1 the horizontal sync polarity, 1 positive).
//!   The first one is the preferred timing.
//! - **VESA CVT reduced blanking** (CVT 1.2, "reduced blanking" timings)
//!   for sizes without a descriptor: a horizontal blank of 160 pixels
//!   (front porch 48, sync 32, back porch 80), a vertical front porch of 3
//!   lines, a sync of 4-10 lines by aspect ratio, at least 460 µs of
//!   vertical blank and a back porch of at least 6 lines, the clock rounded
//!   down to 250 kHz, sync polarity +h/-v.
//!
//! A virtio GPU shows any size (its scanout is a rectangle of a resource),
//! so the timings matter only to clients that look at them (the refresh
//! rate, the vblank period).

use crate::uapi::{ModeInfo, mode as m, name_bytes};
use alloc::format;
use alloc::vec::Vec;

/// The largest width or height a mode or framebuffer may have.
pub const MAX_SIZE: u32 = 8192;
/// The smallest.
pub const MIN_SIZE: u32 = 1;

/// The refresh rate of generated modes, Hz.
pub const REFRESH: u32 = 60;

/// Common sizes offered below a connector's preferred one (each at most
/// the preferred size in both directions).
pub const COMMON_SIZES: [(u32, u32); 10] = [
    (2560, 1440),
    (1920, 1200),
    (1920, 1080),
    (1680, 1050),
    (1600, 900),
    (1280, 1024),
    (1280, 800),
    (1280, 720),
    (1024, 768),
    (800, 600),
];

/// The vertical refresh of `mode` in Hz, rounded (0 for a broken mode).
pub fn refresh(mode: &ModeInfo) -> u32 {
    let total = mode.htotal as u64 * mode.vtotal as u64;
    if total == 0 {
        return 0;
    }
    ((mode.clock as u64 * 1000 + total / 2) / total) as u32
}

/// How long one frame of `mode` takes, ns (60 Hz for a mode whose timings
/// say nothing usable).
pub fn frame_ns(mode: &ModeInfo) -> u64 {
    let total = mode.htotal as u64 * mode.vtotal as u64;
    if mode.clock == 0 || total == 0 {
        return 1_000_000_000 / REFRESH as u64;
    }
    // clock is in kHz: a pixel takes 1e6 / clock ns.
    (total * 1_000_000 / mode.clock as u64).max(1_000_000)
}

/// The mode's name as Linux gives it: `<width>x<height>`, `i` appended for
/// interlaced modes.
fn named(mut mode: ModeInfo) -> ModeInfo {
    let i = if mode.flags & m::FLAG_INTERLACE != 0 { "i" } else { "" };
    mode.name = name_bytes(&format!("{}x{}{i}", mode.hdisplay, mode.vdisplay));
    mode.vrefresh = refresh(&mode);
    mode
}

/// A CVT reduced-blanking mode of `width` x `height` at `hz`.
pub fn cvt_rb(width: u32, height: u32, hz: u32) -> ModeInfo {
    const H_BLANK: u32 = 160;
    const H_FRONT: u32 = 48;
    const H_SYNC: u32 = 32;
    const V_FRONT: u32 = 3;
    const MIN_V_BACK: u32 = 6;
    const MIN_V_BLANK_US: u64 = 460;
    let (w, h) = (width.clamp(MIN_SIZE, MAX_SIZE), height.clamp(MIN_SIZE, MAX_SIZE));
    let hz = hz.max(1) as u64;
    // CVT's aspect ratios and their vertical sync widths.
    let v_sync = [(4, 3, 4), (16, 9, 5), (16, 10, 6), (5, 4, 7), (15, 9, 7)]
        .iter()
        .find(|(a, b, _)| w * b == h * a)
        .map_or(10, |&(_, _, s)| s);
    // The line period, in ps, from the frame time less the minimum blank.
    let frame_ps = 1_000_000_000_000 / hz;
    let line_ps = (frame_ps - MIN_V_BLANK_US * 1_000_000).max(1) / h as u64;
    let vbi = (MIN_V_BLANK_US * 1_000_000 / line_ps.max(1)) as u32 + 1;
    let vbi = vbi.max(V_FRONT + v_sync + MIN_V_BACK);
    let (htotal, vtotal) = (w + H_BLANK, h + vbi);
    // kHz, rounded down to 250 kHz.
    let clock = (hz * htotal as u64 * vtotal as u64 / 1000) as u32 / 250 * 250;
    named(ModeInfo {
        clock: clock.max(250),
        hdisplay: w as u16,
        hsync_start: (w + H_FRONT) as u16,
        hsync_end: (w + H_FRONT + H_SYNC) as u16,
        htotal: htotal as u16,
        vdisplay: h as u16,
        vsync_start: (h + V_FRONT) as u16,
        vsync_end: (h + V_FRONT + v_sync) as u16,
        vtotal: vtotal as u16,
        flags: m::FLAG_PHSYNC | m::FLAG_NVSYNC,
        ty: m::TYPE_DRIVER,
        ..ModeInfo::default()
    })
}

/// What an EDID base block says about modes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EdidModes {
    /// The detailed timings, the preferred one first.
    pub modes: Vec<ModeInfo>,
    /// The image size in mm (0 if unknown).
    pub mm: (u32, u32),
}

/// Read the detailed timing descriptors of an EDID base block (header and
/// checksum checked). `None` for a block that is not one.
pub fn edid_modes(edid: &[u8]) -> Option<EdidModes> {
    const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
    let block = edid.get(..128)?;
    if block[..8] != HEADER || block.iter().fold(0u8, |s, &x| s.wrapping_add(x)) != 0 {
        return None;
    }
    let mut out = EdidModes { mm: (block[21] as u32 * 10, block[22] as u32 * 10), ..EdidModes::default() };
    for i in 0..4 {
        let d = &block[54 + i * 18..][..18];
        let clock = u16::from_le_bytes([d[0], d[1]]) as u32 * 10;
        if clock == 0 {
            continue;
        }
        let hi4 = |b: u8, shift: u32| ((b as u32) >> shift & 0xf) << 8;
        let ha = d[2] as u32 | hi4(d[4], 4);
        let hb = d[3] as u32 | hi4(d[4], 0);
        let va = d[5] as u32 | hi4(d[7], 4);
        let vb = d[6] as u32 | hi4(d[7], 0);
        let hso = d[8] as u32 | ((d[11] as u32 >> 6) & 3) << 8;
        let hsw = d[9] as u32 | ((d[11] as u32 >> 4) & 3) << 8;
        let vso = (d[10] as u32 >> 4) | ((d[11] as u32 >> 2) & 3) << 4;
        let vsw = (d[10] as u32 & 0xf) | (d[11] as u32 & 3) << 4;
        if ha == 0 || va == 0 || ha > MAX_SIZE || va > MAX_SIZE {
            continue;
        }
        let mut flags = 0;
        if d[17] & 0x80 != 0 {
            flags |= m::FLAG_INTERLACE;
        }
        if (d[17] >> 3) & 3 == 3 {
            flags |= if d[17] & 0x04 != 0 { m::FLAG_PVSYNC } else { m::FLAG_NVSYNC };
            flags |= if d[17] & 0x02 != 0 { m::FLAG_PHSYNC } else { m::FLAG_NHSYNC };
        } else {
            flags |= m::FLAG_NHSYNC | m::FLAG_NVSYNC;
        }
        if out.modes.is_empty() {
            let mm = (d[12] as u32 | hi4(d[14], 4), d[13] as u32 | hi4(d[14], 0));
            if mm.0 > 0 && mm.1 > 0 {
                out.mm = mm;
            }
        }
        let ty = m::TYPE_DRIVER | if out.modes.is_empty() { m::TYPE_PREFERRED } else { 0 };
        out.modes.push(named(ModeInfo {
            clock,
            hdisplay: ha as u16,
            hsync_start: (ha + hso) as u16,
            hsync_end: (ha + hso + hsw) as u16,
            htotal: (ha + hb) as u16,
            vdisplay: va as u16,
            vsync_start: (va + vso) as u16,
            vsync_end: (va + vso + vsw) as u16,
            vtotal: (va + vb) as u16,
            flags,
            ty,
            ..ModeInfo::default()
        }));
    }
    Some(out)
}

/// The modes of a connected output whose preferred size is `preferred`
/// (from the device) and whose EDID is `edid`: the EDID's detailed
/// timings (else a CVT mode of the preferred size, marked preferred), then
/// the [`COMMON_SIZES`] that fit, each size once. The preferred mode comes
/// first. Also returns the image size in mm.
pub fn output_modes(preferred: (u32, u32), edid: Option<&[u8]>) -> (Vec<ModeInfo>, (u32, u32)) {
    let from_edid = edid.and_then(edid_modes).unwrap_or_default();
    let mut modes = from_edid.modes;
    // The device's preferred size wins over an EDID that disagrees (QEMU's
    // EDID follows `xres`/`yres`, but a resize changes the display info
    // first).
    let (pw, ph) = preferred;
    let edid_preferred = modes.first().map(|m| (m.hdisplay as u32, m.vdisplay as u32));
    if pw > 0 && ph > 0 && edid_preferred != Some((pw, ph)) {
        for mode in &mut modes {
            mode.ty &= !m::TYPE_PREFERRED;
        }
        let mut p = cvt_rb(pw, ph, REFRESH);
        p.ty |= m::TYPE_PREFERRED;
        modes.insert(0, p);
    }
    let (maxw, maxh) = modes.first().map_or((0, 0), |m| (m.hdisplay as u32, m.vdisplay as u32));
    for (w, h) in COMMON_SIZES {
        if w <= maxw && h <= maxh && !modes.iter().any(|m| (m.hdisplay as u32, m.vdisplay as u32) == (w, h)) {
            modes.push(cvt_rb(w, h, REFRESH));
        }
    }
    (modes, from_edid.mm)
}

/// Whether `mode` is one a CRTC can be set to: a size from [`MIN_SIZE`] to
/// [`MAX_SIZE`], a clock, and timings in order (active <= sync start <=
/// sync end <= total), not interlaced or double scanned.
pub fn valid(mode: &ModeInfo) -> bool {
    let h = [mode.hdisplay, mode.hsync_start, mode.hsync_end, mode.htotal];
    let v = [mode.vdisplay, mode.vsync_start, mode.vsync_end, mode.vtotal];
    let ordered = |t: [u16; 4]| t.windows(2).all(|w| w[0] <= w[1]);
    let size = |s: u16| (MIN_SIZE..=MAX_SIZE).contains(&(s as u32));
    mode.clock > 0
        && size(mode.hdisplay)
        && size(mode.vdisplay)
        && ordered(h)
        && ordered(v)
        && mode.flags & (m::FLAG_INTERLACE | m::FLAG_DBLSCAN) == 0
}
