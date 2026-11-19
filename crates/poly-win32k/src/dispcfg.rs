//! The display configuration API (`GetDisplayConfigBufferSizes`, `QueryDisplayConfig`,
//! `DisplayConfigGetDeviceInfo`; wingdi.h, Learn: "Display Configuration"): the one monitor of
//! the desktop described as one path (source 0 to target) with its modes. Sheets:
//! `specs/windows/user32.dll/{GetDisplayConfigBufferSizes,QueryDisplayConfig,
//! DisplayConfigGetDeviceInfo}.yaml`. The calls return the error code and never set the last
//! error (the glue sets it in the two places the sheets name).
//!
//! Layouts of a 64-bit process: `DISPLAYCONFIG_PATH_INFO` 72 bytes, `DISPLAYCONFIG_MODE_INFO` 64.

use alloc::vec::Vec;

pub const ERROR_INVALID_PARAMETER: i32 = 87;
pub const ERROR_GEN_FAILURE: i32 = 31;
pub const ERROR_INSUFFICIENT_BUFFER: i32 = 122;

pub const QDC_ALL_PATHS: u32 = 1;
pub const QDC_ONLY_ACTIVE_PATHS: u32 = 2;
pub const QDC_DATABASE_CURRENT: u32 = 4;
pub const QDC_VIRTUAL_MODE_AWARE: u32 = 0x10;
pub const QDC_INCLUDE_HMD: u32 = 0x20;
pub const QDC_VIRTUAL_REFRESH_RATE_AWARE: u32 = 0x40;

pub const PATH_INFO_SIZE: usize = 72;
pub const MODE_INFO_SIZE: usize = 64;

const MODE_SOURCE: u32 = 1;
const MODE_TARGET: u32 = 2;
const MODE_DESKTOP_IMAGE: u32 = 3;
const PATH_ACTIVE: u32 = 1;
const PATH_SUPPORT_VIRTUAL_MODE: u32 = 8;
const PIXELFORMAT_32BPP: u32 = 4;
const TOPOLOGY_INTERNAL: u32 = 1;
/// `DISPLAYCONFIG_OUTPUT_TECHNOLOGY_OTHER`.
pub const OUTPUT_TECHNOLOGY: u32 = 0xFFFF_FFFF;
const VIDEO_STANDARD_OTHER: u32 = 255;
const PROGRESSIVE: u32 = 1;
const IDENTITY: u32 = 1;
const REFRESH_NUM: u32 = 60_000;
const REFRESH_DEN: u32 = 1_000;

/// The adapter's id: `LUID` of the machine's one (low part, high part).
pub const ADAPTER_LOW: u32 = 0x0000_A110;
pub const ADAPTER_HIGH: i32 = 0;
/// The target id of the one monitor and the source id (the first source is 0).
pub const TARGET_ID: u32 = 0x0003_1020;
pub const SOURCE_ID: u32 = 0;

/// What a flag combination asks for: the base kind, if the combination is valid.
fn base(flags: u32) -> Option<u32> {
    let kind = flags & 7;
    let rest = flags & !7 & !(QDC_VIRTUAL_MODE_AWARE | QDC_INCLUDE_HMD | QDC_VIRTUAL_REFRESH_RATE_AWARE);
    (rest == 0 && kind.count_ones() == 1).then_some(kind)
}

fn modes_per_path(flags: u32) -> u32 {
    if flags & QDC_VIRTUAL_MODE_AWARE != 0 { 3 } else { 2 }
}

/// `GetDisplayConfigBufferSizes`: `(paths, modes)` or the error. One path per monitor.
pub fn buffer_sizes(flags: u32, monitors: u32) -> Result<(u32, u32), i32> {
    base(flags).ok_or(ERROR_INVALID_PARAMETER)?;
    Ok((monitors, monitors * modes_per_path(flags)))
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

/// A little-endian `u32` of a packet.
pub fn get32_at(b: &[u8], at: usize) -> u32 {
    get32(b, at)
}

fn get32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The path of the monitor.
fn path(flags: u32) -> [u8; PATH_INFO_SIZE] {
    let virt = flags & QDC_VIRTUAL_MODE_AWARE != 0;
    let mut p = [0u8; PATH_INFO_SIZE];
    // DISPLAYCONFIG_PATH_SOURCE_INFO at 0: adapterId, id, modeInfoIdx, statusFlags.
    put32(&mut p, 0, ADAPTER_LOW);
    put32(&mut p, 4, ADAPTER_HIGH as u32);
    put32(&mut p, 8, SOURCE_ID);
    // Source mode at index 1; packed with the clone group (0xFFFF: none) when virtual.
    put32(&mut p, 12, if virt { (1 << 16) | 0xFFFF } else { 1 });
    put32(&mut p, 16, 1); // DISPLAYCONFIG_SOURCE_IN_USE
    // DISPLAYCONFIG_PATH_TARGET_INFO at 20.
    put32(&mut p, 20, ADAPTER_LOW);
    put32(&mut p, 24, ADAPTER_HIGH as u32);
    put32(&mut p, 28, TARGET_ID);
    // Target mode at index 0, desktop image mode (2) in the low half when virtual.
    put32(&mut p, 32, if virt { 2 } else { 0 });
    put32(&mut p, 36, OUTPUT_TECHNOLOGY);
    put32(&mut p, 40, IDENTITY); // rotation
    put32(&mut p, 44, IDENTITY); // scaling
    put32(&mut p, 48, REFRESH_NUM);
    put32(&mut p, 52, REFRESH_DEN);
    put32(&mut p, 56, PROGRESSIVE);
    put32(&mut p, 60, 1); // targetAvailable
    put32(&mut p, 64, 1); // DISPLAYCONFIG_TARGET_IN_USE
    put32(&mut p, 68, PATH_ACTIVE | if virt { PATH_SUPPORT_VIRTUAL_MODE } else { 0 });
    p
}

/// DISPLAYCONFIG_VIDEO_SIGNAL_INFO (48 bytes) of a `w` x `h` monitor.
fn video_signal(out: &mut [u8], at: usize, w: u32, h: u32) {
    put64(out, at, 60 * u64::from(w) * u64::from(h)); // pixelRate
    put32(out, at + 8, REFRESH_NUM * h / REFRESH_DEN); // hSyncFreq numerator: lines per second
    put32(out, at + 12, 1);
    put32(out, at + 16, REFRESH_NUM); // vSyncFreq
    put32(out, at + 20, REFRESH_DEN);
    put32(out, at + 24, w); // activeSize
    put32(out, at + 28, h);
    put32(out, at + 32, w); // totalSize
    put32(out, at + 36, h);
    put32(out, at + 40, VIDEO_STANDARD_OTHER);
    put32(out, at + 44, PROGRESSIVE);
}

/// The modes of the monitor, `w` x `h` at (0, 0): target (index 0), source (1), desktop image (2).
fn modes(flags: u32, w: u32, h: u32) -> Vec<[u8; MODE_INFO_SIZE]> {
    let mut v = Vec::new();
    let mut t = [0u8; MODE_INFO_SIZE];
    put32(&mut t, 0, MODE_TARGET);
    put32(&mut t, 4, TARGET_ID);
    put32(&mut t, 8, ADAPTER_LOW);
    put32(&mut t, 12, ADAPTER_HIGH as u32);
    video_signal(&mut t, 16, w, h);
    v.push(t);
    let mut s = [0u8; MODE_INFO_SIZE];
    put32(&mut s, 0, MODE_SOURCE);
    put32(&mut s, 4, SOURCE_ID);
    put32(&mut s, 8, ADAPTER_LOW);
    put32(&mut s, 12, ADAPTER_HIGH as u32);
    put32(&mut s, 16, w);
    put32(&mut s, 20, h);
    put32(&mut s, 24, PIXELFORMAT_32BPP);
    v.push(s);
    if flags & QDC_VIRTUAL_MODE_AWARE != 0 {
        let mut d = [0u8; MODE_INFO_SIZE];
        put32(&mut d, 0, MODE_DESKTOP_IMAGE);
        put32(&mut d, 4, TARGET_ID);
        put32(&mut d, 8, ADAPTER_LOW);
        put32(&mut d, 12, ADAPTER_HIGH as u32);
        put32(&mut d, 16, w); // PathSourceSize
        put32(&mut d, 20, h);
        put32(&mut d, 32, w); // DesktopImageRegion: left, top, right, bottom
        put32(&mut d, 36, h);
        put32(&mut d, 40 + 8, w); // DesktopImageClip
        put32(&mut d, 40 + 12, h);
        v.push(d);
    }
    v
}

/// What `QueryDisplayConfig` answers.
#[derive(Debug, PartialEq, Eq)]
pub struct Query {
    /// The return value (0 or an error).
    pub code: i32,
    /// The path count and the mode count to store back (`None`: leave what the caller passed).
    pub counts: Option<(u32, u32)>,
    /// The bytes to write to the path array and to the mode array.
    pub paths: Vec<u8>,
    pub modes: Vec<u8>,
    /// The topology to write (only for `QDC_DATABASE_CURRENT`).
    pub topology: Option<u32>,
}

/// `QueryDisplayConfig` for the monitor `w` x `h`: `np`, `nm` are the sizes of the caller's arrays
/// (in elements), `has_topology` whether a topology pointer was given.
pub fn query(flags: u32, np: u32, nm: u32, has_topology: bool, w: u32, h: u32) -> Query {
    let fail = |code| Query { code, counts: None, paths: Vec::new(), modes: Vec::new(), topology: None };
    let Some(kind) = base(flags) else { return fail(ERROR_INVALID_PARAMETER) };
    if (kind == QDC_DATABASE_CURRENT) != has_topology {
        return fail(ERROR_INVALID_PARAMETER);
    }
    if np == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let all = modes(flags, w, h);
    if (nm as usize) < all.len() {
        // The path array is written; the modes that fit, too.
        let paths = path(flags).to_vec();
        let modes = all.iter().take(nm as usize).flatten().copied().collect();
        return Query { code: ERROR_INSUFFICIENT_BUFFER, counts: None, paths, modes, topology: None };
    }
    Query {
        code: 0,
        counts: Some((1, all.len() as u32)),
        paths: path(flags).to_vec(),
        modes: all.iter().flatten().copied().collect(),
        topology: (kind == QDC_DATABASE_CURRENT).then_some(TOPOLOGY_INTERNAL),
    }
}

// ----- DisplayConfigGetDeviceInfo

const HEADER: usize = 20;
const GET_SOURCE_NAME: u32 = 1;
const GET_TARGET_NAME: u32 = 2;
const GET_TARGET_PREFERRED_MODE: u32 = 3;
const GET_ADAPTER_NAME: u32 = 4;
const GET_TARGET_BASE_TYPE: u32 = 6;
const GET_SUPPORT_VIRTUAL_RESOLUTION: u32 = 7;
const GET_ADVANCED_COLOR_INFO: u32 = 9;
const GET_SDR_WHITE_LEVEL: u32 = 11;

/// The exact size of the structure of a request type, `None` for a type that is not supported.
fn packet_size(kind: u32) -> Option<usize> {
    Some(match kind {
        GET_SOURCE_NAME => 84,
        GET_TARGET_NAME => 420,
        GET_TARGET_PREFERRED_MODE => 80,
        GET_ADAPTER_NAME => 276,
        GET_TARGET_BASE_TYPE => 24,
        GET_SUPPORT_VIRTUAL_RESOLUTION => 24,
        GET_ADVANCED_COLOR_INFO => 32,
        GET_SDR_WHITE_LEVEL => 24,
        _ => return None,
    })
}

fn put_utf16(b: &mut [u8], at: usize, max_chars: usize, s: &str) {
    // The rest of the buffer is zero whatever the caller had in it.
    b[at..at + max_chars * 2].fill(0);
    for (i, c) in s.encode_utf16().take(max_chars - 1).enumerate() {
        b[at + i * 2..at + i * 2 + 2].copy_from_slice(&c.to_le_bytes());
    }
}

/// The GDI device name of the source (`\\.\DISPLAY1` for source 0).
pub fn source_device_name(source: u32) -> alloc::string::String {
    alloc::format!("\\\\.\\DISPLAY{}", source + 1)
}

/// The DPI-scale packets (not on Learn; DisplayConfigGetDeviceInfo.yaml `at_200_dpi_scale`,
/// DisplayConfigSetDeviceInfo.yaml `packets`): get (-3, 32 bytes: minimum, current and maximum
/// step after the header) and set (-4, 24 bytes: the step), for the source of the one path.
pub const GET_DPI_SCALE: u32 = (-3i32) as u32;
pub const SET_DPI_SCALE: u32 = (-4i32) as u32;

fn dpi_scale_header_ok(packet: &[u8], kind: u32, size: usize) -> bool {
    packet.len() == size
        && get32(packet, 0) == kind
        && get32(packet, 8) == ADAPTER_LOW
        && get32(packet, 12) == ADAPTER_HIGH as u32
        && get32(packet, 16) == SOURCE_ID
}

/// The get packet: the steps relative to the recommended scale (step 0), the current `step` and
/// the largest one `max`.
pub fn dpi_scale_get(packet: &mut [u8], step: u32, max: u32) -> i32 {
    if !dpi_scale_header_ok(packet, GET_DPI_SCALE, 32) {
        return ERROR_INVALID_PARAMETER;
    }
    put32(packet, 20, 0);
    put32(packet, 24, step);
    put32(packet, 28, max);
    0
}

/// The step a set packet asks for.
pub fn dpi_scale_set(packet: &[u8]) -> Result<i32, i32> {
    if !dpi_scale_header_ok(packet, SET_DPI_SCALE, 24) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(get32(packet, 20) as i32)
}

/// `DisplayConfigGetDeviceInfo` on `packet` (the caller's whole packet, `packet.len()` the header's
/// size field; `packet` holds at least the 20-byte header). The return value; fills `packet`.
/// A size below the header is the caller's business (31 and the last error 87).
pub fn device_info(packet: &mut [u8], w: u32, h: u32) -> i32 {
    let kind = get32(packet, 0);
    let Some(want) = packet_size(kind) else { return ERROR_INVALID_PARAMETER };
    if packet.len() != want {
        return ERROR_INVALID_PARAMETER;
    }
    let (low, high, id) = (get32(packet, 8), get32(packet, 12), get32(packet, 16));
    let adapter_ok = low == ADAPTER_LOW && high == ADAPTER_HIGH as u32;
    match kind {
        GET_SOURCE_NAME => {
            if !adapter_ok || id != SOURCE_ID {
                return ERROR_GEN_FAILURE;
            }
            put_utf16(packet, HEADER, 32, &source_device_name(id));
        }
        GET_ADVANCED_COLOR_INFO => {
            if !adapter_ok || id != TARGET_ID {
                return ERROR_GEN_FAILURE;
            }
            // Only the four low bits are written: all 0 (SDR); the encoding RGB; 8 bits.
            let v = get32(packet, 20) & !0xF;
            put32(packet, 20, v);
            put32(packet, 24, 0);
            put32(packet, 28, 8);
        }
        GET_TARGET_NAME => {
            if !adapter_ok || id != TARGET_ID {
                return ERROR_INVALID_PARAMETER;
            }
            packet[HEADER..].fill(0);
            put32(packet, 20, 2); // friendlyNameForced: no EDID, no friendly name
            put32(packet, 24, OUTPUT_TECHNOLOGY);
            let path =
                alloc::format!("\\\\?\\DISPLAY#POLY0001#5&UID{TARGET_ID}#{{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}}");
            put_utf16(packet, 164, 128, &path);
        }
        GET_ADAPTER_NAME => {
            if !adapter_ok {
                return ERROR_INVALID_PARAMETER;
            }
            let path = "\\\\?\\PCI#VEN_1234&DEV_1111#3&0&0#{5b45201d-f2f2-4f3b-85bb-30ff1f953599}";
            put_utf16(packet, HEADER, 128, path);
        }
        GET_TARGET_BASE_TYPE => {
            if !adapter_ok || id != TARGET_ID {
                return ERROR_INVALID_PARAMETER;
            }
            put32(packet, 20, OUTPUT_TECHNOLOGY);
        }
        GET_SDR_WHITE_LEVEL => {
            if !adapter_ok || id != TARGET_ID {
                return ERROR_INVALID_PARAMETER;
            }
            put32(packet, 20, 1000); // 80 nits
        }
        GET_SUPPORT_VIRTUAL_RESOLUTION => {
            if !adapter_ok || id != TARGET_ID {
                return ERROR_INVALID_PARAMETER;
            }
            put32(packet, 20, 0);
        }
        GET_TARGET_PREFERRED_MODE => {
            if !adapter_ok || id != TARGET_ID {
                return ERROR_INVALID_PARAMETER;
            }
            put32(packet, 20, w);
            put32(packet, 24, h);
            video_signal(packet, 32, w, h);
        }
        _ => return ERROR_INVALID_PARAMETER,
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(kind: u32, size: usize, id: u32) -> Vec<u8> {
        let mut p = alloc::vec![0u8; size];
        put32(&mut p, 0, kind);
        put32(&mut p, 4, size as u32);
        put32(&mut p, 8, ADAPTER_LOW);
        put32(&mut p, 12, 0);
        put32(&mut p, 16, id);
        p
    }

    #[test]
    fn the_dpi_scale_packets() {
        let mut g = header(GET_DPI_SCALE, 32, SOURCE_ID);
        assert_eq!(dpi_scale_get(&mut g, 4, 6), 0);
        assert_eq!((get32(&g, 20), get32(&g, 24), get32(&g, 28)), (0, 4, 6));
        assert_eq!(dpi_scale_get(&mut header(GET_DPI_SCALE, 28, SOURCE_ID), 4, 6), 87);
        assert_eq!(dpi_scale_get(&mut header(GET_DPI_SCALE, 32, TARGET_ID), 4, 6), 87);
        let mut set = header(SET_DPI_SCALE, 24, SOURCE_ID);
        put32(&mut set, 20, 3);
        assert_eq!(dpi_scale_set(&set), Ok(3));
        assert_eq!(dpi_scale_set(&header(SET_DPI_SCALE, 32, SOURCE_ID)), Err(87));
    }

    /// GetDisplayConfigBufferSizes.yaml: one base flag with the optional ones; two modes per path,
    /// three with the virtual flag.
    #[test]
    fn sizes() {
        assert_eq!(buffer_sizes(2, 1), Ok((1, 2)));
        assert_eq!(buffer_sizes(2 | 0x10, 1), Ok((1, 3)));
        assert_eq!(buffer_sizes(4 | 0x10 | 0x20 | 0x40, 1), Ok((1, 3)));
        assert_eq!(buffer_sizes(1 | 0x20, 1), Ok((1, 2)));
        for bad in [0, 8, 0x80, 3, 5, 6, 7, 0x10, 0x82] {
            assert_eq!(buffer_sizes(bad, 1), Err(87), "flags {bad:#x}");
        }
    }

    /// QueryDisplayConfig.yaml `active_shape`, `mode_index`, `source_mode`, `topology`.
    #[test]
    fn query_shapes() {
        let q = query(QDC_ONLY_ACTIVE_PATHS, 1, 2, false, 1920, 1080);
        assert_eq!((q.code, q.counts, q.topology), (0, Some((1, 2)), None));
        assert_eq!(get32(&q.paths, 68), 1);
        assert_eq!((get32(&q.paths, 12), get32(&q.paths, 32)), (1, 0));
        assert_eq!((get32(&q.modes, 0), get32(&q.modes, MODE_INFO_SIZE)), (MODE_TARGET, MODE_SOURCE));
        assert_eq!((get32(&q.modes, MODE_INFO_SIZE + 16), get32(&q.modes, MODE_INFO_SIZE + 20)), (1920, 1080));
        let q = query(QDC_ONLY_ACTIVE_PATHS | QDC_VIRTUAL_MODE_AWARE, 3, 9, false, 800, 600);
        assert_eq!(q.counts, Some((1, 3)));
        assert_eq!(get32(&q.paths, 68), 9);
        assert_eq!((get32(&q.paths, 12), get32(&q.paths, 32)), (0x0001_FFFF, 2));
        assert_eq!(get32(&q.modes, 2 * MODE_INFO_SIZE), MODE_DESKTOP_IMAGE);
        // Topology: only the database kind, and it needs the pointer.
        let q = query(QDC_DATABASE_CURRENT, 1, 2, true, 1, 1);
        assert_eq!((q.code, q.topology), (0, Some(1)));
        assert_eq!(query(QDC_DATABASE_CURRENT, 1, 2, false, 1, 1).code, 87);
        assert_eq!(query(QDC_ONLY_ACTIVE_PATHS, 1, 2, true, 1, 1).code, 87);
        assert_eq!(query(QDC_ALL_PATHS, 1, 2, true, 1, 1).code, 87);
        // Counts: zero paths 87, a short mode array 122 (the first modes still written).
        assert_eq!(query(QDC_ONLY_ACTIVE_PATHS, 0, 2, false, 1, 1).code, 87);
        let q = query(QDC_ONLY_ACTIVE_PATHS, 1, 1, false, 1, 1);
        assert_eq!((q.code, q.counts, q.modes.len(), q.paths.len()), (122, None, MODE_INFO_SIZE, PATH_INFO_SIZE));
        assert_eq!(query(QDC_ONLY_ACTIVE_PATHS, 1, 0, false, 1, 1).code, 122);
        // The refresh rate of the path is the vertical sync of the target mode.
        let q = query(QDC_ONLY_ACTIVE_PATHS, 1, 2, false, 640, 480);
        assert_eq!((get32(&q.paths, 48), get32(&q.paths, 52)), (get32(&q.modes, 32), get32(&q.modes, 36)));
    }

    /// DisplayConfigGetDeviceInfo.yaml.
    #[test]
    fn device_info_types() {
        let mut p = header(GET_SOURCE_NAME, 84, 0);
        assert_eq!(device_info(&mut p, 1024, 768), 0);
        let name: Vec<u16> =
            p[20..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        assert_eq!(alloc::string::String::from_utf16(&name).unwrap(), "\\\\.\\DISPLAY1");
        assert_eq!(device_info(&mut header(GET_SOURCE_NAME, 84, TARGET_ID), 1, 1), 31);
        assert_eq!(device_info(&mut header(GET_SOURCE_NAME, 80, 0), 1, 1), 87);
        let mut t = header(GET_TARGET_NAME, 420, TARGET_ID);
        assert_eq!(device_info(&mut t, 1, 1), 0);
        assert_eq!(get32(&t, 20), 2);
        assert_eq!(device_info(&mut header(GET_TARGET_NAME, 420, 0), 1, 1), 87);
        let mut a = header(GET_ADVANCED_COLOR_INFO, 32, TARGET_ID);
        put32(&mut a, 20, 0xFFFF_FFFF);
        assert_eq!(device_info(&mut a, 1, 1), 0);
        assert_eq!((get32(&a, 20), get32(&a, 24), get32(&a, 28)), (0xFFFF_FFF0, 0, 8));
        assert_eq!(device_info(&mut header(GET_ADVANCED_COLOR_INFO, 32, 0), 1, 1), 31);
        let mut w = header(GET_SDR_WHITE_LEVEL, 24, TARGET_ID);
        assert_eq!(device_info(&mut w, 1, 1), 0);
        assert_eq!(get32(&w, 20), 1000);
        let mut m = header(GET_TARGET_PREFERRED_MODE, 80, TARGET_ID);
        assert_eq!(device_info(&mut m, 1024, 768), 0);
        assert_eq!((get32(&m, 20), get32(&m, 24)), (1024, 768));
        assert_eq!(device_info(&mut header(GET_SUPPORT_VIRTUAL_RESOLUTION, 28, TARGET_ID), 1, 1), 87);
        for kind in [0, 5, 8, 10, 12, 13, 15, 99, u32::MAX] {
            assert_eq!(device_info(&mut header(kind, 32, TARGET_ID), 1, 1), 87, "type {kind}");
        }
        // The adapter name ignores the id; an unknown adapter is refused.
        let mut a1 = header(GET_ADAPTER_NAME, 276, 0);
        let mut a2 = header(GET_ADAPTER_NAME, 276, 99);
        assert_eq!((device_info(&mut a1, 1, 1), device_info(&mut a2, 1, 1)), (0, 0));
        assert_eq!(a1[20..], a2[20..]);
        let mut bad = header(GET_ADAPTER_NAME, 276, 0);
        put32(&mut bad, 8, 7);
        assert_eq!(device_info(&mut bad, 1, 1), 87);
    }
}
