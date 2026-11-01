//! The screen through the Linux framebuffer device (ADR 0107): `/dev/fb0`
//! opened, described (`FBIOGET_VSCREENINFO`, `FBIOGET_FSCREENINFO`),
//! mapped shared, and drawn into directly; `present` shows each damaged
//! rectangle with Poly's `FBIO_POLY_DAMAGE` (one copy into the display
//! and a virtio-gpu transfer and flush of just that rectangle). On a Linux
//! kernel, which does not know that request, it falls back to
//! `FBIOPAN_DISPLAY`, which every fbdev driver accepts.

use poly_fbdev::{FBIO_POLY_DAMAGE, FBIOGET_FSCREENINFO, FBIOGET_VSCREENINFO, FBIOPAN_DISPLAY, FIX_SIZE, VAR_SIZE};
use poly_fbdev::{Rect as FbRect, VarScreenInfo};
use poly_wayland::compose::{Framebuffer, Rect};
use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;

/// `/dev/fb0`, mapped.
#[derive(Debug)]
pub struct FbFramebuffer {
    fd: RawFd,
    ptr: *mut u32,
    len: usize,
    width: u32,
    height: u32,
    /// Pixels from one row to the next.
    stride: usize,
    var: VarScreenInfo,
    /// Poly's damage request works (else the whole frame is shown).
    damage_ok: bool,
    /// Rectangles shown, for the log.
    pub presented: u64,
}

fn ioctl(fd: RawFd, request: u32, arg: *mut u8) -> io::Result<()> {
    // SAFETY: `arg` points to a buffer as large as `request` reads or
    // writes (each caller passes one of the sizes poly-fbdev gives).
    let r = unsafe { libc::ioctl(fd, request as libc::c_ulong, arg) };
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

impl FbFramebuffer {
    pub fn open(path: &str) -> io::Result<FbFramebuffer> {
        let c = CString::new(path).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: a NUL-terminated path.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fail = |e: io::Error| {
            crate::unix::close(fd);
            e
        };
        let mut var = [0u8; VAR_SIZE];
        ioctl(fd, FBIOGET_VSCREENINFO, var.as_mut_ptr()).map_err(fail)?;
        let mut fix = [0u8; FIX_SIZE];
        ioctl(fd, FBIOGET_FSCREENINFO, fix.as_mut_ptr()).map_err(fail)?;
        let var = VarScreenInfo::from_bytes(&var);
        let line = u32::from_le_bytes([fix[48], fix[49], fix[50], fix[51]]) as usize;
        let smem_len = u32::from_le_bytes([fix[24], fix[25], fix[26], fix[27]]) as usize;
        if var.bits_per_pixel != 32 || var.red.offset != 16 || var.blue.offset != 0 || !line.is_multiple_of(4) {
            crate::unix::close(fd);
            return Err(io::Error::other(format!(
                "{path}: not a 32-bit XRGB8888 frame buffer ({} bits per pixel)",
                var.bits_per_pixel
            )));
        }
        let len = smem_len.max(line * var.yres as usize);
        // SAFETY: a new shared mapping of the device chosen by the kernel;
        // nothing else refers to that range.
        let ptr = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
        };
        if ptr == libc::MAP_FAILED {
            return Err(fail(io::Error::last_os_error()));
        }
        Ok(FbFramebuffer {
            fd,
            ptr: ptr.cast(),
            len,
            width: var.xres,
            height: var.yres,
            stride: line / 4,
            var,
            damage_ok: true,
            presented: 0,
        })
    }

    /// Show the whole frame.
    pub fn present_all(&mut self) {
        let mut var = self.var.to_bytes();
        let _ = ioctl(self.fd, FBIOPAN_DISPLAY, var.as_mut_ptr());
    }
}

impl Framebuffer for FbFramebuffer {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn row_mut(&mut self, y: u32) -> &mut [u32] {
        assert!(y < self.height);
        // SAFETY: row `y` is inside the mapping (`len` covers every line),
        // which lives as long as `self`; the `&mut self` borrow keeps other
        // references to it out. Only the kernel reads it as well, when it
        // copies a shown rectangle.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.add(y as usize * self.stride), self.width as usize) }
    }

    fn present(&mut self, damage: &[Rect]) {
        if self.damage_ok {
            for r in damage {
                let rect = FbRect {
                    x: r.x.max(0) as u32,
                    y: r.y.max(0) as u32,
                    width: r.width as u32,
                    height: r.height as u32,
                };
                let mut arg = rect.to_damage();
                if ioctl(self.fd, FBIO_POLY_DAMAGE, arg.as_mut_ptr()).is_err() {
                    // Not Poly: show everything from now on.
                    self.damage_ok = false;
                    break;
                }
                self.presented += 1;
            }
        }
        if !self.damage_ok {
            self.present_all();
            self.presented += 1;
        }
    }
}

impl Drop for FbFramebuffer {
    fn drop(&mut self) {
        // SAFETY: exactly the range mmap returned; no reference into it
        // outlives `self`.
        unsafe { libc::munmap(self.ptr.cast(), self.len) };
        crate::unix::close(self.fd);
    }
}
