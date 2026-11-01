//! `/dev/fb0`: the Linux framebuffer device over the kernel's display
//! (ADR 0107), so that a Linux-personality program (the compositor) can
//! draw on the screen. The interface itself (screen information, requests,
//! damage) is `poly-fbdev`'s; this file connects it to memory and the
//! display.
//!
//! - **Memory.** A program maps a *shadow* frame buffer: an [`AnonShared`]
//!   object of `width * height * 4` bytes, the same memory object shared
//!   anonymous mappings use, so `mmap(MAP_SHARED)` of the device maps its
//!   pages through the ordinary fault path (`linux::services`'s `mmap`
//!   asks [`page_source`]). Its pages are made at the first open and kept,
//!   so copying out of them never allocates.
//! - **Showing.** Nothing reaches the screen until the program says so:
//!   `FBIOPAN_DISPLAY` (offset 0) shows the whole frame, Poly's
//!   `FBIO_POLY_DAMAGE` one rectangle, a `write` the rows it wrote. Each
//!   copies that rectangle from the shadow pages into the display's back
//!   buffer and flushes it ([`display::draw`]): on a virtio GPU one
//!   `TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH` of the rectangle, on Bochs
//!   VBE a copy into the linear frame buffer. The same code works on both.
//! - **Size.** The mode is the display's when the device is installed.
//!   Should the display change size later, copies are clipped to the
//!   smaller of the two.
//!
//! Character device 29, 0 (fbdev(4)), mode 0660 root:root (Linux uses
//! group `video`).

use crate::display;
use crate::sync::SpinLock;
use crate::user::{AnonShared, PageSource};
use crate::{arch, kprintln};
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_fbdev::{Action, Device, Mode, Rect};
use poly_mem::PAGE_SIZE;
use poly_vfs::device::CharDevice;
use poly_vfs::{At, Cred, DeviceId, Errno, FileType, FsContext, Vfs};

/// `/dev/fb0`.
pub const FB0: DeviceId = DeviceId::new(poly_fbdev::MAJOR, 0);

/// The frame buffer device.
pub struct FbDevice {
    device: SpinLock<Device>,
    memory: Arc<AnonShared>,
    /// Physical address of every page of `memory`, made at the first open.
    pages: SpinLock<Vec<u64>>,
    /// Keeps the pages' frames (they belong to `memory` as well).
    frames: SpinLock<Vec<Arc<crate::user::memory::SharedFrame>>>,
}

impl core::fmt::Debug for FbDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FbDevice").field("mode", &self.device.lock().mode).finish_non_exhaustive()
    }
}

static FB: SpinLock<Option<Arc<FbDevice>>> = SpinLock::new(None);

impl FbDevice {
    fn new(mode: Mode, id: &'static str) -> FbDevice {
        FbDevice {
            device: SpinLock::new(Device::new(mode, id)),
            memory: Arc::new(AnonShared::new(mode.size())),
            pages: SpinLock::new(Vec::new()),
            frames: SpinLock::new(Vec::new()),
        }
    }

    pub fn mode(&self) -> Mode {
        self.device.lock().mode
    }

    /// Make every page of the shadow memory (once); false if memory ran
    /// out.
    fn populate(&self) -> bool {
        if !self.pages.lock().is_empty() {
            return true;
        }
        let n = self.memory.len().div_ceil(PAGE_SIZE);
        let mut frames = Vec::with_capacity(n as usize);
        for i in 0..n {
            match self.memory.shared_frame(i * PAGE_SIZE) {
                Some(Ok(f)) => frames.push(f),
                _ => return false,
            }
        }
        let mut pages = self.pages.lock();
        if pages.is_empty() {
            *pages = frames.iter().map(|f| f.phys()).collect();
            *self.frames.lock() = frames;
        }
        true
    }

    /// The bytes at `offset` of shadow page `page` (through the direct map).
    fn page_ptr(pages: &[u64], offset: usize) -> Option<*mut u8> {
        let phys = *pages.get(offset / PAGE_SIZE as usize)?;
        let virt = arch::phys_to_virt(phys)?;
        Some((virt as usize + offset % PAGE_SIZE as usize) as *mut u8)
    }

    /// Copy rectangle `r` of the shadow into the display's back buffer and
    /// put it on the screen.
    pub fn show(&self, r: Rect) {
        let mode = self.mode();
        let pages = self.pages.lock().clone();
        if pages.is_empty() {
            return;
        }
        display::draw(|canvas| {
            let bounds =
                Rect { x: 0, y: 0, width: canvas.width().min(mode.width), height: canvas.height().min(mode.height) };
            let r = r.clip(bounds)?;
            let (src_stride, dst_stride) = (mode.line_length() as usize, canvas.stride());
            let dst = canvas.as_bytes_mut();
            poly_fbdev::spans(r, src_stride, PAGE_SIZE as usize, |s| {
                let (y, x_bytes) = (s.offset / src_stride, s.offset % src_stride);
                let at = y * dst_stride + x_bytes;
                let (Some(src), Some(out)) = (Self::page_ptr(&pages, s.offset), dst.get_mut(at..at + s.len)) else {
                    return;
                };
                // SAFETY: `src` points `s.len` bytes into one page of the
                // shadow (a span never crosses a page), which `self.frames`
                // keeps allocated, through the direct map. A program may be
                // writing those bytes meanwhile; bytes have no invalid
                // values, so the copy sees old or new pixels.
                unsafe { core::ptr::copy_nonoverlapping(src, out.as_mut_ptr(), s.len) };
            });
            Some(poly_gfx::Rect::new(r.x as i32, r.y as i32, r.width, r.height))
        });
    }

    /// Copy between the shadow at `offset` and `buf` (`to_shadow`: into the
    /// shadow); returns how many bytes.
    fn transfer(&self, offset: u64, buf: &mut [u8], to_shadow: bool) -> usize {
        let size = self.memory.len();
        if offset >= size {
            return 0;
        }
        let n = buf.len().min((size - offset) as usize);
        let pages = self.pages.lock().clone();
        let mut done = 0;
        while done < n {
            let at = offset as usize + done;
            let len = (PAGE_SIZE as usize - at % PAGE_SIZE as usize).min(n - done);
            let Some(p) = Self::page_ptr(&pages, at) else { break };
            let chunk = &mut buf[done..done + len];
            // SAFETY: `p` points `len` bytes into one shadow page, kept
            // allocated by `self.frames`, through the direct map; races
            // with a program's writes only mix old and new bytes.
            unsafe {
                if to_shadow {
                    core::ptr::copy_nonoverlapping(chunk.as_ptr(), p, len);
                } else {
                    core::ptr::copy_nonoverlapping(p, chunk.as_mut_ptr(), len);
                }
            }
            done += len;
        }
        done
    }
}

impl CharDevice for FbDevice {
    fn open(&self, _flags: poly_vfs::OpenFlags) -> poly_vfs::Result<Option<Arc<dyn CharDevice>>> {
        if self.populate() { Ok(None) } else { Err(Errno::ENOMEM) }
    }

    fn read(&self, offset: u64, buf: &mut [u8], _: bool) -> poly_vfs::Result<usize> {
        Ok(self.transfer(offset, buf, false))
    }

    fn write(&self, offset: u64, buf: &[u8], _: bool) -> poly_vfs::Result<usize> {
        if offset >= self.memory.len() && !buf.is_empty() {
            // fbdev: writing past the end of the frame buffer.
            return Err(Errno::ENOSPC);
        }
        let mut copy = buf.to_vec();
        let n = self.transfer(offset, &mut copy, true);
        if let Some(r) = poly_fbdev::rows_of(&self.mode(), offset, n) {
            self.show(r);
        }
        Ok(n)
    }

    fn seekable(&self) -> bool {
        true
    }

    fn ioctl(&self, request: u32, arg: u64) -> poly_vfs::Result<u64> {
        let need = Device::input_len(request).ok_or(Errno::ENOTTY)?;
        let mut input = [0u8; poly_fbdev::VAR_SIZE];
        if need > 0 {
            crate::user::copy_from_user(&mut input[..need], arg).map_err(|_| Errno::EFAULT)?;
        }
        let reply = self.device.lock().ioctl(request, arg, &input[..need]).map_err(|e| match e {
            poly_fbdev::Error::NoSuchRequest => Errno::ENOTTY,
            poly_fbdev::Error::Invalid => Errno::EINVAL,
        })?;
        if !reply.out.is_empty() {
            crate::user::copy_to_user(arg, &reply.out).map_err(|_| Errno::EFAULT)?;
        }
        if let Action::Show(r) = reply.action {
            self.show(r);
        }
        Ok(reply.value)
    }
}

/// The device, once installed.
pub fn device() -> Option<Arc<FbDevice>> {
    FB.lock().clone()
}

/// What `mmap` of `file` maps, if it is a frame buffer device: its shadow
/// memory (shared mappings only see the frame buffer; a private one gets
/// copies, as of any shared memory object).
pub fn page_source(file: &poly_vfs::OpenFile) -> Option<Arc<dyn PageSource>> {
    let st = file.stat().ok()?;
    if file.kind() != FileType::CharDevice || st.rdev != FB0 {
        return None;
    }
    let fb = device()?;
    Some(fb.memory.clone())
}

/// Register `/dev/fb0` for the display, if there is one, and make its node.
pub fn install(vfs: &Vfs, ctx: &FsContext) {
    let Some(info) = display::info() else {
        kprintln!("fbdev: no display, no /dev/fb0");
        return;
    };
    let mode = Mode { width: info.width, height: info.height, refresh_mhz: 60_000 };
    let id = match info.device {
        display::DeviceKind::VirtioGpu => "poly-virtio-gpu",
        display::DeviceKind::BochsVbe => "poly-bochs-vbe",
    };
    let fb = Arc::new(FbDevice::new(mode, id));
    if vfs.register_char_device(FB0, fb.clone()).is_err() {
        return;
    }
    *FB.lock() = Some(fb);
    match vfs.mknod(ctx, &Cred::root(), At::Cwd, b"/dev/fb0", FileType::CharDevice, 0o660, FB0) {
        Ok(()) | Err(Errno::EEXIST) => {}
        Err(e) => kprintln!("fbdev: /dev/fb0: {}", e.name()),
    }
    kprintln!("fbdev: /dev/fb0 (char 29,0): {}x{} XRGB8888 on {}", mode.width, mode.height, info.device);
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;

kernel_test! {
    fn the_frame_buffer_shows_what_is_written_and_damaged() -> TestResult {
        let Some(fb) = device() else { return Ok(()) };
        if !fb.populate() {
            return Err("no memory for the shadow");
        }
        let mode = fb.mode();
        let mut d = Device::new(mode, "t");
        let var = d.ioctl(poly_fbdev::FBIOGET_VSCREENINFO, 0, &[]).map_err(|_| "FBIOGET_VSCREENINFO")?;
        if var.out.len() != poly_fbdev::VAR_SIZE {
            return Err("the screen information is not 160 bytes");
        }
        // A red pixel at (3, 2) through `write`, then a green one at
        // (mode.width - 1, 1) written into the shadow and shown with damage.
        // The screen is put back afterwards (the screenshot tests compare it
        // with the boot screen).
        let at = |x: u32, y: u32| u64::from(y * mode.line_length() + x * 4);
        let spots = [(3, 2), (mode.width as i32 - 1, 1)];
        let saved = spots.map(|(x, y)| display::with_canvas(|c| c.pixel(x, y)).flatten());
        let result = check_pixels(&fb, mode, at);
        for ((x, y), p) in spots.into_iter().zip(saved) {
            if let Some(p) = p {
                display::draw(|c| {
                    c.set_pixel(x, y, p);
                    Some(poly_gfx::Rect::new(x, y, 1, 1))
                });
            }
        }
        result
    }
}

/// The checks of the test above, on `fb` of `mode`.
fn check_pixels(fb: &FbDevice, mode: Mode, at: impl Fn(u32, u32) -> u64) -> crate::ktest::TestResult {
    if fb.write(at(3, 2), &0x00ff_0000u32.to_le_bytes(), false) != Ok(4) {
        return Err("a write of one pixel");
    }
    let mut green = 0x0000_ff00u32.to_le_bytes();
    fb.transfer(at(mode.width - 1, 1), &mut green, true);
    let r = Rect { x: mode.width - 1, y: 1, width: 1, height: 1 };
    fb.show(r);
    let mut back = [0u8; 4];
    if fb.read(at(3, 2), &mut back, false) != Ok(4) || u32::from_le_bytes(back) != 0x00ff_0000 {
        return Err("a read does not see the write");
    }
    let px = |x: i32, y: i32| display::with_canvas(|c| c.pixel(x, y)).flatten();
    let (red, green) = (px(3, 2), px(mode.width as i32 - 1, 1));
    if red != Some(poly_gfx::Color::rgb(0xff, 0, 0)) || green != Some(poly_gfx::Color::rgb(0, 0xff, 0)) {
        kprintln!("fbdev test: back buffer has {red:?} and {green:?}");
        return Err("the shown pixels are not in the display's back buffer");
    }
    if fb.write(fb.memory.len(), &[0], false) != Err(Errno::ENOSPC) {
        return Err("a write past the end is not ENOSPC");
    }
    Ok(())
}
