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
    // Deleted and unselected: selecting it succeeds (the old brush comes
    // back), but it paints nothing; a live brush given to FillRect paints.
    {
        let (dc, mut px) = dib32(&mut g, 12, 1);
        px.fill(0x00FF_FFFF);
        let red = u64::from(g.create_solid_brush(0x0000_00FF));
        g.select_object(dc, red).unwrap();
        assert_eq!(g.select_object(dc, b), Ok(red as u32));
        g.pat_blt(dc, 0, 0, 1, 1, 0x00F0_0021);
        assert_eq!(px[0], 0x00FF_FFFF);
        assert!(g.fill_rect(dc, Rect::new(1, 0, 2, 1), b));
        assert_eq!(px[1], 0x00FF_FFFF);
        assert!(g.fill_rect(dc, Rect::new(9, 0, 10, 1), red));
        assert_eq!(&px[8..11], &[0x00FF_FFFF, 0x00FF_0000, 0x00FF_FFFF]);
        // Inverted: filled with its sides swapped; empty: nothing.
        assert!(g.fill_rect(dc, Rect::new(5, 1, 4, 0), red));
        assert_eq!(&px[3..6], &[0x00FF_FFFF, 0x00FF_0000, 0x00FF_FFFF]);
        assert!(g.fill_rect(dc, Rect::new(7, 0, 7, 1), red));
        assert_eq!(px[7], 0x00FF_FFFF);
    }
    // A bitmap busy in one DC; free after that DC goes.
    let (dc, _px) = dib32(&mut g, 2, 2);
    let bmp = g.current_object(dc, OBJ_BITMAP);
    let other = u64::from(g.create_compatible_dc(0));
    assert_eq!(g.select_object(other, u64::from(bmp)), Ok(0));
    g.delete_dc(dc);
    let prev = g.select_object(other, u64::from(bmp)).unwrap();
    assert_eq!(g.object_type(u64::from(prev)), OBJ_BITMAP);
    // A selected brush deleted: it paints no more, and is gone once
    // replaced (DeleteObject do_basic on the oracle).
    let (dc, mut px) = dib32(&mut g, 1, 1);
    px[0] = 0x00FF_FFFF;
    let red = u64::from(g.create_solid_brush(0xFF));
    g.select_object(dc, red).unwrap();
    assert!(g.delete_object(red));
    assert_eq!(g.object_type(red), OBJ_BRUSH);
    g.pat_blt(dc, 0, 0, 1, 1, 0x00F0_0021);
    assert_eq!(px[0], 0x00FF_FFFF);
    g.select_object(dc, u64::from(g.stock(WHITE_BRUSH))).unwrap();
    assert_eq!(g.object_type(red), 0);
    // Its slot used again: the old handle names nothing, selecting it
    // fails and the DC keeps painting white.
    let new = u64::from(g.create_solid_brush(0x0040_5060));
    assert_ne!(new, red);
    assert_eq!(g.object_type(red), 0);
    assert_eq!(g.select_object(dc, red), Ok(0));
    g.pat_blt(dc, 0, 0, 1, 1, 0x00F0_0021);
    assert_eq!(px[0], 0x00FF_FFFF);
    assert_eq!(g.object_type(new), OBJ_BRUSH);
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

/// The rectangles of region `h`.
fn rgn_rects(g: &Gdi, h: u64) -> Vec<Rect> {
    g.region(h).unwrap().rects().to_vec()
}

#[test]
fn region_handles_create_combine_and_fail_as_observed() {
    let mut g = Gdi::new();
    // CreateRectRgn.yaml: limits.
    assert!(g.create_rect_rgn(Rect::new(-(1 << 27), -(1 << 27), (1 << 27) - 1, (1 << 27) - 1)).is_ok());
    assert_eq!(g.create_rect_rgn(Rect::new(0, 0, 1 << 27, 1 << 27)), Err(ERROR_INVALID_PARAMETER));
    assert_eq!(g.create_rect_rgn(Rect::new(i32::MIN, 0, 10, 10)), Err(ERROR_INVALID_PARAMETER));
    let r = u64::from(g.create_rect_rgn(Rect::new(1, 2, 3, 4)).unwrap());
    assert_eq!(g.object_type(r), OBJ_REGION);
    assert!(g.set_rect_rgn(r, Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX)));
    assert_eq!(g.rgn_box(r), Some((2, Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX))));
    assert!(g.set_rect_rgn(r, Rect::new(5, 5, 5, 9)));
    assert_eq!(g.rgn_box(r), Some((1, Rect::default())));
    // CombineRgn.yaml: aliasing and errors.
    let a = u64::from(g.create_rect_rgn(Rect::new(0, 0, 20, 20)).unwrap());
    let d = u64::from(g.create_rect_rgn(Rect::new(10, 10, 30, 30)).unwrap());
    assert_eq!(g.combine_rgn(d, a, d, 2), (3, 0));
    assert_eq!(rgn_rects(&g, d), [Rect::new(0, 0, 20, 10), Rect::new(0, 10, 30, 20), Rect::new(10, 20, 30, 30)]);
    assert_eq!(g.combine_rgn(d, d, d, 3), (1, 0));
    g.set_rect_rgn(d, Rect::new(5, 5, 6, 6));
    let gone = u64::from(g.create_rect_rgn(Rect::new(1, 1, 2, 2)).unwrap());
    assert!(g.delete_object(gone));
    assert!(!g.delete_object(gone));
    assert_eq!(g.combine_rgn(d, a, gone, 5), (2, 0));
    assert_eq!(rgn_rects(&g, d), [Rect::new(0, 0, 20, 20)]);
    g.set_rect_rgn(d, Rect::new(5, 5, 6, 6));
    for (dst, s1, s2, mode, err) in [
        (d, a, 0, 2, ERROR_INVALID_HANDLE),
        (d, a, gone, 1, ERROR_INVALID_HANDLE),
        (0, a, a, 2, ERROR_INVALID_HANDLE),
        (gone, a, a, 2, ERROR_INVALID_HANDLE),
        (d, 0, a, 2, ERROR_INVALID_HANDLE),
        (d, 0x0bad_0bad, 0, 5, ERROR_INVALID_HANDLE),
        (d, a, a, 0, ERROR_INVALID_PARAMETER),
        (d, a, a, 6, ERROR_INVALID_PARAMETER),
    ] {
        assert_eq!(g.combine_rgn(dst, s1, s2, mode), (0, err));
        assert_eq!(rgn_rects(&g, d), [Rect::new(5, 5, 6, 6)]);
    }
    // GetRegionData's layout.
    let data = g.region_data(a).unwrap();
    assert_eq!(data.len(), 48);
    assert_eq!(&data[0..16], &[32, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 16, 0, 0, 0]);
    assert_eq!(&data[32..48], &[0, 0, 0, 0, 0, 0, 0, 0, 20, 0, 0, 0, 20, 0, 0, 0]);
    assert_eq!(g.region_data(u64::from(g.stock(WHITE_BRUSH))), None);
    // PtInRegion / RectInRegion on something else.
    assert_eq!(g.pt_in_region(0, 1, 1), Err(ERROR_INVALID_HANDLE));
    assert_eq!(g.rect_in_region(u64::from(g.stock(BLACK_BRUSH)), Rect::new(1, 1, 2, 2)), None);
    // OffsetRgn past 2^27-1 fails and leaves the region.
    g.set_rect_rgn(d, Rect::new(0, 0, 117_440_427, 10));
    assert_eq!(g.offset_rgn(d, 0x0200_0000, 0), 0);
    assert_eq!(g.rgn_box(d), Some((2, Rect::new(0, 0, 117_440_427, 10))));
    assert_eq!(g.offset_rgn(d, 1, 1), 2);
    assert_eq!(g.equal_rgn(a, a), 1);
    assert_eq!(g.equal_rgn(a, d), 0);
}

#[test]
fn clip_regions_limit_drawing_in_device_coordinates() {
    let mut g = Gdi::new();
    let (dc, mut px) = dib32(&mut g, 20, 10);
    px.fill(0x00FF_FFFF);
    let key = [(0x00FF_FFFF, '.'), (0, '#')];
    let black = u64::from(g.stock(BLACK_BRUSH));
    let a = u64::from(g.create_rect_rgn(Rect::new(2, 2, 8, 6)).unwrap());
    let b = u64::from(g.create_rect_rgn(Rect::new(12, 4, 18, 9)).unwrap());
    let c = u64::from(g.create_rect_rgn(Rect::new(0, 0, 0, 0)).unwrap());
    g.combine_rgn(c, a, b, 2);
    assert_eq!(g.select_clip_rgn(dc, c), (3, 0));
    assert!(g.delete_object(c));
    g.fill_rect(dc, Rect::new(0, 0, 20, 10), black);
    let r = rows(&px, 20, &key);
    assert_eq!(r[2], "..######............");
    assert_eq!(r[4], "..######....######..");
    assert_eq!(r[8], "............######..");
    // A clip larger than the surface; then none.
    let big = u64::from(g.create_rect_rgn(Rect::new(-10, -10, 100, 100)).unwrap());
    assert_eq!(g.select_clip_rgn(dc, big), (2, 0));
    assert_eq!(g.clip_box(dc), (2, Rect::new(0, 0, 20, 10)));
    assert_eq!(g.select_clip_rgn(dc, 0), (2, 0));
    let r0 = u64::from(g.create_rect_rgn(Rect::new(0, 0, 0, 0)).unwrap());
    assert_eq!(g.clip_rgn(dc, r0), (0, 0));
    assert_eq!(g.clip_rgn(0x0bad_0bad, r0), (-1, ERROR_INVALID_PARAMETER));
    // A made-up region: ERROR, no error code, clip unchanged.
    assert_eq!(g.select_clip_rgn(dc, a), (2, 0));
    assert_eq!(g.select_clip_rgn(dc, 0x0bad_0bad), (0, 0));
    assert_eq!(g.clip_rgn(dc, r0), (1, 0));
    assert_eq!(rgn_rects(&g, r0), [Rect::new(2, 2, 8, 6)]);
    assert_eq!(g.select_clip_rgn(0x0bad_0bad, a), (0, ERROR_INVALID_HANDLE));
    // The viewport origin moves drawing, not the clip.
    px.fill(0x00FF_FFFF);
    assert_eq!(g.set_origin(dc, true, 5, 2), Some((0, 0)));
    g.fill_rect(dc, Rect::new(0, 0, 20, 10), black);
    let r = rows(&px, 20, &key);
    assert_eq!(r[1], "....................");
    assert_eq!(r[2], ".....###............");
    assert_eq!(r[5], ".....###............");
    assert_eq!(g.clip_box(dc), (2, Rect::new(-3, 0, 3, 4)));
    assert_eq!(g.set_pixel(dc, 0, 0, 0), 0);
    assert_eq!(g.get_pixel(dc, 0, 0), 0);
    assert_eq!(px[2 * 20 + 5], 0);
}

#[test]
fn clip_rectangles_and_origins_in_clip_boxes() {
    let mut g = Gdi::new();
    let plain = u64::from(g.create_compatible_dc(0));
    assert_eq!(g.clip_box(plain), (2, Rect::new(0, 0, 1, 1)));
    let (dc, _px) = dib32(&mut g, 40, 30);
    assert_eq!(g.clip_box(dc), (2, Rect::new(0, 0, 40, 30)));
    assert_eq!(g.clip_rect(dc, Rect::new(5, 5, 30, 20), false), (3, 0));
    assert_eq!(g.clip_box(dc), (2, Rect::new(5, 5, 30, 20)));
    assert_eq!(g.clip_rect(dc, Rect::new(5, 5, 30, 10), true), (3, 0));
    assert_eq!(g.clip_box(dc), (2, Rect::new(5, 10, 30, 20)));
    assert_eq!(g.clip_rect(dc, Rect::new(10, 12, 15, 15), true), (3, 0));
    assert_eq!(g.clip_box(dc), (3, Rect::new(5, 10, 30, 20)));
    assert_eq!(g.clip_rect(dc, Rect::new(100, 100, 110, 110), false), (1, 0));
    assert_eq!(g.clip_box(dc), (1, Rect::default()));
    g.select_clip_rgn(dc, 0);
    g.set_origin(dc, false, 10, 20);
    assert_eq!(g.clip_box(dc), (2, Rect::new(10, 20, 50, 50)));
    g.set_origin(dc, false, 0, 0);
    g.set_origin(dc, true, 10, 20);
    assert_eq!(g.clip_box(dc), (2, Rect::new(-10, -20, 30, 10)));
    g.set_origin(dc, true, 0, 0);
    // ExtSelectClipRgn without a clip region works on the whole surface.
    let b = u64::from(g.create_rect_rgn(Rect::new(10, 10, 30, 25)).unwrap());
    assert_eq!(g.ext_select_clip_rgn(dc, b, 2), (2, 0));
    assert_eq!(g.clip_box(dc), (2, Rect::new(0, 0, 40, 30)));
    g.select_clip_rgn(dc, 0);
    assert_eq!(g.ext_select_clip_rgn(dc, b, 4), (3, 0));
    assert_eq!(g.ext_select_clip_rgn(dc, 0, 1), (0, 0));
    assert_eq!(g.ext_select_clip_rgn(dc, b, 9), (0, ERROR_INVALID_PARAMETER));
    assert_eq!(g.ext_select_clip_rgn(dc, 0, 5), (2, 0));
    // SaveDC keeps the clip region.
    g.clip_rect(dc, Rect::new(1, 1, 2, 2), false);
    let level = g.save_dc(dc);
    g.select_clip_rgn(dc, 0);
    assert!(g.restore_dc(dc, level));
    assert_eq!(g.clip_box(dc), (2, Rect::new(1, 1, 2, 2)));
    let dead = u64::from(g.create_compatible_dc(0));
    g.delete_dc(dead);
    assert_eq!(g.clip_box(dead).0, 0);
}

#[test]
fn window_dcs_clip_to_their_visible_region() {
    let mut g = Gdi::new();
    // No geometry: the screen.
    let dc = u64::from(g.create_display_dc(7, false));
    assert_eq!(g.clip_box(dc), (2, Rect::new(0, 0, 1024, 768)));
    // A popup at -50,100, 200x100, with the screen's left edge cut off.
    assert!(g.set_display_geometry(dc, (-50, 100), &[Rect::new(0, 100, 150, 200)], &[]));
    assert_eq!(g.clip_box(dc), (2, Rect::new(50, 0, 200, 100)));
    let r = u64::from(g.create_rect_rgn(Rect::new(0, 0, 0, 0)).unwrap());
    assert_eq!(g.random_rgn(dc, r, SYSRGN), 1);
    assert_eq!(rgn_rects(&g, r), [Rect::new(0, 100, 150, 200)]);
    // WS_CLIPCHILDREN: the child is left out, the box stays.
    assert!(g.set_display_geometry(dc, (100, 300), &[Rect::new(100, 300, 300, 400)], &[Rect::new(250, 350, 350, 450)]));
    assert_eq!(g.clip_box(dc), (3, Rect::new(0, 0, 200, 100)));
    g.random_rgn(dc, r, SYSRGN);
    assert_eq!(rgn_rects(&g, r), [Rect::new(100, 300, 300, 350), Rect::new(100, 350, 250, 400)]);
    // A clip region in client coordinates, kept as given.
    assert!(g.set_display_geometry(dc, (100, 100), &[Rect::new(100, 100, 300, 200)], &[]));
    let big = u64::from(g.create_rect_rgn(Rect::new(150, 50, 400, 300)).unwrap());
    assert_eq!(g.select_clip_rgn(dc, big), (2, 0));
    assert_eq!(g.clip_box(dc), (2, Rect::new(150, 50, 200, 100)));
    g.clip_rgn(dc, r);
    assert_eq!(rgn_rects(&g, r), [Rect::new(150, 50, 400, 300)]);
    let out = u64::from(g.create_rect_rgn(Rect::new(300, 300, 400, 400)).unwrap());
    assert_eq!(g.select_clip_rgn(dc, out), (1, 0));
    assert_eq!(g.clip_box(dc), (1, Rect::default()));
    // Hidden: nothing.
    assert!(g.set_display_geometry(dc, (100, 300), &[], &[]));
    g.select_clip_rgn(dc, 0);
    assert_eq!(g.clip_box(dc), (1, Rect::default()));
    // An update region narrows what can be drawn.
    assert!(g.set_display_geometry(dc, (0, 0), &[Rect::new(0, 0, 200, 100)], &[]));
    let two = crate::region::Region::from_rects(&[Rect::new(10, 10, 30, 20), Rect::new(100, 60, 150, 90)]);
    assert!(g.set_system_clip(dc, Some(two)));
    assert_eq!(g.clip_box(dc), (3, Rect::new(10, 10, 150, 90)));
    assert_eq!(g.clip_rgn(dc, r), (0, 0));
    assert!(g.set_system_clip(dc, None));
    assert_eq!(g.clip_box(dc), (2, Rect::new(0, 0, 200, 100)));
    assert!(g.set_system_clip(dc, Some(crate::region::Region::from_rect(Rect::new(10, 20, 40, 50)))));
    assert_eq!(g.clip_box(dc), (2, Rect::new(10, 20, 40, 50)));
    // A new GetDC on the same (own) DC is not limited any more.
    assert!(g.set_display_geometry(dc, (0, 0), &[Rect::new(0, 0, 200, 100)], &[]));
    assert_eq!(g.clip_box(dc), (2, Rect::new(0, 0, 200, 100)));
    let mem = u64::from(g.create_compatible_dc(0));
    assert!(!g.set_display_geometry(mem, (0, 0), &[], &[]));
}

/// A 92-byte `LOGFONTW` (`wingdi.h`) with a height, weight and face name.
fn logfont(name: &str, height: i32) -> Vec<u8> {
    let mut b = vec![0u8; 92];
    b[0..4].copy_from_slice(&height.to_le_bytes());
    b[16..20].copy_from_slice(&400i32.to_le_bytes());
    for (i, c) in name.encode_utf16().enumerate() {
        b[28 + 2 * i..30 + 2 * i].copy_from_slice(&c.to_le_bytes());
    }
    b
}

#[test]
fn created_fonts_are_mapped_by_name_and_height() {
    use crate::font::Face;
    let mut g = Gdi::new();
    let dc = u64::from(g.create_compatible_dc(0));
    let mut height_of = |name: &str, h: i32| {
        let f = u64::from(g.create_font(&logfont(name, h)));
        g.select_object(dc, f).unwrap();
        g.text_metrics(dc).unwrap().height
    };
    assert_eq!(height_of("Segoe UI", -12), Face::Gui.metrics().height);
    assert_eq!(height_of("Courier New", -16), Face::AnsiFixed.metrics().height);
    assert_eq!(height_of("Terminal", -16), Face::OemFixed.metrics().height);
    assert_eq!(height_of("Arial", -24), Face::System.metrics().height);
    // A short LOGFONT is the system font's face.
    let f = u64::from(g.create_font(&[0; 10]));
    g.select_object(dc, f).unwrap();
    assert_eq!(g.text_metrics(dc).unwrap(), Face::System.metrics());
}

#[test]
fn small_fonts_draw_inside_their_extent_with_descenders() {
    let mut g = Gdi::new();
    let (dc, mut px) = dib32(&mut g, 80, 24);
    px.fill(0x00FF_FFFF);
    let f = u64::from(g.create_font(&logfont("Tahoma", -11)));
    g.select_object(dc, f).unwrap();
    g.set_bk_mode(dc, TRANSPARENT);
    let s: Vec<u16> = "Hag gypsy".encode_utf16().collect();
    let (cx, cy) = g.text_extent(dc, &s).unwrap();
    assert_eq!(cy, 13);
    g.text_out(dc, 2, 3, &s);
    let ink: Vec<(i32, i32)> = (0..24)
        .flat_map(|y| (0..80).map(move |x| (x, y)))
        .filter(|&(x, y)| px[(y * 80 + x) as usize] != 0x00FF_FFFF)
        .collect();
    assert!(ink.len() > 60);
    // Inside the cell (2, 3, 2 + cx, 3 + 13); the descenders of g, y and p
    // reach its last rows (descent 2).
    assert!(ink.iter().all(|&(x, y)| x >= 2 && x < 2 + cx && (3..16).contains(&y)), "{ink:?}");
    assert_eq!(ink.iter().map(|p| p.1).max(), Some(3 + 12));
    assert_eq!(ink.iter().map(|p| p.1).min(), Some(3 + 1));
}
