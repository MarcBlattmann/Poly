//! The `oleaut32.dll` functions on the Chrome installers' path (batch B0 of
//! `docs/windows/chrome-installer-plan.md`) as far as they can be decided without a process:
//! `SystemTimeToVariantTime` (ordinal 184), the name rules of `LoadTypeLib` (ordinal 161) and of
//! `OleLoadPicturePath` (ordinal 424), and the picture formats the last one reads.
//!
//! Sources: the spec sheets `specs/windows/oleaut32.dll/{SystemTimeToVariantTime,LoadTypeLib,
//! OleLoadPicturePath}.yaml` and their visible tests; Learn's pages of those functions,
//! `SYSTEMTIME`, `VT_DATE` ("days since 1899-12-30 as a double, the fraction is the time of
//! day"), `IPicture` and `PICTYPE_*`; `winerror.h` / `urlmon.h` for the `HRESULT`s; the BMP file
//! format (Learn: "Bitmap Storage", `BITMAPFILEHEADER`, `BITMAPINFOHEADER`) and the GIF89a
//! specification (CompuServe, 1990) for the pictures.

use alloc::vec::Vec;

// ----- SystemTimeToVariantTime (ordinal 184) ----------------------------------------------------

/// The fields of a `SYSTEMTIME` (`minwinbase.h`: eight `WORD`s, `wYear`, `wMonth`, `wDayOfWeek`,
/// `wDay`, `wHour`, `wMinute`, `wSecond`, `wMilliseconds`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemTime {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
}

impl SystemTime {
    /// From the eight words in their `SYSTEMTIME` order.
    pub fn from_words(w: [u16; 8]) -> SystemTime {
        SystemTime {
            year: w[0],
            month: w[1],
            day_of_week: w[2],
            day: w[3],
            hour: w[4],
            minute: w[5],
            second: w[6],
            milliseconds: w[7],
        }
    }
}

/// Days from 1970-01-01 to `y`-`m`-01 in the proleptic Gregorian calendar (`m` 1 to 12).
fn days_from_civil(y: i64, m: i64) -> i64 {
    // The year counted from March, so that February is its last month.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Days from the automation epoch 1899-12-30 to 1970-01-01.
const EPOCH_1970: i64 = 25_569;

/// `SystemTimeToVariantTime`: the OLE automation date of a `SYSTEMTIME`, or `None` (the
/// function's 0, the output untouched) for fields out of range.
///
/// - A year above 9999, a month above 12, a day above 31, an hour above 23, a minute or a second
///   above 59: `None`. The day of the week and the milliseconds are not read (1000 included).
/// - A year 0 to 99 is a two-digit year: below 30 in the 2000s, else in the 1900s (the sheet saw
///   0, 50 and 99; where the window turns was not probed).
/// - Month 0 is December of the year before, day 0 the last day of the month before, and a day
///   past the month's end rolls into the next month (February 30 is in March).
/// - The value counts days from 1899-12-30; the time of day is the fraction, taken away from a
///   negative day count (1899-12-29 18:00 is -1.75).
pub fn system_time_to_variant_time(t: &SystemTime) -> Option<f64> {
    if t.year > 9999 || t.month > 12 || t.day > 31 || t.hour > 23 || t.minute > 59 || t.second > 59 {
        return None;
    }
    let mut year = i64::from(t.year);
    if year < 100 {
        year += if year < 30 { 2000 } else { 1900 };
    }
    let (year, month) = if t.month == 0 { (year - 1, 12) } else { (year, i64::from(t.month)) };
    let days = days_from_civil(year, month) + i64::from(t.day) - 1 + EPOCH_1970;
    let seconds = u32::from(t.hour) * 3600 + u32::from(t.minute) * 60 + u32::from(t.second);
    let fraction = f64::from(seconds) / 86_400.0;
    Some(if days >= 0 { days as f64 + fraction } else { days as f64 - fraction })
}

// ----- LoadTypeLib (ordinal 161) ----------------------------------------------------------------

/// `TYPE_E_CANTLOADLIBRARY` (`winerror.h`).
pub const TYPE_E_CANTLOADLIBRARY: u32 = 0x8002_9C4A;
/// `STG_E_FILENOTFOUND` (`winerror.h`).
pub const STG_E_FILENOTFOUND: u32 = 0x8003_0002;

/// What `LoadTypeLib`'s name asks for: a file, and which `TYPELIB` resource of it when the file is
/// a module (`module\N` names resource N; the module alone resource 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeLibName {
    pub file: Vec<u16>,
    /// The resource index the name gave after the file (`None`: none given).
    pub index: Option<u16>,
}

/// Split `LoadTypeLib`'s name: the empty name is `STG_E_FILENOTFOUND`. When the whole name is no
/// file (`exists`) but the part before its last backslash is, the last component is a resource
/// index: decimal digits from 1 up; 0 or anything that is not a number names no resource
/// (`TYPE_E_CANTLOADLIBRARY`). Otherwise the name is the file.
pub fn split_type_lib_name(name: &[u16], exists: impl Fn(&[u16]) -> bool) -> Result<TypeLibName, u32> {
    if name.is_empty() {
        return Err(STG_E_FILENOTFOUND);
    }
    if exists(name) {
        return Ok(TypeLibName { file: name.to_vec(), index: None });
    }
    if let Some(at) = name.iter().rposition(|&c| c == u16::from(b'\\')) {
        let (file, last) = (&name[..at], &name[at + 1..]);
        if !file.is_empty() && exists(file) {
            let digits = !last.is_empty() && last.len() <= 5 && last.iter().all(|&c| (0x30..=0x39).contains(&c));
            let n = digits.then(|| last.iter().fold(0u32, |n, &c| n * 10 + u32::from(c - 0x30)));
            return match n {
                Some(n @ 1..=0xFFFF) => Ok(TypeLibName { file: file.to_vec(), index: Some(n as u16) }),
                _ => Err(TYPE_E_CANTLOADLIBRARY),
            };
        }
    }
    Ok(TypeLibName { file: name.to_vec(), index: None })
}

// ----- OleLoadPicturePath (ordinal 424) ---------------------------------------------------------

/// `INET_E_UNKNOWN_PROTOCOL`, `INET_E_OBJECT_NOT_FOUND`, `INET_E_REDIRECT_TO_DIR` (`winerror.h`).
pub const INET_E_UNKNOWN_PROTOCOL: u32 = 0x800C_000D;
pub const INET_E_OBJECT_NOT_FOUND: u32 = 0x800C_0005;
pub const INET_E_REDIRECT_TO_DIR: u32 = 0x800C_0015;
/// `E_FAIL` (`winerror.h`).
pub const E_FAIL: u32 = 0x8000_4005;
/// `PICTYPE_BITMAP` (`olectl.h`).
pub const PICTYPE_BITMAP: i16 = 1;

/// Whether `OleLoadPicturePath`'s name is a full path it reads as a file: a drive letter, a colon
/// and a backslash (or slash), or a UNC name. Anything else, a relative name and the empty one
/// included, is read as a URL of a protocol nobody handles (`INET_E_UNKNOWN_PROTOCOL`).
pub fn picture_path_is_file(name: &[u16]) -> bool {
    let sep = |c: u16| c == u16::from(b'\\') || c == u16::from(b'/');
    let letter = |c: u16| (u16::from(b'A')..=u16::from(b'Z')).contains(&(c & !0x20));
    match name {
        [d, c, s, ..] if letter(*d) && *c == u16::from(b':') && sep(*s) => true,
        [a, b, rest @ ..] if sep(*a) && sep(*b) && !rest.is_empty() => true,
        _ => false,
    }
}

/// A picture's size in `HIMETRIC` units (0.01 mm) at 96 pixels per inch, rounded down as the sheet
/// shows (1 pixel is 26: 2540 / 96 = 26.46).
pub fn himetric(pixels: u32) -> i32 {
    (u64::from(pixels) * 2540 / 96) as i32
}

/// A decoded picture: its size and its pixels as `0x00RRGGBB`, rows top-down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

/// The largest picture side the decoders accept (a guard against absurd headers).
const MAX_SIDE: u32 = 16_384;

/// Decode a picture file by its content (not its name): a Windows bitmap (`BM`) or a GIF
/// (`GIF87a`/`GIF89a`). `None` for anything else, an empty file included (`E_FAIL`).
pub fn decode_picture(data: &[u8]) -> Option<Picture> {
    if data.starts_with(b"BM") {
        decode_bmp(data)
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        decode_gif(data)
    } else {
        None
    }
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// A `BITMAPFILEHEADER` (14 bytes: `bfType`, `bfSize`, two reserved words, `bfOffBits`) and a
/// `BITMAPINFOHEADER` (`biSize` 40 or more; `biWidth`, `biHeight` negative for top-down,
/// `biPlanes`, `biBitCount`, `biCompression` `BI_RGB` 0) with 1, 4, 8, 24 or 32 bits per pixel;
/// rows are padded to 4 bytes and stored bottom-up.
fn decode_bmp(d: &[u8]) -> Option<Picture> {
    let bits_at = u32_at(d, 10)? as usize;
    let header = u32_at(d, 14)? as usize;
    if header < 40 {
        return None;
    }
    let width = u32_at(d, 18)? as i32;
    let height = u32_at(d, 22)? as i32;
    let bpp = u16_at(d, 28)?;
    let compression = u32_at(d, 30)?;
    if width <= 0 || height == 0 || compression != 0 {
        return None;
    }
    let (w, h) = (width as u32, height.unsigned_abs());
    if w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let palette_at = 14 + header;
    let colours = match bpp {
        1 | 4 | 8 => {
            let used = u32_at(d, 46)?;
            if used == 0 { 1u32 << bpp } else { used.min(1 << bpp) }
        }
        24 | 32 => 0,
        _ => return None,
    };
    let entry = |i: u32| -> u32 {
        let at = palette_at + 4 * i as usize;
        d.get(at..at + 3).map_or(0, |c| u32::from(c[2]) << 16 | u32::from(c[1]) << 8 | u32::from(c[0]))
    };
    let stride = (w as usize * usize::from(bpp)).div_ceil(32) * 4;
    if d.len() < bits_at + stride * h as usize {
        return None;
    }
    let mut pixels = alloc::vec![0u32; (w * h) as usize];
    for row in 0..h as usize {
        let src = bits_at + stride * if height > 0 { h as usize - 1 - row } else { row };
        let line = &d[src..src + stride];
        for x in 0..w as usize {
            let p = match bpp {
                24 => u32::from(line[3 * x + 2]) << 16 | u32::from(line[3 * x + 1]) << 8 | u32::from(line[3 * x]),
                32 => u32::from(line[4 * x + 2]) << 16 | u32::from(line[4 * x + 1]) << 8 | u32::from(line[4 * x]),
                _ => {
                    let bit = x * usize::from(bpp);
                    let byte = line[bit / 8];
                    let shift = 8 - usize::from(bpp) - bit % 8;
                    let index = u32::from(byte >> shift) & ((1 << bpp) - 1);
                    if index < colours { entry(index) } else { 0 }
                }
            };
            pixels[row * w as usize + x] = p;
        }
    }
    Some(Picture { width: w, height: h, pixels })
}

/// A GIF's first image on its logical screen: the screen descriptor (width, height, flags with
/// the global colour table's size, background colour index), extensions skipped, the image
/// descriptor (position, size, flags: local colour table, interlace) and the LZW-coded indices.
/// The screen is filled with the background colour first.
fn decode_gif(d: &[u8]) -> Option<Picture> {
    let w = u32::from(u16_at(d, 6)?);
    let h = u32::from(u16_at(d, 8)?);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let flags = *d.get(10)?;
    let background = *d.get(11)?;
    let mut at = 13;
    let table = |at: usize, size_bits: u8| -> Option<Vec<u32>> {
        let n = 2usize << size_bits;
        let bytes = d.get(at..at + 3 * n)?;
        Some(bytes.chunks_exact(3).map(|c| u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2])).collect())
    };
    let global = if flags & 0x80 != 0 {
        let t = table(at, flags & 7)?;
        at += 3 * t.len();
        Some(t)
    } else {
        None
    };
    let back = global.as_ref().and_then(|g| g.get(usize::from(background)).copied()).unwrap_or(0);
    let mut pixels = alloc::vec![back; (w * h) as usize];
    let mut transparent: Option<u8> = None;
    loop {
        match *d.get(at)? {
            // An extension: label, then sub-blocks up to an empty one. A graphic control
            // extension (0xF9) may name a transparent index.
            0x21 => {
                let label = *d.get(at + 1)?;
                at += 2;
                if label == 0xF9 && *d.get(at)? >= 4 && d.get(at + 1)? & 1 != 0 {
                    transparent = Some(*d.get(at + 4)?);
                }
                at = skip_blocks(d, at)?;
            }
            0x2C => break,
            _ => return None,
        }
    }
    let left = u32::from(u16_at(d, at + 1)?);
    let top = u32::from(u16_at(d, at + 3)?);
    let iw = u32::from(u16_at(d, at + 5)?);
    let ih = u32::from(u16_at(d, at + 7)?);
    let iflags = *d.get(at + 9)?;
    at += 10;
    let local = if iflags & 0x80 != 0 {
        let t = table(at, iflags & 7)?;
        at += 3 * t.len();
        Some(t)
    } else {
        None
    };
    let colours = local.or(global)?;
    let min_code = *d.get(at)?;
    at += 1;
    let mut data = Vec::new();
    loop {
        let n = usize::from(*d.get(at)?);
        at += 1;
        if n == 0 {
            break;
        }
        data.extend_from_slice(d.get(at..at + n)?);
        at += n;
    }
    let indices = lzw_decode(&data, min_code, (iw * ih) as usize)?;
    let rows = interlaced_rows(ih, iflags & 0x40 != 0);
    for (i, &index) in indices.iter().enumerate() {
        let (row, col) = (rows[i / iw as usize], i as u32 % iw);
        let (x, y) = (left + col, top + row);
        if x >= w || y >= h || Some(index) == transparent {
            continue;
        }
        if let Some(&c) = colours.get(usize::from(index)) {
            pixels[(y * w + x) as usize] = c;
        }
    }
    Some(Picture { width: w, height: h, pixels })
}

/// Past the data sub-blocks starting at `at` (each a length byte and that many bytes; an empty one
/// ends them).
fn skip_blocks(d: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let n = usize::from(*d.get(at)?);
        at += 1 + n;
        if n == 0 {
            return Some(at);
        }
    }
}

/// The row each stored row of an image of `h` rows goes to: in order, or GIF's four interlace
/// passes (every 8th row from 0, every 8th from 4, every 4th from 2, every 2nd from 1).
fn interlaced_rows(h: u32, interlaced: bool) -> Vec<u32> {
    if !interlaced {
        return (0..h).collect();
    }
    let mut rows = Vec::with_capacity(h as usize);
    for (start, step) in [(0, 8), (4, 8), (2, 4), (1, 2)] {
        rows.extend((start..h).step_by(step));
    }
    rows
}

/// GIF's variable-length LZW (the GIF89a specification, appendix F): codes of `min + 1` to 12
/// bits, least significant bit first, a clear code `1 << min` and an end code after it. Up to
/// `want` indices.
fn lzw_decode(data: &[u8], min: u8, want: usize) -> Option<Vec<u8>> {
    if !(1..=11).contains(&min) {
        return None;
    }
    let clear = 1u16 << min;
    let end = clear + 1;
    // Each entry: (prefix code, last byte, length); codes below `clear` are single bytes.
    let mut table: Vec<(u16, u8, u16)> = (0..clear).map(|c| (u16::MAX, c as u8, 1)).collect();
    table.push((u16::MAX, 0, 0));
    table.push((u16::MAX, 0, 0));
    let mut size = u32::from(min) + 1;
    let (mut bits, mut nbits, mut pos) = (0u32, 0u32, 0usize);
    let mut prev: Option<u16> = None;
    let mut out = Vec::with_capacity(want);
    let expand = |table: &[(u16, u8, u16)], code: u16, out: &mut Vec<u8>| -> u8 {
        let len = usize::from(table[usize::from(code)].2);
        let start = out.len();
        out.resize(start + len, 0);
        let mut c = code;
        for i in (0..len).rev() {
            let (p, b, _) = table[usize::from(c)];
            out[start + i] = b;
            c = p;
        }
        out[start]
    };
    while out.len() < want {
        while nbits < size {
            let Some(&b) = data.get(pos) else { return (!out.is_empty()).then_some(out) };
            bits |= u32::from(b) << nbits;
            nbits += 8;
            pos += 1;
        }
        let code = (bits & ((1 << size) - 1)) as u16;
        bits >>= size;
        nbits -= size;
        if code == clear {
            table.truncate(usize::from(end) + 1);
            size = u32::from(min) + 1;
            prev = None;
            continue;
        }
        if code == end {
            break;
        }
        match prev {
            None => {
                if code >= clear {
                    return None;
                }
                expand(&table, code, &mut out);
            }
            Some(p) => {
                let first = if usize::from(code) < table.len() {
                    expand(&table, code, &mut out)
                } else if usize::from(code) == table.len() {
                    // The code being defined: the previous string and its own first byte.
                    let f = expand(&table, p, &mut out);
                    out.push(f);
                    f
                } else {
                    return None;
                };
                if table.len() < 4096 {
                    let len = table[usize::from(p)].2 + 1;
                    table.push((p, first, len));
                    if table.len() == 1 << size && size < 12 {
                        size += 1;
                    }
                }
            }
        }
        prev = Some(code);
    }
    out.truncate(want);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn st(year: u16, month: u16, day: u16, hour: u16, minute: u16, second: u16) -> SystemTime {
        SystemTime { year, month, day, hour, minute, second, ..Default::default() }
    }

    #[test]
    fn variant_times_of_the_sheet() {
        let v = |t: SystemTime| system_time_to_variant_time(&t);
        assert_eq!(v(st(1899, 12, 30, 0, 0, 0)), Some(0.0));
        assert_eq!(v(st(1899, 12, 31, 0, 0, 0)), Some(1.0));
        assert_eq!(v(st(1900, 1, 1, 0, 0, 0)), Some(2.0));
        assert_eq!(v(st(1900, 2, 28, 0, 0, 0)), Some(60.0));
        assert_eq!(v(st(1900, 3, 1, 0, 0, 0)), Some(61.0));
        assert_eq!(v(st(1970, 1, 1, 0, 0, 0)), Some(25569.0));
        assert_eq!(v(st(2000, 1, 1, 0, 0, 0)), Some(36526.0));
        assert_eq!(v(st(2000, 2, 29, 12, 0, 0)), Some(36585.5));
        assert_eq!(v(st(1899, 12, 29, 18, 0, 0)), Some(-1.75));
        assert_eq!(v(st(100, 1, 1, 0, 0, 0)).map(f64::to_bits), Some(0xc124_1034_0000_0000));
        assert_eq!(v(st(9999, 12, 31, 23, 59, 59)).map(f64::to_bits), Some(0x4146_9240_ffff_9ee9));
        assert_eq!(v(st(99, 6, 1, 0, 0, 0)), Some(36312.0));
        assert_eq!(v(st(50, 1, 1, 0, 0, 0)), Some(18264.0));
        assert_eq!(v(st(0, 1, 1, 0, 0, 0)), Some(36526.0));
        assert_eq!(v(st(2000, 0, 1, 0, 0, 0)), Some(36495.0));
        assert_eq!(v(st(2000, 1, 0, 0, 0, 0)), Some(36525.0));
        assert_eq!(v(st(2000, 2, 30, 0, 0, 0)), Some(36586.0));
        assert_eq!(v(st(2001, 2, 29, 0, 0, 0)), Some(36951.0));
        for bad in [
            st(10000, 1, 1, 0, 0, 0),
            st(2000, 13, 1, 0, 0, 0),
            st(2000, 1, 32, 0, 0, 0),
            st(2000, 1, 1, 24, 0, 0),
            st(2000, 1, 1, 0, 60, 0),
            st(2000, 1, 1, 0, 0, 60),
        ] {
            assert_eq!(v(bad), None, "{bad:?}");
        }
        let mut t = st(2000, 1, 1, 0, 0, 0);
        t.milliseconds = 1000;
        t.day_of_week = 3;
        assert_eq!(v(t), Some(36526.0));
        assert!((v(st(2026, 11, 17, 6, 30, 15)).unwrap() - 46_343.271_007).abs() < 1e-6);
    }

    #[test]
    fn type_library_names() {
        let lib = w("C:\\x\\scrrun.dll");
        let exists = |n: &[u16]| n == lib.as_slice();
        assert_eq!(split_type_lib_name(&[], exists), Err(STG_E_FILENOTFOUND));
        assert_eq!(split_type_lib_name(&lib, exists).unwrap().index, None);
        let n = split_type_lib_name(&w("C:\\x\\scrrun.dll\\1"), exists).unwrap();
        assert_eq!((n.file, n.index), (lib.clone(), Some(1)));
        assert_eq!(split_type_lib_name(&w("C:\\x\\scrrun.dll\\0"), exists), Err(TYPE_E_CANTLOADLIBRARY));
        assert_eq!(split_type_lib_name(&w("C:\\x\\scrrun.dll\\x"), exists), Err(TYPE_E_CANTLOADLIBRARY));
        assert_eq!(split_type_lib_name(&w("C:\\y\\a.tlb"), exists).unwrap().index, None);
    }

    #[test]
    fn picture_paths_and_sizes() {
        assert!(picture_path_is_file(&w("C:\\dir\\a.bmp")));
        assert!(picture_path_is_file(&w("c:/a.bmp")));
        assert!(picture_path_is_file(&w("\\\\server\\share\\a.bmp")));
        assert!(!picture_path_is_file(&w("poly-pic.bmp")));
        assert!(!picture_path_is_file(&w("")));
        assert!(!picture_path_is_file(&w("C:a.bmp")));
        assert_eq!(himetric(1), 26);
        assert_eq!(himetric(96), 2540);
    }

    /// The 58-byte 1x1 24-bit bitmap of the sheet (a red pixel).
    fn bmp_1x1() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&58u32.to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&54u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&[0; 24]);
        b.extend_from_slice(&[0, 0, 0xFF, 0]);
        b
    }

    #[test]
    fn a_one_pixel_bitmap() {
        let p = decode_picture(&bmp_1x1()).unwrap();
        assert_eq!((p.width, p.height, p.pixels), (1, 1, alloc::vec![0xFF0000]));
        assert_eq!(decode_picture(b""), None);
        assert_eq!(decode_picture(b"this is not a picture"), None);
        assert_eq!(decode_picture(&bmp_1x1()[..40]), None);
    }

    #[test]
    fn a_one_pixel_gif() {
        // GIF89a, 1x1, a global table of 2 colours (red, white), one image with the LZW codes
        // clear (4), index 0, end (5) in 3-bit codes: 0b101_000_100 = 0x44, 0x01.
        let mut g = Vec::new();
        g.extend_from_slice(b"GIF89a");
        g.extend_from_slice(&[1, 0, 1, 0, 0x80, 0, 0]);
        g.extend_from_slice(&[0xFF, 0, 0, 0xFF, 0xFF, 0xFF]);
        g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
        g.extend_from_slice(&[2, 2, 0x44, 0x01, 0, 0x3B]);
        assert_eq!(g.len(), 35);
        let p = decode_picture(&g).unwrap();
        assert_eq!((p.width, p.height, p.pixels), (1, 1, alloc::vec![0xFF0000]));
    }

    #[test]
    fn lzw_with_a_code_defined_by_itself() {
        // Indices 0 0 0 with min 2: clear(4) 0 6 end(5): code 6 is "0 0", the case where a code
        // names the entry being made.
        let codes = [(4u32, 3u32), (0, 3), (6, 3), (5, 3)];
        let (mut acc, mut n, mut bytes) = (0u32, 0u32, Vec::new());
        for (c, size) in codes {
            acc |= c << n;
            n += size;
            while n >= 8 {
                bytes.push(acc as u8);
                acc >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            bytes.push(acc as u8);
        }
        assert_eq!(lzw_decode(&bytes, 2, 3), Some(alloc::vec![0, 0, 0]));
    }

    #[test]
    fn interlace_passes() {
        assert_eq!(interlaced_rows(10, true), alloc::vec![0, 8, 4, 2, 6, 1, 3, 5, 7, 9]);
        assert_eq!(interlaced_rows(3, false), alloc::vec![0, 1, 2]);
    }
}
