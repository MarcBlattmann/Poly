//! GDI scenarios of the slice's visible tests, on DIBs in test memory.

extern crate std;

use crate::gdi::*;
use alloc::vec;
use alloc::vec::Vec;

/// A memory DC with a top-down 32-bit DIB selected; the pixels.
fn dib32(g: &mut Gdi, w: i32, h: i32) -> (u64, Vec<u32>) {
    let mem = vec![0u32; (w * h) as usize];
    let dc = u64::from(g.create_compatible_dc(0));
    let b = g.create_dib(w, -h, 32, Vec::new(), mem.as_ptr() as u64, mem.len() * 4);
    g.select_object(dc, u64::from(b)).unwrap();
    (dc, mem)
}

fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

fn rows(px: &[u32], w: usize, key: &[(u32, char)]) -> Vec<std::string::String> {
    px.chunks(w).map(|r| r.iter().map(|v| key.iter().find(|k| k.0 == *v).map_or('?', |k| k.1)).collect()).collect()
}

#[test]
fn select_object_changes_what_is_drawn() {
    let mut g = Gdi::new();
    let (dc, mut px) = dib32(&mut g, 10, 6);
    px.fill(0x00FF_FFFF);
    let key = [(0x00FF_FFFF, '.'), (0x00FF_0000, 'R'), (0x0000_FF00, 'G'), (0x0000_00FF, 'B'), (0x0012_3456, 'D')];
    let red = u64::from(g.create_solid_brush(rgb(255, 0, 0)));
    let green = u64::from(g.create_pen(0, 1, rgb(0, 255, 0)));
    let blue = u64::from(g.create_solid_brush(rgb(0, 0, 255)));
    let old = g.select_object(dc, red).unwrap();
    assert_eq!(old, g.stock(WHITE_BRUSH));
    g.pat_blt(dc, 0, 0, 3, 2, 0x00F0_0021);
    g.select_object(dc, green).unwrap();
    g.select_object(dc, blue).unwrap();
    g.rectangle(dc, 4, 0, 9, 4);
    assert_eq!(
        rows(&px, 10, &key),
        ["RRR.GGGGG.", "RRR.GBBBG.", "....GBBBG.", "....GGGGG.", "..........", ".........."]
    );
    px.fill(0x00FF_FFFF);
    g.select_object(dc, u64::from(g.stock(NULL_BRUSH))).unwrap();
    g.rectangle(dc, 0, 0, 5, 4);
    g.select_object(dc, u64::from(g.stock(NULL_PEN))).unwrap();
    g.select_object(dc, blue).unwrap();
    g.rectangle(dc, 5, 1, 9, 5);
    assert_eq!(
        rows(&px, 10, &key),
        ["GGGGG.....", "G...GBBB..", "G...GBBB..", "GGGGGBBB..", "..........", ".........."]
    );
    px.fill(0x00FF_FFFF);
    g.select_object(dc, u64::from(g.stock(DC_BRUSH))).unwrap();
    assert_eq!(g.set_dc_brush_color(dc, rgb(0x12, 0x34, 0x56)), 0x00FF_FFFF);
    g.pat_blt(dc, 1, 1, 2, 2, 0x00F0_0021);
    assert_eq!(rows(&px, 10, &key)[1], ".DD.......");
}

#[test]
fn blits_raster_operations_and_monochrome() {
    let mut g = Gdi::new();
    let (src, mut s) = dib32(&mut g, 2, 1);
    let (dst, mut d) = dib32(&mut g, 2, 1);
    let brush = u64::from(g.create_solid_brush(rgb(0x0F, 0xF0, 0x55)));
    g.select_object(dst, brush).unwrap();
    let mut run = |rop: u32| {
        s[0] = 0x00F0_F00F;
        s[1] = 0x0012_3456;
        d[0] = 0x000F_0FF0;
        d[1] = 0x00AB_CDEF;
        assert!(g.bit_blt(dst, 0, 0, 2, 1, src, 0, 0, rop).0);
        [d[0], d[1]]
    };
    assert_eq!(run(0x00CC_0020), [0x00f0_f00f, 0x0012_3456]);
    assert_eq!(run(0x00EE_0086), [0x00ff_ffff, 0x00bb_fdff]);
    assert_eq!(run(0x0033_0008), [0xff0f_0ff0, 0xffed_cba9]);
    assert_eq!(run(0x0055_0009), [0xfff0_f00f, 0xff54_3210]);
    assert_eq!(run(0x00F0_0021), [0x000f_f055, 0x000f_f055]);
    assert_eq!(g.bit_blt(dst, 0, 0, 1, 1, 0, 0, 0, 0x00CC_0020), (false, 0));

    // Colour to monochrome and back.
    let (c, mut cp) = dib32(&mut g, 4, 1);
    cp.copy_from_slice(&[0x00FF_0000, 0x0000_FF00, 0x00FF_FFFF, 0]);
    let m = u64::from(g.create_compatible_dc(0));
    let mono = u64::from(g.create_bitmap(4, 1, 1, None));
    g.select_object(m, mono).unwrap();
    g.set_bk_color(c, rgb(0, 255, 0));
    g.bit_blt(m, 0, 0, 4, 1, c, 0, 0, 0x00CC_0020);
    let got: Vec<u32> = (0..4).map(|x| g.get_pixel(m, x, 0)).collect();
    assert_eq!(got, [0, 0xffffff, 0, 0]);
    let (out, op) = dib32(&mut g, 4, 1);
    g.set_text_color(out, rgb(0x11, 0x22, 0x33));
    g.set_bk_color(out, rgb(0xAA, 0xBB, 0xCC));
    g.bit_blt(out, 0, 0, 4, 1, m, 0, 0, 0x00CC_0020);
    assert_eq!(op[..], [0x112233, 0xaabbcc, 0x112233, 0x112233]);
}

#[test]
fn objects_live_and_die_as_observed() {
    let mut g = Gdi::new();
    let m = u64::from(g.create_compatible_dc(0));
    assert_eq!(g.object_type(m), OBJ_MEMDC);
    assert_eq!(g.get_pixel(m, 0, 0), CLR_INVALID);
    assert_eq!(g.delete_dc(m), (true, 0));
    assert_eq!(g.object_type(m), OBJ_DC);
    assert_eq!(g.delete_dc(m), (false, 0));
    let b = u64::from(g.create_solid_brush(0x332211));
    assert!(g.delete_object(b));
    assert_eq!(g.object_type(b), OBJ_BRUSH);
    assert!(!g.delete_object(b));
    // A bitmap busy in one DC; free after that DC goes.
    let (dc, _px) = dib32(&mut g, 2, 2);
    let bmp = g.current_object(dc, OBJ_BITMAP);
    let other = u64::from(g.create_compatible_dc(0));
    assert_eq!(g.select_object(other, u64::from(bmp)), Ok(0));
    g.delete_dc(dc);
    let prev = g.select_object(other, u64::from(bmp)).unwrap();
    assert_eq!(g.object_type(u64::from(prev)), OBJ_BITMAP);
    // A selected brush deleted: painting on, gone once replaced.
    let (dc, px) = dib32(&mut g, 1, 1);
    let red = u64::from(g.create_solid_brush(0xFF));
    g.select_object(dc, red).unwrap();
    assert!(g.delete_object(red));
    g.pat_blt(dc, 0, 0, 1, 1, 0x00F0_0021);
    assert_eq!(px[0], 0x00FF_0000);
    g.select_object(dc, u64::from(g.stock(WHITE_BRUSH))).unwrap();
    assert_eq!(g.object_type(red), 0);
}

#[test]
fn text_cells_and_glyphs() {
    let mut g = Gdi::new();
    let (dc, mut px) = dib32(&mut g, 32, 20);
    px.fill(0x00FF_FFFF);
    let s: Vec<u16> = "Ag1".encode_utf16().collect();
    g.text_out(dc, 1, 1, &s);
    let r = rows(&px, 32, &[(0x00FF_FFFF, '.'), (0, '#')]);
    assert_eq!(r[4], "....##..............##..........");
    assert_eq!(r[13], ".##....##..#####....##..........");
    assert_eq!(r[16], "...........####.................");
    // "Mx" transparent: 90 glyph pixels in 4,6,23,16.
    let (dc, mut px) = dib32(&mut g, 60, 30);
    px.fill(0x0022_2222);
    g.set_bk_mode(dc, TRANSPARENT);
    g.text_out(dc, 3, 3, &[0x4D, 0x78]);
    let changed: Vec<(i32, i32)> = (0..30)
        .flat_map(|y| (0..60).map(move |x| (x, y)))
        .filter(|&(x, y)| px[(y * 60 + x) as usize] != 0x0022_2222)
        .collect();
    assert_eq!(changed.len(), 90);
    assert_eq!(changed.iter().map(|p| p.0).min(), Some(4));
    assert_eq!(changed.iter().map(|p| p.0).max(), Some(22));
    assert_eq!(changed.iter().map(|p| p.1).min(), Some(6));
    assert_eq!(changed.iter().map(|p| p.1).max(), Some(15));
}
