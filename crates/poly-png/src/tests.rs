//! The decoder's tests: handcrafted files, files made by a small encoder
//! written here from the specification (every colour type, depth, filter
//! and interlace method), Debian's Chromium icons, and damaged files.

use super::*;
use alloc::string::String;
use alloc::vec;
use std::format;

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = Vec::new();
    c.extend_from_slice(&(data.len() as u32).to_be_bytes());
    c.extend_from_slice(kind);
    c.extend_from_slice(data);
    c.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
    c
}

fn ihdr(w: u32, h: u32, depth: u8, ct: u8, interlace: u8) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&w.to_be_bytes());
    d.extend_from_slice(&h.to_be_bytes());
    d.extend_from_slice(&[depth, ct, 0, 0, interlace]);
    chunk(b"IHDR", &d)
}

/// A zlib stream of stored blocks of at most `block` bytes.
fn zlib_stored(raw: &[u8], block: usize) -> Vec<u8> {
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(block).peekable();
    if blocks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(b) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&inflate::adler32(raw).to_be_bytes());
    z
}

fn file(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut f = SIGNATURE.to_vec();
    for c in chunks {
        f.extend_from_slice(c);
    }
    f
}

#[test]
fn checksums() {
    // The CRC of "IEND" is the same in every PNG file's last chunk.
    assert_eq!(crc32(&[b"IEND"]), 0xae42_6082);
    assert_eq!(crc32(&[b"IE", b"ND"]), 0xae42_6082, "in parts");
    assert_eq!(crc32(&[b"123456789"]), 0xcbf4_3926, "the standard check value");
}

// ---- Handcrafted files (made with Python's struct and zlib) -------------

#[test]
fn a_one_pixel_file() {
    // 1 x 1, RGBA, one red pixel at half opacity.
    let png = unhex(
        "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d49444154789c63f8cfc0d00000048101802c55ceb00000000049454e44ae426082",
    );
    let h = info(&png).unwrap();
    assert_eq!((h.width, h.height, h.bit_depth, h.color_type, h.interlaced), (1, 1, 8, ColorType::Rgba, false));
    let img = decode(&png).unwrap();
    assert_eq!((img.width, img.height), (1, 1));
    assert_eq!(img.pixels, [0x80ff_0000]);
}

#[test]
fn a_palette_with_transparency() {
    // 2 x 2, palette, 2 bits: red, green, blue, white; tRNS makes the
    // first entry transparent and the second half opaque.
    let png = unhex(
        "89504e470d0a1a0a0000000d49484452000000020000000202030000000fd8e5b70000000c504c5445ff000000ff000000fffffffffb0060f60000000274524e5300809b2b4e180000000c49444154789c631060d8000000e400c127a8e8570000000049454e44ae426082",
    );
    let img = decode(&png).unwrap();
    assert_eq!(img.pixels, [0x00ff_0000, 0x8000_ff00, 0xff00_00ff, 0xffff_ffff]);
}

// ---- A small encoder, for round trips -----------------------------------

/// A deterministic stream of numbers.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
}

#[derive(Clone, Copy, Debug)]
enum FilterChoice {
    /// The same filter (0 to 4) on every row.
    Fixed(u8),
    /// A different one on each row.
    Cycle,
}

/// What to encode: the samples of every pixel (each below 2^depth), and
/// the extra chunks.
struct Spec {
    w: usize,
    h: usize,
    depth: u8,
    ct: u8,
    interlace: bool,
    filter: FilterChoice,
    /// Samples, `channels` per pixel, rows top to bottom.
    samples: Vec<u16>,
    palette: Vec<[u8; 3]>,
    trns: Option<Vec<u8>>,
    idat_size: usize,
}

fn channels(ct: u8) -> usize {
    match ct {
        0 | 3 => 1,
        4 => 2,
        2 => 3,
        _ => 4,
    }
}

/// Pack one row's samples at `depth` bits each, most significant first.
fn pack_row(samples: &[u16], depth: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut n = 0u32;
    for &s in samples {
        if depth == 16 {
            out.extend_from_slice(&s.to_be_bytes());
            continue;
        }
        acc = (acc << depth) | u32::from(s);
        n += u32::from(depth);
        if n == 8 {
            out.push(acc as u8);
            acc = 0;
            n = 0;
        }
    }
    if n > 0 {
        out.push((acc << (8 - n)) as u8);
    }
    out
}

/// The Paeth predictor as the specification writes it.
fn paeth_ref(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Filter `rows` (each `rb` bytes) forwards, `bpp` bytes per pixel.
fn filter_rows(rows: &[Vec<u8>], bpp: usize, choice: FilterChoice) -> Vec<u8> {
    let mut out = Vec::new();
    let zero = vec![0u8; rows.first().map_or(0, Vec::len)];
    for (r, row) in rows.iter().enumerate() {
        let f = match choice {
            FilterChoice::Fixed(f) => f,
            FilterChoice::Cycle => (r % 5) as u8,
        };
        let prev = if r == 0 { &zero } else { &rows[r - 1] };
        out.push(f);
        for i in 0..row.len() {
            let a = if i >= bpp { i32::from(row[i - bpp]) } else { 0 };
            let b = i32::from(prev[i]);
            let c = if i >= bpp { i32::from(prev[i - bpp]) } else { 0 };
            let x = i32::from(row[i]);
            let predicted = match f {
                0 => 0,
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                _ => paeth_ref(a, b, c),
            };
            out.push((x - predicted) as u8);
        }
    }
    out
}

fn encode(s: &Spec) -> Vec<u8> {
    let ch = channels(s.ct);
    let bpp = (ch * usize::from(s.depth)).div_ceil(8).max(1);
    let mut raw = Vec::new();
    let sub_image = |x0: usize, y0: usize, dx: usize, dy: usize| -> Vec<Vec<u8>> {
        let mut rows = Vec::new();
        let mut y = y0;
        while y < s.h {
            let mut row = Vec::new();
            let mut x = x0;
            while x < s.w {
                row.extend_from_slice(&s.samples[(y * s.w + x) * ch..(y * s.w + x + 1) * ch]);
                x += dx;
            }
            if !row.is_empty() {
                rows.push(pack_row(&row, s.depth));
            }
            y += dy;
        }
        rows
    };
    if s.interlace {
        for (x0, y0, dx, dy) in
            [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]
        {
            // A pass with no pixels has no rows at all.
            raw.extend(filter_rows(&sub_image(x0, y0, dx, dy), bpp, s.filter));
        }
    } else {
        raw.extend(filter_rows(&sub_image(0, 0, 1, 1), bpp, s.filter));
    }
    let z = zlib_stored(&raw, 1000);
    let mut chunks = vec![ihdr(s.w as u32, s.h as u32, s.depth, s.ct, u8::from(s.interlace))];
    if s.ct == 3 {
        let plte: Vec<u8> = s.palette.iter().flatten().copied().collect();
        chunks.push(chunk(b"PLTE", &plte));
    }
    if let Some(t) = &s.trns {
        chunks.push(chunk(b"tRNS", t));
    }
    chunks.push(chunk(b"gAMA", &[0, 1, 0x86, 0xa0]));
    for part in z.chunks(s.idat_size) {
        chunks.push(chunk(b"IDAT", part));
    }
    chunks.push(chunk(b"IEND", &[]));
    file(&chunks)
}

/// What the decoder should say, worked out here from the samples.
fn expected(s: &Spec) -> Vec<u32> {
    let ch = channels(s.ct);
    let max = (1u32 << s.depth) - 1;
    // A sample as 8 bits.
    let to8 = |v: u16| -> u32 { (f64::from(v) * 255.0 / f64::from(max)).round() as u32 };
    let mut out = Vec::new();
    for p in s.samples.chunks(ch) {
        let (r, g, b, a) = match s.ct {
            0 => {
                let key = s.trns.as_ref().map(|t| u16::from_be_bytes([t[0], t[1]]));
                (to8(p[0]), to8(p[0]), to8(p[0]), if key == Some(p[0]) { 0 } else { 255 })
            }
            2 => {
                let key = s.trns.as_ref().map(|t| [0, 2, 4].map(|i| u16::from_be_bytes([t[i], t[i + 1]])));
                (to8(p[0]), to8(p[1]), to8(p[2]), if key == Some([p[0], p[1], p[2]]) { 0 } else { 255 })
            }
            3 => {
                let [r, g, b] = s.palette[usize::from(p[0])];
                let a = s.trns.as_ref().and_then(|t| t.get(usize::from(p[0]))).copied().unwrap_or(255);
                (u32::from(r), u32::from(g), u32::from(b), u32::from(a))
            }
            4 => (to8(p[0]), to8(p[0]), to8(p[0]), to8(p[1])),
            _ => (to8(p[0]), to8(p[1]), to8(p[2]), to8(p[3])),
        };
        out.push((a << 24) | (r << 16) | (g << 8) | b);
    }
    out
}

fn spec(w: usize, h: usize, depth: u8, ct: u8, interlace: bool, filter: FilterChoice, seed: u64) -> Spec {
    let mut rng = Rng(seed ^ ((w as u64) << 20) ^ ((h as u64) << 32) ^ (u64::from(depth) << 8) ^ u64::from(ct));
    let max = (1u32 << depth) - 1;
    let ch = channels(ct);
    let palette_len = if ct == 3 { (1usize << depth).min(256).min(5 + w) } else { 0 };
    let mut samples: Vec<u16> = (0..w * h * ch).map(|_| (rng.next() % (max + 1)) as u16).collect();
    // Some runs, so that every filter has something to predict.
    for i in (0..samples.len()).step_by(7) {
        if i > 0 {
            samples[i] = samples[i - 1];
        }
    }
    let palette: Vec<[u8; 3]> =
        (0..palette_len).map(|_| [rng.next() as u8, rng.next() as u8, rng.next() as u8]).collect();
    if ct == 3 {
        for s in &mut samples {
            *s %= palette_len as u16;
        }
    }
    // Transparency where the type has a tRNS: the first pixel's colour (or
    // palette entries' alphas).
    let trns = match ct {
        0 => Some(samples[0].to_be_bytes().to_vec()),
        2 => Some(samples[..3].iter().flat_map(|v| v.to_be_bytes()).collect()),
        3 => Some((0..palette_len.saturating_sub(1)).map(|_| rng.next() as u8).collect()),
        _ => None,
    };
    Spec { w, h, depth, ct, interlace, filter, samples, palette, trns, idat_size: 37 }
}

const KINDS: [(u8, &[u8]); 5] =
    [(0, &[1, 2, 4, 8, 16]), (2, &[8, 16]), (3, &[1, 2, 4, 8]), (4, &[8, 16]), (6, &[8, 16])];

#[test]
fn every_colour_type_depth_filter_and_interlace_round_trips() {
    let sizes = [(1, 1), (2, 3), (3, 5), (8, 8), (9, 9), (17, 11), (33, 1), (1, 33), (5, 20)];
    let mut files = 0;
    for (ct, depths) in KINDS {
        for &depth in depths {
            for interlace in [false, true] {
                for filter in [
                    FilterChoice::Fixed(0),
                    FilterChoice::Fixed(1),
                    FilterChoice::Fixed(2),
                    FilterChoice::Fixed(3),
                    FilterChoice::Fixed(4),
                    FilterChoice::Cycle,
                ] {
                    for &(w, h) in &sizes {
                        let s = spec(w, h, depth, ct, interlace, filter, 7);
                        let png = encode(&s);
                        let what = format!("ct {ct} depth {depth} interlace {interlace} {filter:?} {w}x{h}");
                        let img = decode(&png).unwrap_or_else(|e| panic!("{what}: {e}"));
                        assert_eq!((img.width as usize, img.height as usize), (w, h), "{what}");
                        assert_eq!(img.pixels, expected(&s), "{what}");
                        files += 1;
                    }
                }
            }
        }
    }
    // 15 colour type and depth pairs, 2 interlace methods, 6 filter choices, 9 sizes.
    assert_eq!(files, 15 * 2 * 6 * 9);
}

#[test]
fn a_tiny_adam7_image_skips_empty_passes() {
    // 1 x 1: only the first of the seven passes has a pixel, so the
    // stream has one filter byte and one pixel.
    let s = spec(1, 1, 8, 6, true, FilterChoice::Fixed(0), 1);
    let png = encode(&s);
    let img = decode(&png).unwrap();
    assert_eq!(img.pixels, expected(&s));
    // 3 x 3: several of the passes are empty or have one pixel.
    let s = spec(3, 3, 8, 2, true, FilterChoice::Cycle, 2);
    assert_eq!(decode(&encode(&s)).unwrap().pixels, expected(&s));
}

#[test]
fn sixteen_bit_samples_scale_to_eight() {
    // 0 -> 0, 65535 -> 255, 32768 -> 128, 0x0100 -> 1.
    let mut s = spec(4, 1, 16, 0, false, FilterChoice::Fixed(0), 1);
    s.samples = vec![0, 65535, 32768, 0x0100];
    s.trns = None;
    let img = decode(&encode(&s)).unwrap();
    assert_eq!(img.pixels, [0xff00_0000, 0xffff_ffff, 0xff80_8080, 0xff01_0101]);
}

#[test]
fn low_depth_greys_use_the_full_range() {
    for (depth, max) in [(1u8, 1u16), (2, 3), (4, 15)] {
        let mut s = spec(2, 1, depth, 0, false, FilterChoice::Fixed(0), 1);
        s.samples = vec![0, max];
        s.trns = None;
        let img = decode(&encode(&s)).unwrap();
        assert_eq!(img.pixels, [0xff00_0000, 0xffff_ffff], "depth {depth}");
    }
}

#[test]
fn transparent_colour_keys() {
    // Grey: the sample equal to tRNS is transparent. RGB: all three.
    let mut s = spec(3, 1, 8, 0, false, FilterChoice::Fixed(0), 1);
    s.samples = vec![10, 20, 10];
    s.trns = Some(vec![0, 10]);
    assert_eq!(decode(&encode(&s)).unwrap().pixels, [0x000a_0a0a, 0xff14_1414, 0x000a_0a0a]);
    let mut s = spec(2, 1, 8, 2, false, FilterChoice::Fixed(0), 1);
    s.samples = vec![1, 2, 3, 1, 2, 4];
    s.trns = Some(vec![0, 1, 0, 2, 0, 3]);
    assert_eq!(decode(&encode(&s)).unwrap().pixels, [0x0001_0203, 0xff01_0204]);
}

#[test]
fn a_big_image_in_many_idats() {
    let s = spec(300, 200, 8, 6, true, FilterChoice::Cycle, 5);
    let mut s = s;
    s.idat_size = 4999;
    let png = encode(&s);
    assert_eq!(decode(&png).unwrap().pixels, expected(&s));
}

// ---- Debian's Chromium icons ---------------------------------------------

/// The CRC-32 of an image's pixels as RGBA bytes, which Python's
/// `zlib.crc32` of an independent reference decoding gave.
fn rgba_crc(img: &Image) -> u32 {
    let bytes: Vec<u8> =
        img.pixels.iter().flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]).collect();
    crc32(&[&bytes])
}

#[test]
fn chromiums_icons() {
    let icons: [(&[u8], u32, u32, u32); 3] = [
        (include_bytes!("../tests/data/chromium-16.png"), 16, 0xbec5_5835, 0xff4b_8bf5),
        (include_bytes!("../tests/data/chromium-48.png"), 48, 0x554b_0c2f, 0xff1a_73e8),
        (include_bytes!("../tests/data/chromium-256.png"), 256, 0x8c98_cbe9, 0xff1a_73e8),
    ];
    for (data, size, crc, centre) in icons {
        let h = info(data).unwrap();
        assert_eq!((h.width, h.height, h.bit_depth, h.color_type), (size, size, 8, ColorType::Rgba));
        let img = decode(data).unwrap();
        assert_eq!(rgba_crc(&img), crc, "{size} px: pixels differ from the reference decoding");
        // The corner is transparent; the middle is the logo's colour.
        assert_eq!(img.pixels[0] >> 24, 0, "{size} px: corner");
        let mid = img.pixels[((size / 2) * size + size / 2) as usize];
        assert_eq!(mid, centre, "{size} px: centre");
    }
}

#[test]
fn every_prefix_of_a_real_icon_is_an_error() {
    let data = include_bytes!("../tests/data/chromium-16.png");
    for n in 0..data.len() {
        assert!(decode(&data[..n]).is_err(), "prefix of {n} bytes");
    }
    assert!(decode(data).is_ok());
}

#[test]
fn flipping_any_bit_of_a_real_icon_never_panics() {
    let data = include_bytes!("../tests/data/chromium-16.png");
    let good = decode(data).unwrap();
    let mut errors = 0;
    for i in 0..data.len() * 8 {
        let mut d = data.to_vec();
        d[i / 8] ^= 1 << (i % 8);
        match decode(&d) {
            Err(_) => errors += 1,
            // Only an ancillary chunk's bits can go unnoticed.
            Ok(img) => assert_eq!(img, good),
        }
    }
    assert!(errors > data.len() * 6, "the checksums catch nearly every flip ({errors})");
}

// ---- Damaged files ---------------------------------------------------------

fn small() -> Spec {
    spec(5, 4, 8, 6, false, FilterChoice::Fixed(2), 3)
}

#[test]
fn not_a_png() {
    assert_eq!(decode(b""), Err(Error::NotPng));
    assert_eq!(decode(b"GIF89a"), Err(Error::NotPng));
    assert_eq!(decode(&SIGNATURE[..4]), Err(Error::NotPng));
    assert_eq!(info(b"\x89PNG\r\n"), Err(Error::NotPng));
    // Only the signature.
    assert_eq!(decode(&SIGNATURE), Err(Error::Truncated));
    // A JPEG.
    assert_eq!(decode(&[0xff, 0xd8, 0xff, 0xe0, 0, 16, b'J', b'F', b'I', b'F']), Err(Error::NotPng));
}

#[test]
fn cut_short_files() {
    let png = encode(&small());
    for n in 8..png.len() {
        let e = decode(&png[..n]).unwrap_err();
        assert!(matches!(e, Error::Truncated | Error::BadHeader), "prefix of {n}: {e:?}");
    }
    // Without IEND (the last 12 bytes).
    assert_eq!(decode(&png[..png.len() - 12]), Err(Error::Truncated));
    // A chunk that claims more than the file has.
    let mut big = png.clone();
    big[8..12].copy_from_slice(&0x7fff_ffffu32.to_be_bytes());
    assert!(decode(&big).is_err());
    big[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(decode(&big).is_err());
}

#[test]
fn bad_checksums() {
    let png = encode(&spec(4, 4, 8, 3, false, FilterChoice::Fixed(0), 3));
    // Find the chunks and damage one byte of each critical one's data.
    let mut at = 8;
    let mut seen = 0;
    while at < png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = png[at + 4..at + 8].try_into().unwrap();
        let mut bad = png.clone();
        // The data's first byte (an empty chunk's: its CRC's).
        bad[at + 8] ^= 0x55;
        let r = decode(&bad);
        match &kind {
            b"gAMA" | b"tRNS" => {
                // Ancillary: ignored (tRNS then means opaque).
                assert!(r.is_ok(), "{}: {r:?}", String::from_utf8_lossy(&kind));
            }
            _ => {
                assert_eq!(r, Err(Error::BadCrc), "{}", String::from_utf8_lossy(&kind));
                seen += 1;
            }
        }
        at += 12 + len;
    }
    assert_eq!(seen, 4, "IHDR, PLTE, IDAT and IEND");
}

#[test]
fn a_bad_ancillary_crc_drops_the_chunk() {
    // tRNS with a bad CRC: the palette stays opaque.
    let mut s = spec(2, 1, 8, 3, false, FilterChoice::Fixed(0), 3);
    s.palette = vec![[1, 2, 3], [4, 5, 6]];
    s.samples = vec![0, 1];
    s.trns = Some(vec![0, 0]);
    let mut png = encode(&s);
    let at = png.windows(4).position(|w| w == b"tRNS").unwrap();
    png[at + 4] ^= 1;
    assert_eq!(decode(&png).unwrap().pixels, [0xff01_0203, 0xff04_0506]);
}

#[test]
fn header_checks() {
    let idat = chunk(b"IDAT", &zlib_stored(&[0, 0], 10));
    let iend = chunk(b"IEND", &[]);
    let with = |h: Vec<u8>| decode(&file(&[h, idat.clone(), iend.clone()]));
    assert_eq!(with(ihdr(0, 1, 8, 0, 0)), Err(Error::BadHeader), "no width");
    assert_eq!(with(ihdr(1, 0, 8, 0, 0)), Err(Error::BadHeader), "no height");
    assert_eq!(with(ihdr(1, 1, 8, 1, 0)), Err(Error::BadHeader), "colour type 1");
    assert_eq!(with(ihdr(1, 1, 8, 5, 0)), Err(Error::BadHeader), "colour type 5");
    assert_eq!(with(ihdr(1, 1, 3, 0, 0)), Err(Error::BadHeader), "depth 3");
    assert_eq!(with(ihdr(1, 1, 16, 3, 0)), Err(Error::BadHeader), "16-bit palette");
    assert_eq!(with(ihdr(1, 1, 4, 2, 0)), Err(Error::BadHeader), "4-bit RGB");
    assert_eq!(with(ihdr(1, 1, 1, 6, 0)), Err(Error::BadHeader), "1-bit RGBA");
    assert_eq!(with(ihdr(1, 1, 8, 0, 2)), Err(Error::BadHeader), "interlace method 2");
    assert_eq!(with(ihdr(0x8000_0000, 1, 8, 0, 0)), Err(Error::BadHeader), "width 2^31");
    // Compression and filter methods other than 0.
    let mut d = Vec::new();
    d.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 1, 0, 0]);
    assert_eq!(with(chunk(b"IHDR", &d)), Err(Error::BadHeader));
    d[10] = 0;
    d[11] = 1;
    assert_eq!(with(chunk(b"IHDR", &d)), Err(Error::BadHeader));
    // Wrong length, and not first.
    assert_eq!(with(chunk(b"IHDR", &d[..12])), Err(Error::BadHeader));
    assert_eq!(decode(&file(&[iend.clone(), ihdr(1, 1, 8, 0, 0)])), Err(Error::BadHeader));
}

#[test]
fn huge_images_are_refused_before_anything_is_allocated() {
    let iend = chunk(b"IEND", &[]);
    let big = file(&[ihdr(0x7fff_ffff, 0x7fff_ffff, 16, 6, 1), iend.clone()]);
    assert_eq!(decode(&big), Err(Error::TooLarge));
    assert_eq!(decode(&file(&[ihdr(65_535, 65_535, 8, 6, 0), iend.clone()])), Err(Error::TooLarge));
    // The limit is the caller's.
    let s = spec(20, 20, 8, 6, false, FilterChoice::Fixed(0), 1);
    let png = encode(&s);
    assert!(decode_with(&png, &Limits { max_pixels: 400 }).is_ok());
    assert_eq!(decode_with(&png, &Limits { max_pixels: 399 }), Err(Error::TooLarge));
}

#[test]
fn chunk_order_and_kinds() {
    let s = spec(2, 2, 8, 6, false, FilterChoice::Fixed(0), 1);
    let mut s = s;
    s.idat_size = 10;
    let png = encode(&s);
    let parts: Vec<Vec<u8>> = {
        let mut v = Vec::new();
        let mut at = 8;
        while at < png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            v.push(png[at..at + 12 + len].to_vec());
            at += 12 + len;
        }
        v
    };
    // IHDR, gAMA, IDAT..., IEND.
    assert!(parts.len() >= 4);
    let idats: Vec<Vec<u8>> = parts.iter().filter(|p| &p[4..8] == b"IDAT").cloned().collect();
    assert!(idats.len() >= 2, "the file has several IDATs");
    let (ihdr_c, iend_c) = (parts[0].clone(), parts[parts.len() - 1].clone());
    let text = chunk(b"tEXt", b"Comment\0hello");
    // A second IHDR.
    let mut two = vec![ihdr_c.clone(), ihdr_c.clone()];
    two.extend(idats.clone());
    two.push(iend_c.clone());
    assert_eq!(decode(&file(&two)), Err(Error::BadChunkOrder));
    // No IDAT at all.
    assert_eq!(decode(&file(&[ihdr_c.clone(), iend_c.clone()])), Err(Error::BadChunkOrder));
    // A chunk between IDATs.
    let mut split = vec![ihdr_c.clone(), idats[0].clone(), text.clone()];
    split.extend(idats[1..].iter().cloned());
    split.push(iend_c.clone());
    assert_eq!(decode(&file(&split)), Err(Error::BadChunkOrder));
    // Ancillary chunks around the image data are fine, and so are unknown
    // ancillary ones; unknown critical ones are not.
    let mut ok = vec![ihdr_c.clone(), text.clone(), chunk(b"zzZz", &[1, 2, 3])];
    ok.extend(idats.clone());
    ok.push(text.clone());
    ok.push(iend_c.clone());
    assert_eq!(decode(&file(&ok)).unwrap().pixels, expected(&s));
    let mut critical = vec![ihdr_c.clone(), chunk(b"ZZzz", &[])];
    critical.extend(idats.clone());
    critical.push(iend_c.clone());
    assert_eq!(decode(&file(&critical)), Err(Error::UnknownCritical));
    // Data after IEND is ignored.
    let mut after = file(&ok);
    after.extend_from_slice(b"trailing garbage");
    assert_eq!(decode(&after).unwrap().pixels, expected(&s));
}

#[test]
fn palette_problems() {
    let image = |pal: Option<Vec<u8>>, depth: u8, index_byte: u8, trns: Option<Vec<u8>>| {
        let mut c = vec![ihdr(1, 1, depth, 3, 0)];
        if let Some(p) = pal {
            c.push(chunk(b"PLTE", &p));
        }
        if let Some(t) = trns {
            c.push(chunk(b"tRNS", &t));
        }
        c.push(chunk(b"IDAT", &zlib_stored(&[0, index_byte], 10)));
        c.push(chunk(b"IEND", &[]));
        decode(&file(&c))
    };
    let two = vec![1, 2, 3, 4, 5, 6];
    assert_eq!(image(Some(two.clone()), 8, 1, None).unwrap().pixels, [0xff04_0506]);
    assert_eq!(image(None, 8, 0, None), Err(Error::BadPalette), "no PLTE");
    assert_eq!(image(Some(two.clone()), 8, 2, None), Err(Error::BadPalette), "index past the palette");
    assert_eq!(image(Some(vec![1, 2, 3, 4]), 8, 0, None), Err(Error::BadChunk), "not a multiple of 3");
    assert_eq!(image(Some(vec![]), 8, 0, None), Err(Error::BadChunk), "empty PLTE");
    assert_eq!(
        image(Some([two.clone(), two.clone()].concat()), 1, 0, None),
        Err(Error::BadChunk),
        "4 entries at 1 bit"
    );
    assert_eq!(image(Some(two.clone()), 8, 0, Some(vec![9, 9, 9])).unwrap().pixels, [0x0901_0203], "long tRNS");
    // tRNS of the wrong length for grey and RGB images.
    let grey = |t: Vec<u8>| {
        decode(&file(&[
            ihdr(1, 1, 8, 0, 0),
            chunk(b"tRNS", &t),
            chunk(b"IDAT", &zlib_stored(&[0, 0], 10)),
            chunk(b"IEND", &[]),
        ]))
    };
    assert_eq!(grey(vec![0]), Err(Error::BadChunk));
    assert!(grey(vec![0, 0]).is_ok());
    // tRNS in an image with an alpha channel is not allowed: ignored.
    let rgba = decode(&file(&[
        ihdr(1, 1, 8, 6, 0),
        chunk(b"tRNS", &[1, 2]),
        chunk(b"IDAT", &zlib_stored(&[0, 1, 2, 3, 4], 10)),
        chunk(b"IEND", &[]),
    ]));
    assert_eq!(rgba.unwrap().pixels, [0x0401_0203]);
    // A palette suggestion in an RGB image is ignored.
    let rgb = decode(&file(&[
        ihdr(1, 1, 8, 2, 0),
        chunk(b"PLTE", &two),
        chunk(b"IDAT", &zlib_stored(&[0, 9, 8, 7], 10)),
        chunk(b"IEND", &[]),
    ]));
    assert_eq!(rgb.unwrap().pixels, [0xff09_0807]);
}

#[test]
fn damaged_image_data() {
    let raw_rgba = [0u8, 1, 2, 3, 4]; // a 1 x 1 RGBA row
    let png =
        |z: Vec<u8>, w: u32, h: u32| decode(&file(&[ihdr(w, h, 8, 6, 0), chunk(b"IDAT", &z), chunk(b"IEND", &[])]));
    assert!(png(zlib_stored(&raw_rgba, 10), 1, 1).is_ok());
    // A filter type 5.
    assert_eq!(png(zlib_stored(&[5, 1, 2, 3, 4], 10), 1, 1), Err(Error::BadFilter));
    // Too little data for the image, and too much.
    assert_eq!(png(zlib_stored(&raw_rgba, 10), 1, 2), Err(Error::Inflate(InflateError::Truncated)));
    assert_eq!(png(zlib_stored(&[0; 11], 10), 1, 1), Err(Error::Inflate(InflateError::TooLarge)));
    // A wrong Adler-32, and a stream that is not zlib.
    let mut z = zlib_stored(&raw_rgba, 10);
    *z.last_mut().unwrap() ^= 1;
    assert_eq!(png(z, 1, 1), Err(Error::Inflate(InflateError::BadChecksum)));
    assert_eq!(png(vec![1, 2, 3, 4, 5, 6, 7], 1, 1), Err(Error::Inflate(InflateError::BadHeader)));
    assert_eq!(png(vec![], 1, 1), Err(Error::Truncated));
    // A decompression bomb in small: a thousand bytes in sixteen (made
    // with zlib), for a 1 x 1 image, is stopped at the image's size.
    let bomb = unhex("78014b1c05a360140c7b0000f9d87af8");
    assert_eq!(png(bomb, 1, 1), Err(Error::Inflate(InflateError::TooLarge)));
}

#[test]
fn errors_say_what_is_wrong() {
    assert_eq!(format!("{}", Error::BadCrc), "a PNG chunk's CRC is wrong");
    assert_eq!(format!("{}", Error::from(InflateError::BadChecksum)), "PNG image data: zlib checksum mismatch");
    assert_eq!(Error::from(InflateError::Truncated), Error::Truncated);
}
