use super::*;
use std::vec;

const MODE: Mode = Mode { width: 1280, height: 800, refresh_mhz: 60_000 };

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[test]
fn request_numbers_are_the_headers() {
    // linux/fb.h, printed by a C program on x86-64.
    assert_eq!(FBIOGET_VSCREENINFO, 0x4600);
    assert_eq!(FBIOPUT_VSCREENINFO, 0x4601);
    assert_eq!(FBIOGET_FSCREENINFO, 0x4602);
    assert_eq!(FBIOPAN_DISPLAY, 0x4606);
    assert_eq!(FBIOBLANK, 0x4611);
    // FBIO_WAITFORVSYNC is _IOW('F', 0x20, __u32) = 0x40044620: the same
    // encoding as Poly's damage request.
    assert_eq!(ioc_write(b'F', 0x20, 4), 0x4004_4620);
    assert_eq!(FBIO_POLY_DAMAGE, 0x4010_4680);
}

#[test]
fn variable_screen_information_layout() {
    let v = MODE.var();
    let b = v.to_bytes();
    // Offsets from offsetof() on x86-64: bits_per_pixel 24, red 32, transp
    // 68, nonstd 80, activate 84, height 88, pixclock 100, sync 128, vmode
    // 132, rotate 136, colorspace 140, reserved 144; 160 bytes in all.
    assert_eq!((u32_at(&b, 0), u32_at(&b, 4), u32_at(&b, 8), u32_at(&b, 12)), (1280, 800, 1280, 800));
    assert_eq!(u32_at(&b, 24), 32);
    assert_eq!((u32_at(&b, 32), u32_at(&b, 36)), (16, 8), "red at bit 16");
    assert_eq!((u32_at(&b, 44), u32_at(&b, 48)), (8, 8), "green at bit 8");
    assert_eq!((u32_at(&b, 56), u32_at(&b, 60)), (0, 8), "blue at bit 0");
    assert_eq!(u32_at(&b, 72), 0, "no alpha channel");
    assert_eq!((u32_at(&b, 88), u32_at(&b, 92)), (u32::MAX, u32::MAX), "size unknown");
    // 1e12 ps / (1280 * 800 * 60) = 16276 ps.
    assert_eq!(u32_at(&b, 100), 16_276);
    assert!(b[144..].iter().all(|&x| x == 0));
    assert_eq!(VarScreenInfo::from_bytes(&b), v);
}

#[test]
fn fixed_screen_information_layout() {
    let f = MODE.fix("poly-virtio-gpu-and-more", 0x1234_5678_9000);
    let b = f.to_bytes();
    assert_eq!(&b[..16], b"poly-virtio-gpu\0", "15 bytes and a NUL");
    assert_eq!(u64::from_le_bytes(b[16..24].try_into().unwrap()), 0x1234_5678_9000);
    assert_eq!(u32_at(&b, 24), 1280 * 800 * 4, "smem_len");
    assert_eq!(u32_at(&b, 28), FB_TYPE_PACKED_PIXELS);
    assert_eq!(u32_at(&b, 36), FB_VISUAL_TRUECOLOR);
    assert_eq!(u32_at(&b, 48), 1280 * 4, "line_length at 48");
    assert!(b[56..].iter().all(|&x| x == 0), "no MMIO, acceleration or capabilities");
}

#[test]
fn ioctls() {
    let mut d = Device::new(MODE, "polyfb");
    let r = d.ioctl(FBIOGET_VSCREENINFO, 0, &[]).unwrap();
    assert_eq!(r.out.len(), VAR_SIZE);
    assert_eq!(r.action, Action::None);
    let r = d.ioctl(FBIOGET_FSCREENINFO, 0, &[]).unwrap();
    assert_eq!(r.out.len(), FIX_SIZE);
    assert_eq!(Device::input_len(0x4604), None, "no colour maps");
    assert_eq!(d.ioctl(0x4604, 0, &[]), Err(Error::NoSuchRequest));

    // Panning to the origin shows the frame; anywhere else there is nothing.
    let mut v = MODE.var();
    assert_eq!(d.ioctl(FBIOPAN_DISPLAY, 0, &v.to_bytes()).unwrap().action, Action::Show(MODE.bounds()));
    v.yoffset = 800;
    assert_eq!(d.ioctl(FBIOPAN_DISPLAY, 0, &v.to_bytes()), Err(Error::Invalid));
    assert_eq!(d.ioctl(FBIOPAN_DISPLAY, 0, &[0; 8]), Err(Error::Invalid), "short input");

    // Setting the mode there is works and reads it back; another does not.
    let mut want = VarScreenInfo { xres: 1280, yres: 800, bits_per_pixel: 32, activate: 0, ..Default::default() };
    let r = d.ioctl(FBIOPUT_VSCREENINFO, 0, &want.to_bytes()).unwrap();
    assert_eq!(VarScreenInfo::from_bytes(&r.out.clone().try_into().unwrap()), MODE.var());
    want.bits_per_pixel = 16;
    assert_eq!(d.ioctl(FBIOPUT_VSCREENINFO, 0, &want.to_bytes()), Err(Error::Invalid));
    want.bits_per_pixel = 0;
    want.xres = 640;
    assert_eq!(d.ioctl(FBIOPUT_VSCREENINFO, 0, &want.to_bytes()), Err(Error::Invalid));

    // Blanking takes the level by value.
    assert_eq!(d.ioctl(FBIOBLANK, 4, &[]).unwrap().action, Action::None);
    assert_eq!(d.blank, 4);
    assert_eq!(d.ioctl(FBIOBLANK, 5, &[]), Err(Error::Invalid));
    assert_eq!(d.ioctl(FBIOBLANK, 0, &[]).unwrap().action, Action::Show(MODE.bounds()));
}

#[test]
fn damage_is_clipped_to_the_screen() {
    let mut d = Device::new(MODE, "polyfb");
    let show = |d: &mut Device, r: Rect| d.ioctl(FBIO_POLY_DAMAGE, 0, &r.to_damage()).unwrap().action;
    let r = Rect { x: 10, y: 20, width: 30, height: 40 };
    assert_eq!(show(&mut d, r), Action::Show(r));
    assert_eq!(
        show(&mut d, Rect { x: 1270, y: 790, width: 100, height: 100 }),
        Action::Show(Rect { x: 1270, y: 790, width: 10, height: 10 })
    );
    assert_eq!(show(&mut d, Rect { x: 2000, y: 0, width: 10, height: 10 }), Action::None);
    assert_eq!(show(&mut d, Rect { x: u32::MAX, y: u32::MAX, width: u32::MAX, height: 1 }), Action::None);
    assert_eq!(show(&mut d, Rect { x: 0, y: 0, width: 0, height: 5 }), Action::None);
}

#[test]
fn spans_stop_at_page_boundaries() {
    let mut got = vec![];
    // Row 0 of a 1280-pixel screen: 5120 bytes from 0, cut at 4096.
    spans(Rect { x: 0, y: 0, width: 1280, height: 1 }, 5120, 4096, |s| got.push(s));
    assert_eq!(got, [Span { offset: 0, len: 4096 }, Span { offset: 4096, len: 1024 }]);
    got.clear();
    // Two rows of 10 pixels at x = 1020: the first row crosses 4096.
    spans(Rect { x: 1020, y: 0, width: 10, height: 2 }, 5120, 4096, |s| got.push(s));
    assert_eq!(
        got,
        [Span { offset: 4080, len: 16 }, Span { offset: 4096, len: 24 }, Span { offset: 5120 + 4080, len: 40 },]
    );
    // Every byte of a rectangle exactly once.
    let mut total = 0;
    spans(Rect { x: 3, y: 5, width: 700, height: 9 }, 5120, 4096, |s| {
        assert!(s.offset / 4096 == (s.offset + s.len - 1) / 4096);
        total += s.len;
    });
    assert_eq!(total, 700 * 9 * 4);
}

#[test]
fn rows_a_write_touches() {
    assert_eq!(rows_of(&MODE, 0, 5120), Some(Rect { x: 0, y: 0, width: 1280, height: 1 }));
    assert_eq!(rows_of(&MODE, 5119, 2), Some(Rect { x: 0, y: 0, width: 1280, height: 2 }));
    assert_eq!(rows_of(&MODE, 5120 * 799, 10_000), Some(Rect { x: 0, y: 799, width: 1280, height: 1 }));
    assert_eq!(rows_of(&MODE, 5120 * 800, 4), None);
    assert_eq!(rows_of(&MODE, 0, 0), None);
}
