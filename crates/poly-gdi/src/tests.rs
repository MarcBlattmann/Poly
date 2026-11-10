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
    // back), and it paints white; a live brush given to FillRect paints.
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
    // A selected brush the DC never drew with, deleted: it paints white,
    // and is gone once replaced (DeleteObject.yaml `selected_brush_pen`).
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

/// CreateRectRgn.yaml `normalised`, `empty`, `negative`, `interface.GetRegionData`.
#[test]
fn rectangular_regions_are_normalised_and_empty_ones_are_empty() {
    let mut g = Gdi::new();
    for (l, t, r, b) in [(110, 20, 10, 70), (10, 70, 110, 20), (110, 70, 10, 20), (10, 20, 110, 70)] {
        let h = u64::from(g.create_rect_rgn(Rect::new(l, t, r, b)).unwrap());
        assert_eq!(g.rgn_box(h), Some((2, Rect::new(10, 20, 110, 70))), "{l},{t},{r},{b}");
        assert_eq!(rgn_rects(&g, h), [Rect::new(10, 20, 110, 70)]);
        // One rectangle: 32 bytes of header and 16 of rectangle.
        assert_eq!(g.region_data(h).unwrap().len(), 48);
    }
    for (l, t, r, b) in [(10, 20, 10, 70), (10, 20, 60, 20), (0, 0, 0, 0), (60, 70, 10, 70)] {
        let h = u64::from(g.create_rect_rgn(Rect::new(l, t, r, b)).unwrap());
        assert_eq!(g.object_type(h), OBJ_REGION);
        assert_eq!(g.rgn_box(h), Some((1, Rect::default())), "{l},{t},{r},{b}");
        assert!(rgn_rects(&g, h).is_empty());
        let data = g.region_data(h).unwrap();
        assert_eq!(data.len(), 32, "an empty region is the header only");
        assert_eq!(&data[8..16], &[0, 0, 0, 0, 0, 0, 0, 0], "no rectangles, no bytes of them");
        assert_eq!(&data[16..32], &[0u8; 16], "bound 0,0,0,0");
    }
    // Negative coordinates are kept.
    let h = u64::from(g.create_rect_rgn(Rect::new(-30, -20, -10, -5)).unwrap());
    assert_eq!(g.rgn_box(h), Some((2, Rect::new(-30, -20, -10, -5))));
    // The limits on every side: -2^27 and 2^27 - 1 work, one past fails.
    for bad in [1 << 27, -(1 << 27) - 1, 1 << 28, i32::MAX, i32::MIN] {
        for corner in 0..4 {
            let mut c = [0, 0, 10, 10];
            c[corner] = bad;
            assert_eq!(g.create_rect_rgn(Rect::new(c[0], c[1], c[2], c[3])), Err(ERROR_INVALID_PARAMETER), "{c:?}");
        }
    }
    let sq = u64::from(g.create_rect_rgn(Rect::new(0, 0, 1_000_000, 1_000_000)).unwrap());
    assert_eq!(g.rgn_box(sq), Some((2, Rect::new(0, 0, 1_000_000, 1_000_000))));
    // SetRectRgn: swapped corners are put in order, anything else kept.
    assert!(g.set_rect_rgn(sq, Rect::new(50, 40, 10, 20)));
    assert_eq!(g.rgn_box(sq), Some((2, Rect::new(10, 20, 50, 40))));
    assert!(g.set_rect_rgn(sq, Rect::new(1 << 27, 0, 1 << 28, 10)));
    assert_eq!(g.rgn_box(sq), Some((2, Rect::new(1 << 27, 0, 1 << 28, 10))));
    // 1000 regions made and deleted in turn all work.
    for i in 0..1000 {
        let h = u64::from(g.create_rect_rgn(Rect::new(i, i, i + 5, i + 5)).unwrap());
        assert!(g.delete_object(h));
        assert!(!g.delete_object(h));
        // type_after_delete: a deleted region keeps reporting OBJ_REGION.
        assert_eq!(g.object_type(h), OBJ_REGION);
        assert_eq!(g.rgn_box(h), None);
    }
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

// ----- held-out hunt: DeleteObject.yaml and CreateRectRgn.yaml rules the
// visible tests do not show.

#[test]
fn delete_object_follows_the_sheet_for_every_kind_of_object() {
    let mut g = Gdi::new();
    // success: brush, pen, bitmap, font, region: 1; the second delete 0.
    let brush = u64::from(g.create_solid_brush(0x123456));
    let pen = u64::from(g.create_pen(0, 1, 0x654321));
    let bmp = u64::from(g.create_bitmap(4, 4, 32, None));
    let rgn = u64::from(g.create_rect_rgn(Rect::new(0, 0, 5, 5)).unwrap());
    for h in [brush, pen, bmp, rgn] {
        assert!(g.delete_object(h), "{h:#x}");
    }
    // type_after_delete: every deleted object keeps reporting its type.
    assert_eq!(g.object_type(brush), OBJ_BRUSH);
    for (h, t) in [(pen, OBJ_PEN), (bmp, OBJ_BITMAP), (rgn, OBJ_REGION)] {
        assert_eq!(g.object_type(h), t, "{h:#x}");
    }
    for h in [brush, pen, bmp, rgn] {
        assert!(!g.delete_object(h), "second delete {h:#x}");
    }
    // NULL and made-up handles: false.
    for h in [0, 0x1234, 0x0bad_0bad, u64::MAX] {
        assert!(!g.delete_object(h), "{h:#x}");
    }
    // stock objects: 1, and they stay usable.
    for i in [WHITE_BRUSH, BLACK_BRUSH, NULL_BRUSH, DC_BRUSH] {
        let s = u64::from(g.stock(i));
        assert!(g.delete_object(s));
        assert_eq!(g.object_type(s), OBJ_BRUSH);
        assert!(g.delete_object(s), "still deletable-as-no-op");
    }
    // DEFAULT_PALETTE (15) too: 1, type OBJ_PAL, the same handle again.
    let pal = u64::from(g.stock(15));
    assert!(g.delete_object(pal));
    assert_eq!(g.object_type(pal), OBJ_PAL);
    assert_eq!(u64::from(g.stock(15)), pal);
    let (dc, mut px) = dib32(&mut g, 2, 1);
    px.fill(0);
    g.select_object(dc, u64::from(g.stock(WHITE_BRUSH))).unwrap();
    g.pat_blt(dc, 0, 0, 1, 1, 0x00F0_0021);
    assert_eq!(px[0], 0x00FF_FFFF);
}

#[test]
fn a_pen_or_bitmap_deleted_while_selected_goes_when_deselected_or_with_its_dc() {
    let mut g = Gdi::new();
    let (dc, _px) = dib32(&mut g, 2, 2);
    let pen = u64::from(g.create_pen(0, 1, 0xFF));
    let old = g.select_object(dc, pen).unwrap();
    assert!(g.delete_object(pen));
    assert_eq!(g.object_type(pen), OBJ_PEN, "still there while selected");
    assert!(!g.delete_object(pen), "a second delete fails");
    assert_eq!(g.object_bytes(pen), None, "GetObject answers nothing while it waits");
    g.select_object(dc, u64::from(old)).unwrap();
    // after_deselect: a deleted pen is reported as OBJ_PEN for ever, but
    // cannot be deleted or selected again.
    assert_eq!(g.object_type(pen), OBJ_PEN);
    assert!(!g.delete_object(pen));
    assert_eq!(g.select_object(dc, pen), Ok(0));
    // selected_bitmap: 1, stays OBJ_BITMAP until the DC is deleted; then a
    // DeleteObject fails.
    let bmp = u64::from(g.current_object(dc, OBJ_BITMAP));
    assert!(g.delete_object(bmp));
    assert_eq!(g.object_type(bmp), OBJ_BITMAP);
    assert!(!g.delete_object(bmp));
    assert_eq!(g.object_bytes_for(bmp, 128).map(|b| b.len()), Some(104), "a DIB section's DIBSECTION");
    assert_eq!(g.object_bytes_for(bmp, 32).map(|b| b.len()), Some(32));
    let other = u64::from(g.create_compatible_dc(0));
    assert_eq!(g.select_object(other, bmp), Ok(0), "not into a second DC");
    assert!(g.delete_dc(dc).0);
    assert_eq!(g.object_type(bmp), OBJ_BITMAP, "the type stays after the DC is gone");
    assert_eq!(g.object_bytes(bmp), None);
    assert!(!g.delete_object(bmp));
    assert_eq!(g.select_object(other, bmp), Ok(0));
    // dc: DeleteObject of a memory DC is 1; the DC still reads OBJ_DC.
    let m = u64::from(g.create_compatible_dc(0));
    assert!(g.delete_object(m));
    assert_eq!(g.object_type(m), OBJ_DC);
    assert!(!g.delete_object(m));
}

#[test]
fn a_display_dc_knows_its_window() {
    // WindowFromDC (BeginPaint.yaml `erase_sent`) asks which window a DC is for.
    let mut g = Gdi::new();
    let dc = u64::from(g.create_display_dc(0x30042, false));
    assert_eq!(g.display_hwnd(dc), Some(0x30042));
    let screen = u64::from(g.create_display_dc(0, false));
    assert_eq!(g.display_hwnd(screen), Some(0));
    let mem = u64::from(g.create_compatible_dc(0));
    assert_eq!(g.display_hwnd(mem), None);
    assert!(g.release_display_dc(dc));
    assert_eq!(g.display_hwnd(dc), None, "a released DC names no window");
    assert_eq!(g.display_hwnd(0x77), None);
}

#[test]
fn a_deleted_region_is_not_a_source_and_a_region_may_be_deleted_while_clipping() {
    // CreateRectRgn.yaml `delete`; SelectClipRgn.yaml (the clip is a copy).
    let mut g = Gdi::new();
    let a = u64::from(g.create_rect_rgn(Rect::new(0, 0, 10, 10)).unwrap());
    let dst = u64::from(g.create_rect_rgn(Rect::new(1, 1, 2, 2)).unwrap());
    let (dc, _px) = dib32(&mut g, 20, 20);
    assert_eq!(g.select_clip_rgn(dc, a).0, 2);
    assert!(g.delete_object(a));
    assert_eq!(g.clip_box(dc), (2, Rect::new(0, 0, 10, 10)), "the DC keeps its copy");
    assert_eq!(g.combine_rgn(dst, a, 0, 5), (0, 6), "ERROR_INVALID_HANDLE");
    assert_eq!(g.rgn_box(a), None);
    // 1000 regions made and deleted in turn.
    for i in 0..1000 {
        let r = u64::from(g.create_rect_rgn(Rect::new(0, 0, i + 1, 3)).unwrap());
        assert!(g.delete_object(r));
    }
    // Every limit of the range, on each side.
    let lim = 1 << 27;
    for r in [
        Rect::new(-lim, -lim, lim - 1, lim - 1),
        Rect::new(0, -lim, 1, lim - 1),
        Rect::new(lim - 2, lim - 2, lim - 1, lim - 1),
    ] {
        assert!(g.create_rect_rgn(r).is_ok(), "{r:?}");
    }
    for r in [
        Rect::new(lim, 0, 1, 1),
        Rect::new(0, lim, 1, 1),
        Rect::new(0, 0, lim, 1),
        Rect::new(0, 0, 1, lim),
        Rect::new(-lim - 1, 0, 1, 1),
        Rect::new(0, -lim - 1, 1, 1),
        Rect::new(0, 0, i32::MAX, 1),
        Rect::new(0, 0, 1, i32::MIN),
    ] {
        assert_eq!(g.create_rect_rgn(r), Err(87), "{r:?}");
    }
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
    // Names Poly knows get a scalable font (the built-in one where the font
    // store has no files): the em is the height asked for, the cell the
    // font's ascent plus descent.
    assert_eq!(height_of("Segoe UI", -12), 12);
    assert_eq!(height_of("Courier New", -16), 16);
    assert_eq!(height_of("Arial", -24), 24);
    // Raster font names are their bitmap faces at every size.
    assert_eq!(height_of("Terminal", -16), Face::OemFixed.metrics().height);
    assert_eq!(height_of("System", -40), Face::System.metrics().height);
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
    assert_eq!(cy, 11);
    g.text_out(dc, 2, 3, &s);
    let ink: Vec<(i32, i32)> = (0..24)
        .flat_map(|y| (0..80).map(move |x| (x, y)))
        .filter(|&(x, y)| px[(y * 80 + x) as usize] != 0x00FF_FFFF)
        .collect();
    assert!(ink.len() > 60);
    // Inside the cell (2, 3, 2 + cx, 3 + 11); the descenders of g, y and p
    // reach its last rows (descent 3).
    assert!(ink.iter().all(|&(x, y)| x >= 1 && x < 3 + cx && (3..14).contains(&y)), "{ink:?}");
    assert!(ink.iter().map(|p| p.1).max() >= Some(3 + 9));
    assert!(ink.iter().map(|p| p.1).min() <= Some(3 + 3));
}

// ----- outline text (ADR 0142): needs the DejaVu fonts (`POLY_FONT_DIR`).

/// A GDI whose font store reads DejaVu from `POLY_FONT_DIR`; `None` (and a
/// note) where the fonts are not fetched.
fn gdi_with_fonts() -> Option<Gdi> {
    if crate::outline::tests::dejavu("DejaVuSans.ttf").is_none() {
        std::eprintln!("skipped: no DejaVu fonts (POLY_FONT_DIR)");
        return None;
    }
    let mut g = Gdi::new();
    g.set_font_loader(crate::outline::tests::dejavu);
    Some(g)
}

fn white_canvas(g: &mut Gdi, w: i32, h: i32) -> (u64, Vec<u32>) {
    let (dc, mut px) = dib32(g, w, h);
    px.fill(0x00FF_FFFF);
    (dc, px)
}

fn ink_of(px: &[u32], w: i32) -> Vec<(i32, i32, u32)> {
    px.iter().enumerate().filter(|(_, v)| **v != 0x00FF_FFFF).map(|(i, &v)| (i as i32 % w, i as i32 / w, v)).collect()
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn px_fill(px: &[u32], v: u32) {
    // SAFETY: the DIB's memory is the test's own `Vec`, which the GDI only
    // borrows by address.
    unsafe { core::slice::from_raw_parts_mut(px.as_ptr() as *mut u32, px.len()) }.fill(v);
}

#[test]
fn logical_fonts_draw_antialiased_glyphs_on_their_baseline() {
    let Some(mut g) = gdi_with_fonts() else { return };
    let (dc, px) = white_canvas(&mut g, 90, 30);
    let f = u64::from(g.create_font(&lf_full("Segoe UI", -16, 400, false, 2)));
    g.select_object(dc, f).unwrap();
    g.set_bk_mode(dc, TRANSPARENT);
    let tm = g.text_metrics(dc).unwrap();
    assert_eq!((tm.ascent, tm.descent), (15, 4));
    let s = utf16("Hxg");
    let (cx, cy) = g.text_extent(dc, &s).unwrap();
    assert_eq!(cy, tm.height);
    assert!(g.text_out(dc, 4, 4, &s));
    let ink = ink_of(&px, 90);
    assert!(ink.len() > 40);
    // Grey scale: black, white and the greys between.
    assert!(ink.iter().any(|&(_, _, v)| v == 0), "a fully covered pixel");
    let grey = |v: u32| v != 0 && (v & 0xFF) == (v >> 8 & 0xFF) && (v & 0xFF) == (v >> 16 & 0xFF);
    assert!(ink.iter().any(|&(_, _, v)| grey(v)), "grey pixels");
    // The glyphs sit inside the cell; capitals end on the baseline (row
    // 4 + ascent - 1), the descender of g reaches below it.
    assert!(ink.iter().all(|&(x, y, _)| x >= 3 && x < 4 + cx + 2 && y >= 4 && y < 4 + cy), "{ink:?}");
    let h_width = g.text_extent(dc, &utf16("H")).unwrap().0;
    let h_ink: Vec<_> = ink.iter().filter(|&&(x, _, _)| x < 4 + h_width).collect();
    assert_eq!(h_ink.iter().map(|p| p.1).max(), Some(4 + tm.ascent - 1));
    assert!(ink.iter().map(|p| p.1).max().unwrap() > 4 + tm.ascent);
}

#[test]
fn text_colour_background_and_alignment_follow_the_dc() {
    let Some(mut g) = gdi_with_fonts() else { return };
    let (dc, px) = white_canvas(&mut g, 120, 40);
    let f = u64::from(g.create_font(&lf_full("Arial", -14, 400, false, 2)));
    g.select_object(dc, f).unwrap();
    g.set_text_color(dc, rgb(200, 0, 0));
    g.set_bk_color(dc, rgb(0, 0, 220));
    let s = utf16("Mm");
    let (cx, cy) = g.text_extent(dc, &s).unwrap();
    // OPAQUE: the cell is the background colour, the glyphs red on it.
    g.text_out(dc, 10, 5, &s);
    let ink = ink_of(&px, 120);
    let cell = |x: i32, y: i32| (10..10 + cx).contains(&x) && (5..5 + cy).contains(&y);
    assert!(ink.iter().all(|&(x, y, _)| cell(x, y)), "only the cell changes");
    assert_eq!(px[(5 * 120 + 10) as usize], 220, "background (0x00RRGGBB)");
    assert!(ink.iter().any(|&(_, _, v)| v == 200 << 16), "full coverage is the text colour");
    // TA_RIGHT | TA_BOTTOM puts the cell's bottom right corner at the point.
    px_fill(&px, 0x00FF_FFFF);
    g.set_text_align(dc, TA_RIGHT | 8);
    g.text_out(dc, 100, 30, &s);
    let ink = ink_of(&px, 120);
    assert!(ink.iter().all(|&(x, y, _)| x >= 100 - cx && x < 100 && y >= 30 - cy && y < 30));
}

#[test]
fn ext_text_out_fills_clips_and_spaces() {
    let Some(mut g) = gdi_with_fonts() else { return };
    let (dc, px) = white_canvas(&mut g, 100, 30);
    let f = u64::from(g.create_font(&logfont("Courier New", -14)));
    g.select_object(dc, f).unwrap();
    g.set_bk_color(dc, rgb(255, 255, 0));
    let s = utf16("abcdef");
    // ETO_OPAQUE | ETO_CLIPPED: the rectangle yellow, the text only inside.
    let r = Rect::new(10, 5, 40, 20);
    assert!(g.ext_text_out(dc, 5, 5, ETO_OPAQUE | ETO_CLIPPED, Some(r), &s, None));
    let ink = ink_of(&px, 100);
    assert!(ink.iter().all(|&(x, y, _)| r.contains_point(x, y)), "nothing outside the rectangle");
    assert_eq!(px[(19 * 100 + 39) as usize], 0x00FF_FF00, "the rectangle is filled");
    // lpDx spaces the characters.
    g.set_bk_mode(dc, TRANSPARENT);
    let w = g.text_extent(dc, &utf16("i")).unwrap().0;
    let xs = |g: &mut Gdi, dx: Option<&[i32]>| {
        px_fill(&px, 0x00FF_FFFF);
        g.ext_text_out(dc, 0, 5, 0, None, &utf16("ii"), dx);
        ink_of(&px, 100).iter().map(|p| p.0).collect::<Vec<_>>()
    };
    let tight = xs(&mut g, None);
    let spread = xs(&mut g, Some(&[w + 20, w]));
    assert_eq!(spread.iter().max().unwrap() - tight.iter().max().unwrap(), 20);
    // An empty string with ETO_OPAQUE still fills the rectangle.
    px_fill(&px, 0x00FF_FFFF);
    g.set_bk_mode(dc, OPAQUE);
    assert!(g.ext_text_out(dc, 0, 0, ETO_OPAQUE, Some(Rect::new(1, 1, 4, 3)), &[], None));
    assert_eq!(ink_of(&px, 100).len(), 6);
    assert!(!g.ext_text_out(0xdead, 0, 0, 0, None, &s, None));
}

#[test]
fn underline_and_strike_out_come_from_the_logfont() {
    let Some(mut g) = gdi_with_fonts() else { return };
    let (dc, px) = white_canvas(&mut g, 60, 30);
    g.set_bk_mode(dc, TRANSPARENT);
    let plain = u64::from(g.create_font(&logfont("Tahoma", -16)));
    let mut lf = logfont("Tahoma", -16);
    lf[21] = 1;
    let under = u64::from(g.create_font(&lf));
    let s = utf16("lll");
    g.select_object(dc, plain).unwrap();
    let tm = g.text_metrics(dc).unwrap();
    g.text_out(dc, 2, 2, &s);
    let rows_plain: std::collections::BTreeSet<i32> = ink_of(&px, 60).iter().map(|p| p.1).collect();
    px_fill(&px, 0x00FF_FFFF);
    g.select_object(dc, under).unwrap();
    assert_eq!(g.text_metrics(dc).unwrap().underlined, 255);
    g.text_out(dc, 2, 2, &s);
    let rows_under: std::collections::BTreeSet<i32> = ink_of(&px, 60).iter().map(|p| p.1).collect();
    // The new rows are below the baseline, inside the cell.
    let extra: Vec<_> = rows_under.difference(&rows_plain).collect();
    assert!(!extra.is_empty() && extra.iter().all(|&&y| y >= 2 + tm.ascent && y < 2 + tm.height), "{extra:?}");
}

#[test]
fn draw_text_aligns_ellipsizes_and_underlines_prefixes() {
    use crate::text::*;
    let Some(mut g) = gdi_with_fonts() else { return };
    let (dc, px) = white_canvas(&mut g, 160, 60);
    let f = u64::from(g.create_font(&logfont("Segoe UI", -15)));
    g.select_object(dc, f).unwrap();
    g.set_bk_mode(dc, TRANSPARENT);
    let mut r = Rect::new(10, 10, 150, 40);
    // DT_CENTER | DT_VCENTER | DT_SINGLELINE: centred in both directions.
    let s = utf16("Centre");
    let (cx, cy) = g.text_extent(dc, &s).unwrap();
    g.draw_text(dc, &s, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    let ink = ink_of(&px, 160);
    let x0 = 10 + (140 - cx) / 2;
    let y0 = 10 + (30 - cy) / 2;
    assert!(ink.iter().all(|&(x, y, _)| x >= x0 - 1 && x <= x0 + cx && y >= y0 && y < y0 + cy), "{ink:?}");
    // DT_END_ELLIPSIS in a narrow rectangle: the ink stays inside, the
    // modified string ends in dots.
    px_fill(&px, 0x00FF_FFFF);
    let long = utf16("A rather long caption");
    let mut r = Rect::new(10, 10, 80, 30);
    let modified = g
        .draw_text_ex(dc, &long, &mut r, DT_SINGLELINE | DT_END_ELLIPSIS | DT_MODIFYSTRING, DrawParams::default())
        .modified;
    let m = modified.expect("the text did not fit");
    assert!(m.ends_with(&utf16("...")) && m.len() < long.len());
    assert!(g.text_extent(dc, &m).unwrap().0 <= 70);
    assert!(ink_of(&px, 160).iter().all(|&(x, _, _)| (10..80).contains(&x)));
    // Text that fits is not modified.
    let mut r = Rect::new(10, 10, 150, 30);
    assert_eq!(
        g.draw_text_ex(
            dc,
            &utf16("Short"),
            &mut r,
            DT_SINGLELINE | DT_END_ELLIPSIS | DT_MODIFYSTRING,
            DrawParams::default()
        )
        .modified,
        None
    );
    // "&F" underlines F unless DT_HIDEPREFIX; DT_PREFIXONLY draws only the
    // underline.
    let count = |g: &mut Gdi, fmt: u32| {
        px_fill(&px, 0x00FF_FFFF);
        let mut r = Rect::new(10, 10, 150, 40);
        g.draw_text(dc, &utf16("&File"), &mut r, DT_SINGLELINE | fmt);
        ink_of(&px, 160)
    };
    let shown = count(&mut g, 0);
    let hidden = count(&mut g, DT_HIDEPREFIX);
    let only = count(&mut g, DT_PREFIXONLY);
    assert!(shown.len() > hidden.len());
    assert!(hidden.iter().map(|p| p.1).max() < shown.iter().map(|p| p.1).max(), "the underline is the lowest row");
    assert!(!only.is_empty() && only.len() < hidden.len());
    let f_width = g.text_extent(dc, &utf16("F")).unwrap().0;
    assert!(only.iter().all(|&(x, _, _)| (10..10 + f_width).contains(&x)));
}

#[test]
fn text_faces_widths_and_fallback() {
    let Some(mut g) = gdi_with_fonts() else { return };
    let dc = u64::from(g.create_compatible_dc(0));
    let f = u64::from(g.create_font(&logfont("Segoe UI", -12)));
    g.select_object(dc, f).unwrap();
    assert_eq!(g.text_face(dc).unwrap(), utf16("Segoe UI"));
    let (_, fit, running) = g.text_extent_ex(dc, &utf16("Hello"), 25).unwrap();
    assert_eq!(running.len(), 5);
    assert!(fit < 5 && running[fit - 1] <= 25 && running[fit] > 25);
    let w = g.char_widths(dc, 'a' as u32, 'c' as u32).unwrap();
    assert_eq!(w.len(), 3);
    assert_eq!(w[0], g.text_extent(dc, &utf16("a")).unwrap().0);
    assert!(g.char_widths(dc, 5, 4).is_none());
    // A GDI with no font files draws the bitmap stand-ins.
    let mut plain = Gdi::new();
    let dc2 = u64::from(plain.create_compatible_dc(0));
    let f2 = u64::from(plain.create_font(&logfont("Segoe UI", -12)));
    plain.select_object(dc2, f2).unwrap();
    assert_eq!(plain.text_metrics(dc2).unwrap().height, 12);
    assert_eq!(plain.text_face(dc2).unwrap(), utf16("Segoe UI"));
    // Stock fonts keep the sheets' bitmap faces whatever the store has.
    let stock = u64::from(g.stock(13));
    g.select_object(dc, stock).unwrap();
    assert_eq!(g.text_metrics(dc).unwrap().height, 16);
}

/// DeleteObject.yaml `selected_brush_pen`: what a DC draws with after its
/// selected brush or pen was deleted depends on whether it drew with it
/// before (or saved its state with it).
#[test]
fn a_deleted_selected_brush_or_pen_draws_by_its_use() {
    const PATCOPY: u32 = 0x00F0_0021;
    const BG: u32 = 0x0010_2030;
    // (used before the delete, SaveDC before / RestoreDC after, stock object selected between)
    for (used, save, stock_between) in
        [(false, false, false), (true, false, false), (false, true, false), (true, true, false), (false, true, true)]
    {
        let own = used || save;
        // Brush.
        let mut g = Gdi::new();
        let (dc, mut px) = dib32(&mut g, 8, 8);
        let br = u64::from(g.create_solid_brush(0x0060_4020));
        g.select_object(dc, br).unwrap();
        if used {
            g.pat_blt(dc, 0, 0, 8, 2, PATCOPY);
        }
        let saved = if save { g.save_dc(dc) } else { 0 };
        if stock_between {
            g.select_object(dc, u64::from(g.stock(BLACK_BRUSH))).unwrap();
        }
        assert!(g.delete_object(br));
        assert_eq!(g.object_type(br), OBJ_BRUSH);
        assert!(!g.delete_object(br), "a second delete fails");
        assert_eq!(g.object_bytes(br), None);
        if save {
            assert!(g.restore_dc(dc, saved));
        }
        assert_eq!(u64::from(g.current_object(dc, OBJ_BRUSH)), br);
        px.fill(BG);
        g.pat_blt(dc, 0, 0, 8, 4, PATCOPY);
        let rgn = u64::from(g.create_rect_rgn(Rect::new(0, 4, 8, 6)).unwrap());
        assert!(g.fill_rgn(dc, rgn, br));
        let case = (used, save, stock_between);
        assert_eq!(px[3 * 8 + 3], if own { 0x0020_4060 } else { 0x00FF_FFFF }, "PatBlt {case:?}");
        assert_eq!(px[5 * 8 + 3], if own { 0x0020_4060 } else { BG }, "FillRgn {case:?}");
        // Deselected: the brush is gone.
        g.select_object(dc, u64::from(g.stock(WHITE_BRUSH))).unwrap();
        assert_eq!(g.object_type(br), 0);
        assert_eq!(g.select_object(dc, br), Ok(0));

        // Pen.
        let mut g = Gdi::new();
        let (dc, mut px) = dib32(&mut g, 8, 8);
        let pen = u64::from(g.create_pen(0, 1, 0x0010_10E0));
        g.select_object(dc, pen).unwrap();
        if used {
            g.move_to(dc, 0, 1);
            g.line_to(dc, 8, 1);
        }
        let saved = if save { g.save_dc(dc) } else { 0 };
        if stock_between {
            g.select_object(dc, u64::from(g.stock(BLACK_PEN))).unwrap();
        }
        assert!(g.delete_object(pen));
        if save {
            assert!(g.restore_dc(dc, saved));
        }
        px.fill(BG);
        g.move_to(dc, 0, 3);
        g.line_to(dc, 8, 3);
        assert_eq!(px[3 * 8 + 3], if own { 0x00E0_1010 } else { 0 }, "LineTo {case:?}");
    }
    // A brush in two DCs, neither drew with it: white in both.
    let mut g = Gdi::new();
    let (a, mut pa) = dib32(&mut g, 2, 2);
    let (b, mut pb) = dib32(&mut g, 2, 2);
    let br = u64::from(g.create_solid_brush(0x0050_9030));
    g.select_object(a, br).unwrap();
    g.select_object(b, br).unwrap();
    assert!(g.delete_object(br));
    pa.fill(BG);
    pb.fill(BG);
    g.pat_blt(a, 0, 0, 2, 2, PATCOPY);
    g.pat_blt(b, 0, 0, 2, 2, PATCOPY);
    assert_eq!((pa[0], pb[0]), (0x00FF_FFFF, 0x00FF_FFFF));
    // Not into a third DC while it waits.
    let c = u64::from(g.create_compatible_dc(0));
    assert_eq!(g.select_object(c, br), Ok(0));
    // A deleted region: FillRgn fails; a brush deleted before it was
    // selected fills white through FillRect.
    let rgn = u64::from(g.create_rect_rgn(Rect::new(0, 0, 1, 1)).unwrap());
    let live = u64::from(g.create_solid_brush(0x0000_00FF));
    assert!(g.delete_object(rgn));
    assert!(!g.fill_rgn(a, rgn, live));
    assert!(g.delete_object(live));
    pa.fill(BG);
    assert!(g.fill_rect(a, Rect::new(0, 0, 2, 2), live));
    assert_eq!(pa[0], 0x00FF_FFFF);
    // A font's GetObject fills a buffer larger than LOGFONTW.
    let font = u64::from(g.create_font(&[0u8; 92]));
    assert_eq!(g.object_bytes_for(font, 128).map(|b| b.len().min(128)), Some(128));
    assert_eq!(g.object_bytes(font).map(|b| b.len()), Some(92));
}

/// GetSystemMetrics.yaml `device_caps_screen`.
#[test]
fn device_caps_are_the_screens_for_every_dc() {
    let mut g = Gdi::new();
    g.screen = Screen { width: 1920, height: 1080 };
    let screen = u64::from(g.create_display_dc(0, false));
    let window = u64::from(g.create_display_dc(0x1234, false));
    let (mem, _px) = dib32(&mut g, 4, 4);
    let want = [
        (0, 16384),
        (2, 1),
        (4, 508),
        (6, 286),
        (8, 1920),
        (10, 1080),
        (12, 32),
        (14, 1),
        (16, -1),
        (18, -1),
        (20, 0),
        (22, 0),
        (24, -1),
        (26, 0),
        (28, 511),
        (30, 254),
        (32, 255),
        (34, 30727),
        (36, 1),
        (38, 32409),
        (40, 36),
        (42, 36),
        (44, 51),
        (88, 96),
        (90, 96),
        (104, 0),
        (106, 20),
        (108, 24),
        (110, 0),
        (114, 0),
        (116, 60),
        (117, 1080),
        (118, 1920),
        (119, 1),
        (120, 0),
        (121, 2),
        (-1, 0),
        (3, 0),
        (1000, 0),
    ];
    for dc in [screen, window, mem] {
        for (index, value) in want {
            assert_eq!(g.device_caps(dc, index), Some(value), "index {index}");
        }
    }
    assert_eq!(g.device_caps(0, 8), None);
    assert_eq!(g.device_caps(0x1234_5678, 8), None);
}

/// BeginPaint.yaml `dc_value`: a window's consecutive common DCs have the
/// same handle value, in the default state each time.
#[test]
fn a_window_gets_its_released_dc_again() {
    let mut g = Gdi::new();
    let a = u64::from(g.create_display_dc(0x10, false));
    g.set_text_color(a, 0x123456);
    assert!(g.release_display_dc(a));
    assert_eq!(g.object_type(a), 0, "released: the handle names nothing");
    assert_eq!(g.display_hwnd(a), None);
    let again = u64::from(g.create_display_dc(0x10, false));
    assert_eq!(again, a);
    assert_eq!(g.text_color(again), 0, "the default state");
    assert!(g.release_display_dc(again));
    // The screen's DC in between does not disturb it.
    let screen = u64::from(g.create_display_dc(0, false));
    assert_ne!(screen, a);
    assert!(g.release_display_dc(screen));
    assert_eq!(u64::from(g.create_display_dc(0x10, false)), a);
    assert!(g.release_display_dc(a));
    // Another window's DC in between: no promise, and never the same DC twice at once.
    let b = u64::from(g.create_display_dc(0x20, false));
    let a2 = u64::from(g.create_display_dc(0x10, false));
    assert_ne!(b, a2);
    assert!(g.release_display_dc(b));
    assert!(g.release_display_dc(a2));
    // An own DC stays what it is.
    let own = u64::from(g.create_display_dc(0x30, true));
    assert!(g.release_display_dc(own));
    assert_eq!(g.display_hwnd(own), Some(0x30));
}

/// DeleteObject.yaml `palette`, `bitmaps_other_kinds`, `type_after_delete`;
/// CreateRectRgn.yaml `indirect_and_set`.
#[test]
fn palettes_and_styled_brushes_follow_the_delete_rules() {
    let mut g = Gdi::new();
    let (dc, _px) = dib32(&mut g, 2, 2);
    let stock = g.stock(DEFAULT_PALETTE);
    assert_eq!(g.current_object(dc, OBJ_PAL), stock);
    // A palette that is not selected: 1, and the type stays.
    let free = u64::from(g.create_palette());
    assert_eq!(g.object_type(free), OBJ_PAL);
    assert!(g.delete_object(free));
    assert_eq!(g.object_type(free), OBJ_PAL);
    assert!(!g.delete_object(free));
    // Selected: the delete fails and it stays; back to the stock one, it goes.
    let pal = u64::from(g.create_palette());
    assert_eq!(g.select_palette(dc, pal), stock);
    assert_eq!(u64::from(g.current_object(dc, OBJ_PAL)), pal);
    assert!(!g.delete_object(pal));
    assert_eq!(g.object_type(pal), OBJ_PAL);
    assert_eq!(u64::from(g.select_palette(dc, u64::from(stock))), pal);
    assert!(g.delete_object(pal));
    // Not a palette, not a DC: NULL.
    assert_eq!(g.select_palette(dc, u64::from(g.stock(WHITE_BRUSH))), 0);
    assert_eq!(g.select_palette(0x1234, u64::from(stock)), 0);
    // Hatch and pattern brushes are brushes; deleted, they keep their type.
    for style in [BS_HATCHED, BS_PATTERN, BS_DIBPATTERNPT] {
        let b = u64::from(g.create_styled_brush(style, 0x0000_FF00, 3));
        assert_eq!(g.object_type(b), OBJ_BRUSH);
        assert_eq!(g.object_bytes(b).map(|v| v[0]), Some(style as u8));
        assert!(g.delete_object(b));
        assert_eq!(g.object_type(b), OBJ_BRUSH);
        assert!(!g.delete_object(b));
    }
    // SetRectRgn: 0 for what is not a region, 1 for one deleted just before.
    let rgn = u64::from(g.create_rect_rgn(Rect::new(0, 0, 4, 4)).unwrap());
    assert!(g.is_region(rgn));
    assert!(!g.set_rect_rgn(u64::from(g.stock(WHITE_BRUSH)), Rect::new(0, 0, 1, 1)));
    assert!(!g.set_rect_rgn(0, Rect::new(0, 0, 1, 1)));
    assert!(g.delete_object(rgn));
    assert!(!g.is_region(rgn));
    assert!(g.set_rect_rgn(rgn, Rect::new(0, 0, 1, 1)));
    assert_eq!(g.rgn_box(rgn), None);
}

// ----- private fonts (`specs/windows/gdi32.dll`: the PolyTest fonts, which
// every test of the text sheets writes for itself).

use crate::testfonts as tfont;

/// A LOGFONTW for a face name with a height, weight, italic and quality.
fn lf_full(name: &str, height: i32, weight: i32, italic: bool, quality: u8) -> Vec<u8> {
    let mut b = logfont(name, height);
    b[16..20].copy_from_slice(&weight.to_le_bytes());
    b[20] = u8::from(italic);
    b[26] = quality;
    b
}

fn select(g: &mut Gdi, dc: u64, lf: &[u8]) -> u64 {
    let f = u64::from(g.create_font(lf));
    g.select_object(dc, f).unwrap();
    f
}

#[test]
fn memory_fonts_are_found_by_name_and_go_away() {
    let mut g = Gdi::new();
    let dc = u64::from(g.create_compatible_dc(0));
    let font = tfont::build(&tfont::MONO);
    // Before: a substitute (the stand-in family of the pitch asked for).
    select(&mut g, dc, &lf_full("PolyTest Mono", -16, 400, false, 3));
    assert_ne!(g.text_face(dc).unwrap(), utf16("PolyTest Mono"));
    let h = g.add_font_mem(&font).unwrap();
    // The existing font maps again (generation changed); lower case finds it.
    assert_eq!(g.text_face(dc).unwrap(), utf16("PolyTest Mono"));
    select(&mut g, dc, &lf_full("polytest mono", -16, 400, false, 3));
    assert_eq!(g.text_face(dc).unwrap(), utf16("PolyTest Mono"));
    let tm = g.text_metrics(dc).unwrap();
    assert_eq!((tm.height, tm.ascent, tm.ave_char_width, tm.pitch_and_family), (16, 12, 10, 0x36));
    assert_eq!(g.text_extent(dc, &utf16("AB gi")).unwrap(), (50, 16));
    // Added twice: usable until both are removed; the data was copied.
    let h2 = g.add_font_mem(&font).unwrap();
    assert_ne!(h, h2);
    assert!(g.remove_font_mem(u64::from(h)));
    assert_eq!(g.text_face(dc).unwrap(), utf16("PolyTest Mono"));
    assert!(!g.remove_font_mem(u64::from(h)));
    assert!(g.remove_font_mem(u64::from(h2)));
    assert_ne!(g.text_face(dc).unwrap(), utf16("PolyTest Mono"));
    assert!(!g.remove_font_mem(0x1234));
    assert!(g.add_font_mem(&font[..font.len() / 2]).is_none());
    assert!(g.add_font_mem(&[0u8; 600]).is_none());
    assert!(g.add_font_mem(&[]).is_none());
}

#[test]
fn a_family_picks_its_face_and_synthesises_the_rest() {
    let mut g = Gdi::new();
    let dc = u64::from(g.create_compatible_dc(0));
    for d in [&tfont::SANS, &tfont::SANS_BOLD, &tfont::SANS_ITALIC, &tfont::TALL] {
        g.add_font_file(&utf16(d.family)[..], 0x10, tfont::build(d));
    }
    let a = |g: &mut Gdi, dc: u64, name: &str, weight: i32, italic: bool| {
        select(g, dc, &lf_full(name, -16, weight, italic, 3));
        (g.text_extent(dc, &utf16("A")).unwrap().0, g.text_metrics(dc).unwrap())
    };
    // Weights 0..550 the Regular face, 600.. the Bold one.
    for w in [0, 100, 400, 550] {
        assert_eq!(a(&mut g, dc, "PolyTest Sans", w, false).0, 12);
    }
    for w in [600, 700, 900] {
        let (adv, tm) = a(&mut g, dc, "PolyTest Sans", w, false);
        assert_eq!((adv, tm.weight), (13, 700));
    }
    // Italic: the Italic face; italic and bold: the Bold face, slanted.
    let (adv, tm) = a(&mut g, dc, "PolyTest Sans", 400, true);
    assert_eq!((adv, tm.italic), (14, 255));
    assert_eq!(a(&mut g, dc, "PolyTest Sans", 700, true).0, 13);
    // A full name finds the face and is reported as asked.
    let (adv, tm) = a(&mut g, dc, "PolyTest Sans Bold", 400, false);
    assert_eq!((adv, tm.weight), (13, 700));
    assert_eq!(g.text_face(dc).unwrap(), utf16("PolyTest Sans Bold"));
    // Names that are not the family: no match.
    for n in ["PolyTest Sans ", " PolyTest Sans", "PolyTest  Sans", "PolyTest", "PolyTest Sans Extra"] {
        select(&mut g, dc, &lf_full(n, -16, 400, false, 3));
        assert_ne!(g.text_face(dc).unwrap(), utf16("PolyTest Sans"), "{n:?}");
    }
    // Tall has only a Regular face: bold and italic are synthesised, the
    // advance one pixel wider, the slant leaves it alone.
    assert_eq!(a(&mut g, dc, "PolyTest Tall", 700, false).0, 13);
    let (adv, tm) = a(&mut g, dc, "PolyTest Tall", 400, true);
    assert_eq!((adv, tm.italic), (12, 255));
    // Enumeration lists the faces of a family and nothing for a prefix.
    let faces = g.enum_fonts(&utf16("polytest sans"), 1);
    let names: Vec<_> = faces.iter().map(|f| f.full.clone()).collect();
    assert_eq!(names, [utf16("PolyTest Sans"), utf16("PolyTest Sans Bold"), utf16("PolyTest Sans Italic")]);
    assert_eq!(
        faces.iter().map(|f| (f.weight, f.italic, f.pitch_and_family())).collect::<Vec<_>>(),
        [(400, false, 2), (700, false, 2), (400, true, 2)]
    );
    assert!(g.enum_fonts(&utf16("PolyTest"), 1).is_empty());
    assert!(g.enum_fonts(&utf16("PolyTest Sans"), 2).is_empty());
    assert!(g.enum_fonts(&[], 1).len() > 10);
}

/// The ink pixels of a canvas, as (x, y).
fn ink(px: &[u32], w: i32, bg: u32) -> Vec<(i32, i32)> {
    px.iter().enumerate().filter(|(_, v)| **v != bg).map(|(i, _)| (i as i32 % w, i as i32 / w)).collect()
}

fn bounds(p: &[(i32, i32)]) -> (i32, i32, i32, i32) {
    (
        p.iter().map(|q| q.0).min().unwrap(),
        p.iter().map(|q| q.1).min().unwrap(),
        p.iter().map(|q| q.0).max().unwrap() + 1,
        p.iter().map(|q| q.1).max().unwrap() + 1,
    )
}

#[test]
fn bi_level_glyphs_are_exact_boxes_and_alignment_moves_the_cell() {
    let mut g = Gdi::new();
    assert!(g.add_font_mem(&tfont::build(&tfont::SANS)).is_some());
    let (dc, mut px) = dib32(&mut g, 100, 70);
    let bg = 0x0080_8080;
    px.fill(bg);
    select(&mut g, dc, &lf_full("PolyTest Sans", -16, 400, false, 3));
    g.set_text_color(dc, 0x00FF_FFFF);
    g.set_bk_mode(dc, TRANSPARENT);
    // TextOutW: 'A' is an 8x12 box 2 pixels right of the origin.
    assert!(g.text_out(dc, 50, 40, &utf16("A")));
    assert_eq!(bounds(&ink(&px, 100, bg)), (52, 40, 60, 52));
    assert_eq!(ink(&px, 100, bg).len(), 96);
    assert!(px.iter().all(|&v| v == bg || v == 0x00FF_FFFF), "bi-level: only the two colours");
    // 'B' 6x8, 'i' 2x12, 'g' 8x10 reaching 4 below the baseline.
    for (s, want) in [("B", (51, 44, 57, 52)), ("i", (51, 40, 53, 52)), ("g", (51, 46, 59, 56))] {
        px.fill(bg);
        g.text_out(dc, 50, 40, &utf16(s));
        let b = bounds(&ink(&px, 100, bg));
        assert_eq!((b.0, b.2), (want.0, want.2), "{s}");
        assert_eq!(b.3 - b.1, want.3 - want.1, "{s}");
    }
    // The cell of "AB" (20x16, ascent 12) at (50,40), OPAQUE.
    g.set_bk_mode(dc, OPAQUE);
    g.set_bk_color(dc, 0x0000_0080);
    for (align, cell) in [
        (0, (50, 40, 70, 56)),
        (2, (30, 40, 50, 56)),
        (6, (40, 40, 60, 56)),
        (8, (50, 24, 70, 40)),
        (24, (50, 28, 70, 44)),
    ] {
        px.fill(bg);
        g.set_text_align(dc, align);
        g.text_out(dc, 50, 40, &utf16("AB"));
        assert_eq!(bounds(&ink(&px, 100, bg)), cell, "align {align}");
    }
    // lpDx: "AAA" with 20,14,30 at x=4: boxes at 6-14, 26-34, 40-48.
    g.set_text_align(dc, 0);
    g.set_bk_mode(dc, TRANSPARENT);
    px.fill(bg);
    g.ext_text_out(dc, 4, 2, 0, None, &utf16("AAA"), Some(&[20, 14, 30]));
    let cols: std::collections::BTreeSet<i32> = ink(&px, 100, bg).iter().map(|p| p.0).collect();
    assert_eq!(cols.iter().copied().min(), Some(6));
    assert!(
        cols.contains(&13) && !cols.contains(&14) && cols.contains(&26) && cols.contains(&47) && !cols.contains(&48)
    );
    // ETO_GLYPH_INDEX: glyphs 5 and 6 are 'A' and 'B'.
    px.fill(bg);
    g.ext_text_out(dc, 4, 2, ETO_GLYPH_INDEX, None, &[5, 6], None);
    let with_glyphs = ink(&px, 100, bg);
    px.fill(bg);
    g.ext_text_out(dc, 4, 2, 0, None, &utf16("AB"), None);
    assert_eq!(with_glyphs, ink(&px, 100, bg));
    assert!(!g.ext_text_out(dc, 4, 2, u32::MAX, None, &utf16("A"), None));
}

#[test]
fn underline_strike_and_bold_synthesis_follow_the_font_file() {
    let mut g = Gdi::new();
    assert!(g.add_font_mem(&tfont::build(&tfont::SANS)).is_some());
    let (dc, mut px) = dib32(&mut g, 60, 40);
    let bg = 0x0080_8080;
    g.set_bk_mode(dc, TRANSPARENT);
    g.set_text_color(dc, 0x00FF_FFFF);
    let draw = |g: &mut Gdi, px: &mut Vec<u32>, lf: &[u8]| {
        px.fill(bg);
        select(g, dc, lf);
        g.text_out(dc, 4, 4, &utf16("AB g"));
        ink(px, 60, bg)
    };
    let plain = draw(&mut g, &mut px, &lf_full("PolyTest Sans", -16, 400, false, 3));
    assert_eq!(bounds(&plain), (6, 4, 39, 20));
    assert_eq!(plain.len(), 224);
    let mut lf = lf_full("PolyTest Sans", -16, 400, false, 3);
    lf[21] = 1;
    let under = draw(&mut g, &mut px, &lf);
    // The line is 2 below the baseline (row 18), 1 thick, over the whole advance.
    assert_eq!(bounds(&under), (4, 4, 40, 20));
    assert_eq!(under.len(), 252);
    assert!((4..40).all(|x| under.contains(&(x, 18))));
    let mut lf = lf_full("PolyTest Sans", -16, 400, false, 3);
    lf[22] = 1;
    let strike = draw(&mut g, &mut px, &lf);
    assert!((4..40).all(|x| strike.contains(&(x, 12))), "4 above the baseline");
    // Bold without a bold face: wider by a pixel each; the ink of 'A' too.
    let bold = draw(&mut g, &mut px, &lf_full("PolyTest Sans", -16, 700, false, 3));
    assert_eq!(bounds(&bold).0, 6);
    assert_eq!(g.text_extent(dc, &utf16("AB gi")).unwrap(), (45, 16));
    // Italic without an italic face: the ink reaches further right, the top stays.
    let slanted = draw(&mut g, &mut px, &lf_full("PolyTest Sans", -16, 400, true, 3));
    assert!(bounds(&slanted).2 > bounds(&plain).2 && bounds(&slanted).1 == bounds(&plain).1);
}

fn calc(g: &mut Gdi, dc: u64, s: &str, width: i32, fmt: u32) -> (i32, i32, i32) {
    let mut r = Rect::new(10, 10, 10 + width, 90);
    let n = g.draw_text(dc, &utf16(s), &mut r, fmt | crate::text::DT_CALCRECT);
    (n, r.right - r.left, r.bottom - r.top)
}

#[test]
fn draw_text_wraps_and_ellipsizes_like_the_sheet() {
    use crate::text::*;
    let mut g = Gdi::new();
    assert!(g.add_font_mem(&tfont::build(&tfont::SANS)).is_some());
    let dc = u64::from(g.create_compatible_dc(0));
    select(&mut g, dc, &lf_full("PolyTest Sans", -16, 400, false, 3));
    // dt_pf_layout: sizes of wrapped text, trailing blanks that fit count.
    assert_eq!(calc(&mut g, dc, "AB", 100, DT_SINGLELINE), (16, 20, 16));
    assert_eq!(calc(&mut g, dc, "AB\nAB", 100, 0), (32, 20, 32));
    assert_eq!(calc(&mut g, dc, "AB\r\nAB", 100, 0), (32, 20, 32));
    assert_eq!(calc(&mut g, dc, "AB\rAB", 100, 0), (32, 20, 32));
    assert_eq!(calc(&mut g, dc, "AB\nAB", 100, DT_SINGLELINE), (16, 40, 16));
    assert_eq!(calc(&mut g, dc, "AB\n", 100, 0), (32, 20, 32));
    assert_eq!(calc(&mut g, dc, "\n\n", 100, 0), (48, 0, 48));
    assert_eq!(calc(&mut g, dc, "AB   ", 100, 0), (16, 38, 16));
    assert_eq!(calc(&mut g, dc, "&A", 100, 0), (16, 12, 16));
    assert_eq!(calc(&mut g, dc, "&A", 100, DT_NOPREFIX), (16, 21, 16));
    assert_eq!(calc(&mut g, dc, "A\tB", 100, DT_EXPANDTABS), (16, 72, 16));
    assert_eq!(calc(&mut g, dc, "AAAA BBBB AAAA", 60, DT_WORDBREAK), (48, 54, 48));
    assert_eq!(calc(&mut g, dc, "AB AB AB AB", 60, DT_WORDBREAK), (32, 52, 32));
    assert_eq!(calc(&mut g, dc, "AAAA BBBB AAAA", 50, DT_WORDBREAK), (48, 48, 48));
    assert_eq!(calc(&mut g, dc, "AB AB AB AB", 50, DT_WORDBREAK), (32, 46, 32));
    assert_eq!(calc(&mut g, dc, "AB AB AB AB", 40, DT_WORDBREAK), (64, 26, 64));
    assert_eq!(calc(&mut g, dc, "AB AB AB AB", 24, DT_WORDBREAK), (64, 20, 64));
    assert_eq!(calc(&mut g, dc, "AAAAAAAAAA", 40, DT_WORDBREAK), (16, 120, 16));
    assert_eq!(calc(&mut g, dc, "      ", 20, DT_WORDBREAK), (32, 18, 32));
    assert_eq!(calc(&mut g, dc, "AB     AB", 30, DT_WORDBREAK), (48, 26, 48));
    assert_eq!(calc(&mut g, dc, "AAAA BBBB\nAAAA BBBB", 60, DT_WORDBREAK), (64, 54, 64));
    // The ellipsis rules of dt_pf_ellipsis, through DT_MODIFYSTRING.
    let ell = |g: &mut Gdi, s: &str, w: i32, fmt: u32| {
        let mut r = Rect::new(10, 10, 10 + w, 60);
        let out = g.draw_text_ex(dc, &utf16(s), &mut r, fmt | DT_MODIFYSTRING, DrawParams::default());
        out.modified.map_or_else(|| std::string::String::from(s), |m| std::string::String::from_utf16(&m).unwrap())
    };
    let sl = DT_SINGLELINE;
    assert_eq!(ell(&mut g, "AABBAABBAB", 100, sl | DT_END_ELLIPSIS), "AABBAABBAB");
    assert_eq!(ell(&mut g, "AABBAABBAB", 60, sl | DT_END_ELLIPSIS), "AABB...");
    assert_eq!(ell(&mut g, "AABBAABBAB", 50, sl | DT_END_ELLIPSIS), "AAB...");
    assert_eq!(ell(&mut g, "AABBAABBAB", 40, sl | DT_END_ELLIPSIS), "AA...");
    assert_eq!(ell(&mut g, "AABBAABBAB", 30, sl | DT_END_ELLIPSIS), "A...");
    assert_eq!(ell(&mut g, "AABBAABBAB", 0, sl | DT_END_ELLIPSIS), "A...");
    assert_eq!(ell(&mut g, "AB AB AB AB", 40, sl | DT_END_ELLIPSIS), "AB ...");
    assert_eq!(ell(&mut g, "gi.gi.gi", 30, sl | DT_END_ELLIPSIS), "gi....");
    assert_eq!(ell(&mut g, "AAAAAAAA BBBB AAAAAAAA", 80, sl | DT_WORD_ELLIPSIS), "AAAAA...");
    assert_eq!(ell(&mut g, "AAAA\\BBBB\\AAAA\\BBB.AB", 100, sl | DT_PATH_ELLIPSIS), "AA...\\BBB.AB");
    assert_eq!(ell(&mut g, "AAAA\\BBBB\\AAAA\\BBB.AB", 80, sl | DT_PATH_ELLIPSIS), "A...\\BBB.AB");
    assert_eq!(ell(&mut g, "AAAA\\BBBB\\AAAA\\BBB.AB", 20, sl | DT_PATH_ELLIPSIS), "...\\BBB.AB");
    assert_eq!(ell(&mut g, "AABBAABBAB", 50, sl | DT_PATH_ELLIPSIS), "AABBAABBAB");
    assert_eq!(ell(&mut g, "AABBAABB\\", 50, sl | DT_PATH_ELLIPSIS), "AABBAABB\\");
    // With a prefix, the buffer keeps the '&'.
    assert_eq!(ell(&mut g, "&AABBAABBAB", 50, sl | DT_END_ELLIPSIS), "&AAB...");
    assert_eq!(ell(&mut g, "&AABBAABBAB", 50, sl | DT_END_ELLIPSIS | DT_NOPREFIX), "&AA...");
    // END_ELLIPSIS|WORDBREAK cuts the last line laid out.
    let wb = DT_WORDBREAK | DT_END_ELLIPSIS;
    assert_eq!(ell(&mut g, "AAAA BBBB AAAA BBBB", 60, wb), "AAAA BBBB AAAA BBBB");
    assert_eq!(ell(&mut g, "AAAA BBBB AAAA BBBB", 30, wb), "AAAA BBBB AAAA BB...");
    let mut r = Rect::new(10, 10, 70, 42);
    let out = g.draw_text_ex(dc, &utf16("AAAA BBBB AAAA BBBB"), &mut r, wb | DT_MODIFYSTRING, DrawParams::default());
    assert_eq!(out.height, 48);
    assert_eq!(std::string::String::from_utf16(&out.modified.unwrap()).unwrap(), "AAAA BBBB AAAA...");
    // uiLengthDrawn: lines that start inside the rectangle are laid out.
    let mut r = Rect::new(10, 10, 130, 42);
    let out = g.draw_text_ex(dc, &utf16("AB\nAB\nAB"), &mut r, 0, DrawParams::default());
    assert_eq!((out.height, out.drawn), (48, 8));
    let mut r = Rect::new(10, 10, 50, 26);
    let out = g.draw_text_ex(dc, &utf16("AAAA BBBB AAAA"), &mut r, DT_WORDBREAK, DrawParams::default());
    assert_eq!((out.height, out.drawn), (32, 10));
}

#[test]
fn draw_text_cells_alignment_and_mnemonic_underline() {
    use crate::text::*;
    let mut g = Gdi::new();
    assert!(g.add_font_mem(&tfont::build(&tfont::SANS)).is_some());
    let (dc, mut px) = dib32(&mut g, 140, 90);
    let bg = 0x0080_8080;
    px.fill(bg);
    select(&mut g, dc, &lf_full("PolyTest Sans", -16, 400, false, 3));
    g.set_text_color(dc, 0x00FF_FFFF);
    g.set_bk_color(dc, 0x0000_0080);
    // Wrapped lines: the cells of left aligned lines include the blank that
    // fit, the centred and right aligned do not.
    let mut r = Rect::new(10, 10, 40, 60);
    g.draw_text(dc, &utf16("AB AB AB"), &mut r, DT_WORDBREAK);
    assert_eq!(bounds(&ink(&px, 140, bg)), (10, 10, 36, 58));
    px.fill(bg);
    g.draw_text(dc, &utf16("AB AB AB"), &mut r, DT_WORDBREAK | DT_CENTER);
    assert_eq!(bounds(&ink(&px, 140, bg)), (15, 10, 35, 58));
    px.fill(bg);
    g.draw_text(dc, &utf16("AB AB AB"), &mut r, DT_WORDBREAK | DT_RIGHT);
    assert_eq!(bounds(&ink(&px, 140, bg)), (20, 10, 40, 58));
    // The mnemonic underline: one row at baseline + 1, as wide as the character.
    px.fill(bg);
    g.set_bk_mode(dc, TRANSPARENT);
    let mut r = Rect::new(4, 4, 80, 40);
    g.draw_text(dc, &utf16("A&B"), &mut r, 0);
    let pts = ink(&px, 140, bg);
    // 'B' starts at 4 + 12; its underline is on row 4 + 12 + 1 = 17, 8 wide.
    assert!((16..24).all(|x| pts.contains(&(x, 17))));
    assert!(!pts.contains(&(15, 17)) && !pts.contains(&(24, 17)));
    // DT_HIDEPREFIX: no underline; DT_PREFIXONLY: only the underline.
    px.fill(bg);
    g.draw_text(dc, &utf16("A&B"), &mut r, DT_HIDEPREFIX);
    assert!(!ink(&px, 140, bg).contains(&(16, 17)));
    px.fill(bg);
    g.draw_text(dc, &utf16("A&B"), &mut r, DT_PREFIXONLY);
    assert_eq!(bounds(&ink(&px, 140, bg)), (16, 17, 24, 18));
}

#[test]
fn rotated_text_turns_about_the_reference_point() {
    let mut g = Gdi::new();
    assert!(g.add_font_mem(&tfont::build(&tfont::SANS)).is_some());
    let (dc, mut px) = dib32(&mut g, 80, 80);
    let bg = 0x0080_8080;
    g.set_bk_mode(dc, TRANSPARENT);
    g.set_text_color(dc, 0x00FF_FFFF);
    // `CreateFontIndirectW.yaml`: "AB" at (30,30), TA_LEFT|TA_TOP.
    for (esc, want) in [
        (0i32, (32, 30, 49, 42)),
        (900, (31, 11, 43, 28)),
        (1800, (11, 18, 28, 30)),
        (2700, (17, 32, 29, 49)),
        (-900, (17, 32, 29, 49)),
        (3600, (32, 30, 49, 42)),
    ] {
        px.fill(bg);
        let mut lf = lf_full("PolyTest Sans", -16, 400, false, 3);
        lf[8..12].copy_from_slice(&esc.to_le_bytes());
        select(&mut g, dc, &lf);
        g.text_out(dc, 30, 30, &utf16("AB"));
        assert_eq!(bounds(&ink(&px, 80, bg)), want, "escapement {esc}");
        // The unrotated width stays.
        assert_eq!(g.text_extent(dc, &utf16("AB")).unwrap(), (20, 16));
    }
}

#[test]
fn clear_type_text_filters_three_subpixels_per_pixel() {
    let mut g = Gdi::new();
    assert!(g.add_font_mem(&tfont::build(&tfont::SANS)).is_some());
    let (dc, mut px) = dib32(&mut g, 60, 30);
    let (bg, fg) = (0x0080_8080, 0x00FF_FFFF);
    g.set_bk_mode(dc, TRANSPARENT);
    g.set_text_color(dc, fg);
    let count = |g: &mut Gdi, px: &mut Vec<u32>, q: u8| {
        px.fill(bg);
        select(g, dc, &lf_full("PolyTest Sans", -16, 400, false, q));
        g.text_out(dc, 2, 2, &utf16("AB gi"));
        let pure = px.iter().filter(|&&v| v == fg).count();
        let other = px.iter().filter(|&&v| v != fg && v != bg).count();
        (pure, other)
    };
    // The box glyphs of `to_pfglyphs`: grey scale keeps the edges exact;
    // ClearType (and the default) spreads each edge over a pixel on both sides.
    assert_eq!(count(&mut g, &mut px, 4), (248, 0));
    assert_eq!(count(&mut g, &mut px, 5), (164, 168));
    assert_eq!(count(&mut g, &mut px, 0), (164, 168));
}

#[test]
fn clear_type_text_still_has_pixels_of_the_text_colour() {
    // to_ink: at ordinary sizes some pixel is the text colour exactly, with
    // the fonts of the store and with the built-in font.
    for fonts in [false, true] {
        let mut g = Gdi::new();
        if fonts {
            let Some(g2) = gdi_with_fonts() else { continue };
            g = g2;
        }
        for (name, h) in
            [("Arial", -16), ("Segoe UI", -12), ("Tahoma", -11), ("Courier New", -14), ("Times New Roman", -16)]
        {
            let (dc, mut px) = dib32(&mut g, 200, 40);
            let (bg, fg) = (0x0080_8080, 0x00FF_FFFF);
            px.fill(bg);
            g.set_bk_mode(dc, TRANSPARENT);
            g.set_text_color(dc, fg);
            select(&mut g, dc, &lf_full(name, h, 400, false, 5));
            g.text_out(dc, 2, 2, &utf16("The quick brown fox jumps"));
            let pure = px.iter().filter(|&&v| v == fg).count();
            assert!(pure > 0, "{name} {h} (store fonts {fonts})");
        }
    }
}

/// The columns of row `y` that are `v`, as runs (left, right).
fn runs(px: &[u32], w: i32, y: i32, v: u32) -> Vec<(i32, i32)> {
    let mut out: Vec<(i32, i32)> = Vec::new();
    for x in 0..w {
        if px[(y * w + x) as usize] == v {
            match out.last_mut() {
                Some(l) if l.1 == x => l.1 = x + 1,
                _ => out.push((x, x + 1)),
            }
        }
    }
    out
}

fn where_is(px: &[u32], w: i32, v: u32) -> Vec<(i32, i32)> {
    px.iter().enumerate().filter(|(_, p)| **p == v).map(|(i, _)| (i as i32 % w, i as i32 / w)).collect()
}

#[test]
fn ext_text_out_dx_glyph_indices_and_odd_input_follow_the_sheet() {
    let mut g = Gdi::new();
    assert!(g.add_font_mem(&tfont::build(&tfont::SANS)).is_some());
    let (w, h) = (100, 40);
    let (dc, mut px) = dib32(&mut g, w, h);
    let (bg, fg, bk) = (0x0080_8080, 0x00FF_FFFF, 0x0000_0080);
    select(&mut g, dc, &lf_full("PolyTest Sans", -16, 400, false, 3));
    g.set_text_color(dc, fg);
    g.set_bk_color(dc, 0x0080_0000);
    let aaa = utf16("AAA");
    // dx: the boxes of "AAA" with 20,14,30 at x=4, in a row inside the text.
    px.fill(bg);
    assert!(g.ext_text_out(dc, 4, 2, 0, None, &aaa, Some(&[20, 14, 30])));
    assert_eq!(runs(&px, w, 8, fg), [(6, 14), (26, 34), (40, 48)]);
    // The OPAQUE cell is as wide as the sum.
    assert_eq!(bounds(&ink(&px, w, bg)), (4, 2, 68, 18));
    assert_eq!(bounds(&where_is(&px, w, bk)), (4, 2, 68, 18));
    // 0,0,0: on top of each other, no cell.
    px.fill(bg);
    g.ext_text_out(dc, 4, 2, 0, None, &aaa, Some(&[0, 0, 0]));
    assert_eq!(runs(&px, w, 8, fg), [(6, 14)]);
    assert!(where_is(&px, w, bk).is_empty());
    // Negative values move back.
    px.fill(bg);
    g.set_bk_mode(dc, TRANSPARENT);
    g.ext_text_out(dc, 40, 2, 0, None, &aaa, Some(&[-12, -12, 5]));
    assert_eq!(runs(&px, w, 8, fg), [(18, 26), (30, 38), (42, 50)]);
    g.set_bk_mode(dc, OPAQUE);
    // TA_RIGHT: the sum ends at x; TA_CENTER: centred about it.
    px.fill(bg);
    g.set_text_align(dc, TA_RIGHT);
    g.ext_text_out(dc, 90, 2, 0, None, &aaa, Some(&[20, 14, 30]));
    assert_eq!(runs(&px, w, 8, fg)[0].0, 90 - 64 + 2);
    assert_eq!(bounds(&where_is(&px, w, bk)), (26, 2, 90, 18));
    px.fill(bg);
    g.set_text_align(dc, TA_CENTER);
    g.ext_text_out(dc, 50, 2, 0, None, &aaa, Some(&[20, 14, 30]));
    assert_eq!(bounds(&where_is(&px, w, bk)), (18, 2, 82, 18));
    // TA_UPDATECP: the current position moves by the sum, the arguments are ignored.
    for (align, start, end, cell) in [
        (TA_UPDATECP, 10, 74, (10, 74)),
        (TA_UPDATECP | TA_RIGHT, 80, 16, (16, 80)),
        (TA_UPDATECP | TA_CENTER, 50, 50, (18, 82)),
    ] {
        px.fill(bg);
        g.set_text_align(dc, align);
        g.move_to(dc, start, 20);
        g.ext_text_out(dc, 1, 1, 0, None, &aaa, Some(&[20, 14, 30]));
        assert_eq!(g.current_position(dc), Some((end, 20)), "align {align}");
        let b = bounds(&where_is(&px, w, bk));
        assert_eq!((b.0, b.1, b.2, b.3), (cell.0, 20, cell.1, 36), "align {align}");
    }
    g.set_text_align(dc, 0);
    g.move_to(dc, 7, 9);
    g.ext_text_out(dc, 1, 1, 0, None, &aaa, None);
    assert_eq!(g.current_position(dc), Some((7, 9)));
    // An empty string with TA_UPDATECP moves nothing.
    g.set_text_align(dc, TA_UPDATECP);
    assert!(g.ext_text_out(dc, 1, 1, 0, None, &[], None));
    assert_eq!(g.current_position(dc), Some((7, 9)));
    g.set_text_align(dc, 0);
    // Glyph indices: an index beyond the font is .notdef, as a character the font lacks.
    g.set_bk_mode(dc, TRANSPARENT);
    px.fill(bg);
    g.ext_text_out(dc, 4, 2, ETO_GLYPH_INDEX, None, &[999], None);
    let beyond = ink(&px, w, bg);
    px.fill(bg);
    g.ext_text_out(dc, 4, 2, 0, None, &utf16("Z"), None);
    assert!(!beyond.is_empty());
    assert_eq!(beyond, ink(&px, w, bg));
    px.fill(bg);
    g.ext_text_out(dc, 4, 2, ETO_GLYPH_INDEX, None, &[0], None);
    assert_eq!(beyond, ink(&px, w, bg));
    // Glyph indices with distances and an opaque rectangle.
    px.fill(bg);
    g.ext_text_out(dc, 4, 2, ETO_GLYPH_INDEX | ETO_OPAQUE, Some(Rect::new(0, 0, 60, 30)), &[5, 5], Some(&[30, 0]));
    assert_eq!(runs(&px, w, 8, fg), [(6, 14), (36, 44)]);
    assert_eq!(bounds(&ink(&px, w, bg)), (0, 0, 60, 30));
    // Option bits that do not exist are ignored.
    px.fill(bg);
    g.ext_text_out(dc, 4, 2, 0, None, &utf16("AB"), None);
    let plain = px.clone();
    for bits in [0x1, 0x8, 0x8000, 0x1 | 0x8 | 0x8000] {
        px.fill(bg);
        assert!(g.ext_text_out(dc, 4, 2, bits, None, &utf16("AB"), None));
        assert_eq!(px, plain, "bits {bits:#x}");
        px.fill(bg);
        assert!(g.ext_text_out(dc, 4, 2, bits, Some(Rect::new(0, 0, 8, 8)), &utf16("AB"), None));
        assert_eq!(px, plain, "bits {bits:#x} with a rectangle");
    }
    assert!(!g.ext_text_out(dc, 4, 2, u32::MAX, Some(Rect::new(0, 0, 8, 8)), &utf16("AB"), None));
    // 9000 characters draw across the whole surface.
    g.set_bk_mode(dc, OPAQUE);
    px.fill(bg);
    let long = vec![u16::from(b'A'); 9000];
    assert!(g.ext_text_out(dc, 0, 0, 0, None, &long, None));
    assert_eq!(bounds(&ink(&px, w, bg)), (0, 0, 100, 16));
    assert_eq!(runs(&px, w, 4, fg).len(), 9);
    // Off the surface: clipped; entirely off: nothing.
    px.fill(bg);
    g.ext_text_out(dc, -5, -3, 0, None, &utf16("AB"), None);
    assert_eq!(bounds(&ink(&px, w, bg)), (0, 0, 15, 13));
    px.fill(bg);
    g.ext_text_out(dc, 200, 200, 0, None, &utf16("AB"), None);
    g.ext_text_out(dc, -100, 10, ETO_CLIPPED, Some(Rect::new(0, 0, 10, 10)), &utf16("AB"), None);
    assert!(ink(&px, w, bg).is_empty());
}

#[test]
fn stock_fonts_have_the_face_names_of_the_sheet() {
    let mut g = Gdi::new();
    let dc = u64::from(g.create_compatible_dc(0));
    assert_eq!(g.text_face(dc).unwrap(), utf16("System"), "a new DC");
    // OEM_FIXED_FONT 10, ANSI_FIXED_FONT 11, ANSI_VAR_FONT 12, SYSTEM_FONT 13, DEFAULT_GUI_FONT 17.
    for (stock, name) in
        [(10, "Terminal"), (11, "Courier"), (12, "MS Sans Serif"), (13, "System"), (17, "MS Shell Dlg")]
    {
        let f = u64::from(g.stock(stock));
        g.select_object(dc, f).unwrap();
        assert_eq!(g.text_face(dc).unwrap(), utf16(name), "stock font {stock}");
    }
    assert!(g.text_face(0).is_none());
}

#[test]
fn draw_text_tab_stops_and_ellipsis_flags_follow_learn() {
    use crate::text::*;
    let mut g = Gdi::new();
    assert!(g.add_font_mem(&tfont::build(&tfont::SANS)).is_some());
    let (w, h) = (140, 90);
    let (dc, mut px) = dib32(&mut g, w, h);
    let (bg, fg) = (0x0080_8080, 0x00FF_FFFF);
    select(&mut g, dc, &lf_full("PolyTest Sans", -16, 400, false, 3));
    g.set_text_color(dc, fg);
    g.set_bk_mode(dc, TRANSPARENT);
    // DrawTextW's DT_TABSTOP: bits 8 to 15 are the tab stop in average widths (8 pixels each), and
    // not DT_NOCLIP (1), DT_EXTERNALLEADING (2), DT_CALCRECT (4), DT_NOPREFIX (8) or DT_END_ELLIPSIS (128).
    for (stop, b_at) in [(1, 16), (2, 16), (4, 32), (8, 64), (0, 64), (128, 1024)] {
        px.fill(bg);
        let mut r = Rect::new(0, 0, 130, 40);
        let n = g.draw_text(dc, &utf16("&A\tB"), &mut r, DT_EXPANDTABS | DT_TABSTOP | (stop << 8));
        assert_eq!((n, r), (16, Rect::new(0, 0, 130, 40)), "tab stop {stop}: drawn, not measured");
        let mut want = vec![(2, 10)];
        if b_at < w {
            want.push((b_at + 1, b_at + 7));
        }
        assert_eq!(runs(&px, w, 10, fg), want, "tab stop {stop}");
        assert_eq!(runs(&px, w, 13, fg), [(0, 12)], "tab stop {stop}: the mnemonic underline");
    }
    // DrawTextExW takes the tab stop from its parameters: there the bits are the flags they name.
    let mut r = Rect::new(0, 0, 130, 40);
    let p = DrawParams { tab_chars: 4, ..DrawParams::default() };
    let out = g.draw_text_ex(dc, &utf16("A\tB"), &mut r, DT_EXPANDTABS | DT_TABSTOP | DT_CALCRECT, p);
    assert_eq!((out.height, r), (16, Rect::new(0, 0, 40, 16)));
    let mut r = Rect::new(0, 0, 130, 40);
    let out =
        g.draw_text_ex(dc, &utf16("A\tB"), &mut r, DT_EXPANDTABS | DT_TABSTOP | DT_CALCRECT, DrawParams::default());
    assert_eq!((out.height, r), (16, Rect::new(0, 0, 72, 16)), "iTabLength 0: the default of 8");
    // Ellipsis flags on several lines, through DT_MODIFYSTRING.
    let ell = |g: &mut Gdi, s: &str, w: i32, h: i32, fmt: u32| {
        let mut r = Rect::new(10, 10, 10 + w, 10 + h);
        let out = g.draw_text_ex(dc, &utf16(s), &mut r, fmt | DT_MODIFYSTRING, DrawParams::default());
        out.modified.map_or_else(|| std::string::String::from(s), |m| std::string::String::from_utf16(&m).unwrap())
    };
    // DT_WORD_ELLIPSIS: every word that is too wide, on its line.
    let words = "AAAAAAAA BBBB AAAAAAAA";
    assert_eq!(ell(&mut g, words, 80, 60, DT_WORDBREAK | DT_WORD_ELLIPSIS), "AAAAA... BBBB AAAAA...");
    assert_eq!(ell(&mut g, words, 80, 60, DT_SINGLELINE | DT_WORD_ELLIPSIS), "AAAAA...");
    assert_eq!(ell(&mut g, "AAAAAAAA\nAB\nAAAAAAAA", 80, 60, DT_WORD_ELLIPSIS), "AAAAA...\nAB\nAAAAA...");
    // DT_END_ELLIPSIS: only the end of the text; a wide line before it is clipped without dots.
    assert_eq!(ell(&mut g, "AAAAAAAA\nAB\nAAAAAAAA", 80, 60, DT_END_ELLIPSIS), "AAAAAAAA\nAB\nAAAAA...");
    assert_eq!(ell(&mut g, "AAAAAAAA\nAB", 80, 60, DT_END_ELLIPSIS), "AAAAAAAA\nAB");
    assert_eq!(ell(&mut g, words, 80, 60, DT_WORDBREAK | DT_END_ELLIPSIS), "AAAAAAAA BBBB AAAAA...");
    // Both: every wide word, and the end of the text on the last line that fits.
    assert_eq!(ell(&mut g, words, 80, 32, DT_WORDBREAK | DT_END_ELLIPSIS | DT_WORD_ELLIPSIS), "AAAAA... BBBB AAAAA...");
    assert_eq!(ell(&mut g, "AAAA BBBB AAAA BBBB", 60, 32, DT_WORDBREAK | DT_END_ELLIPSIS), "AAAA BBBB AAAA...");
    // DT_PATH_ELLIPSIS keeps what follows the last backslash, on every line.
    assert_eq!(ell(&mut g, "AAAA\\BBBB\\AAAA\\BBB.AB\nAB", 100, 60, DT_PATH_ELLIPSIS), "AA...\\BBB.AB\nAB");
}
