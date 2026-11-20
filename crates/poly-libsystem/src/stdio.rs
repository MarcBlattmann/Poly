//! stdio's buffering (C11 7.21.3, 7.21.5.6) and the `FILE` layout of the
//! macOS SDK (`<_stdio.h>`, `struct __sFILE`): programs compiled against
//! the SDK reach into a `FILE` through its macros (`putc_unlocked`,
//! `getc_unlocked`, `fileno_unlocked`, `feof_unlocked`, `ferror_unlocked`).
//!
//! Poly keeps the macros' fields so that they always call the library:
//! `_w` and `_r` stay 0, so `__sputc` calls `__swbuf` and `__sgetc` calls
//! `__srget`; `_flags` carries `__SEOF` and `__SERR` as the macros test
//! them, `_file` the descriptor. The buffer is [`Stream`]'s, behind
//! `_cookie`.

/// `<_stdio.h>` flag bits of `_flags`.
pub const SLBF: i16 = 0x0001;
pub const SNBF: i16 = 0x0002;
pub const SRD: i16 = 0x0004;
pub const SWR: i16 = 0x0008;
pub const SRW: i16 = 0x0010;
pub const SEOF: i16 = 0x0020;
pub const SERR: i16 = 0x0040;
pub const SAPP: i16 = 0x0100;

/// `setvbuf` modes (`<stdio.h>`).
pub const IOFBF: i32 = 0;
pub const IOLBF: i32 = 1;
pub const IONBF: i32 = 2;
/// `BUFSIZ` (`<_stdio.h>`).
pub const BUFSIZ: usize = 1024;
/// `EOF`.
pub const EOF: i32 = -1;

/// `struct __sbuf`.
#[repr(C)]
#[derive(Debug)]
pub struct SBuf {
    pub base: *mut u8,
    pub size: i32,
}

/// `struct __sFILE` (`FILE`), 152 bytes.
#[repr(C)]
#[derive(Debug)]
pub struct File {
    pub p: *mut u8,
    pub r: i32,
    pub w: i32,
    pub flags: i16,
    pub file: i16,
    pub bf: SBuf,
    pub lbfsize: i32,
    pub cookie: *mut core::ffi::c_void,
    pub close: usize,
    pub read: usize,
    pub seek: usize,
    pub write: usize,
    pub ub: SBuf,
    pub extra: *mut core::ffi::c_void,
    pub ur: i32,
    pub ubuf: [u8; 3],
    pub nbuf: [u8; 1],
    pub lb: SBuf,
    pub blksize: i32,
    pub offset: i64,
}

impl File {
    /// A stream on descriptor `fd` whose state is at `cookie`.
    pub const fn new(fd: i16, flags: i16, cookie: *mut core::ffi::c_void) -> Self {
        let none = SBuf { base: core::ptr::null_mut(), size: 0 };
        File {
            p: core::ptr::null_mut(),
            r: 0,
            w: 0,
            flags,
            file: fd,
            bf: SBuf { base: core::ptr::null_mut(), size: 0 },
            lbfsize: 0,
            cookie,
            close: 0,
            read: 0,
            seek: 0,
            write: 0,
            ub: SBuf { base: core::ptr::null_mut(), size: 0 },
            extra: core::ptr::null_mut(),
            ur: 0,
            ubuf: [0; 3],
            nbuf: [0; 1],
            lb: none,
            blksize: 0,
            offset: 0,
        }
    }
}

/// How a stream buffers its output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Every write goes out at once (`stderr`).
    Unbuffered,
    /// Out at every newline, when the buffer fills, and on flush.
    Line,
    /// Out when the buffer fills, and on flush.
    Full,
}

/// Where output goes: `write(2)`.
pub trait Output {
    /// The bytes written, or the errno.
    fn write(&mut self, fd: i32, bytes: &[u8]) -> Result<usize, i32>;
}

/// Where input comes from and how a descriptor moves: `read(2)` and
/// `lseek(2)`.
pub trait Input {
    /// The bytes read (0 at the end), or the errno.
    fn read(&mut self, fd: i32, buf: &mut [u8]) -> Result<usize, i32>;
    /// The new offset, or the errno.
    fn seek(&mut self, fd: i32, offset: i64, whence: i32) -> Result<i64, i32>;
}

/// Bytes `ungetc` can push back before any is read again (C guarantees
/// one).
pub const UNGET: usize = 4;

/// A stream: its output buffer and its input buffer (one of them holds
/// bytes at a time: switching direction flushes or drops the other).
#[derive(Debug)]
pub struct Stream {
    pub fd: i32,
    pub mode: Mode,
    buf: [u8; BUFSIZ],
    len: usize,
    rbuf: [u8; BUFSIZ],
    rpos: usize,
    rlen: usize,
    /// Pushed-back bytes, the next to read last.
    unget: [u8; UNGET],
    nunget: usize,
}

impl Stream {
    pub const fn new(fd: i32, mode: Mode) -> Self {
        Stream { fd, mode, buf: [0; BUFSIZ], len: 0, rbuf: [0; BUFSIZ], rpos: 0, rlen: 0, unget: [0; UNGET], nunget: 0 }
    }

    /// Bytes read from the descriptor (or pushed back) that the program
    /// has not taken yet.
    pub fn unread(&self) -> usize {
        self.rlen - self.rpos + self.nunget
    }

    /// Give back the unread input: the descriptor moves back over what was
    /// read ahead (before a write, a seek or a close of a file one shares).
    pub fn drop_input(&mut self, inp: &mut dyn Input) -> Result<(), i32> {
        let ahead = (self.rlen - self.rpos) as i64;
        self.rpos = 0;
        self.rlen = 0;
        self.nunget = 0;
        if ahead > 0 {
            // A pipe or terminal cannot move back; what was read ahead is lost.
            let _ = inp.seek(self.fd, -ahead, 1);
        }
        Ok(())
    }

    /// The next byte, `None` at the end of the input.
    pub fn getc(&mut self, inp: &mut dyn Input) -> Result<Option<u8>, i32> {
        if self.nunget > 0 {
            self.nunget -= 1;
            return Ok(Some(self.unget[self.nunget]));
        }
        if self.rpos == self.rlen {
            let n = inp.read(self.fd, &mut self.rbuf)?;
            self.rpos = 0;
            self.rlen = n;
            if n == 0 {
                return Ok(None);
            }
        }
        self.rpos += 1;
        Ok(Some(self.rbuf[self.rpos - 1]))
    }

    /// Fill `out` from the stream: the bytes read (fewer only at the end
    /// of the input). Large reads go to the descriptor directly.
    pub fn read(&mut self, out: &mut [u8], inp: &mut dyn Input) -> Result<usize, i32> {
        let mut n = 0;
        while n < out.len() && self.nunget > 0 {
            self.nunget -= 1;
            out[n] = self.unget[self.nunget];
            n += 1;
        }
        let have = (self.rlen - self.rpos).min(out.len() - n);
        out[n..n + have].copy_from_slice(&self.rbuf[self.rpos..self.rpos + have]);
        self.rpos += have;
        n += have;
        while n < out.len() {
            let want = out.len() - n;
            let got = if want >= BUFSIZ {
                inp.read(self.fd, &mut out[n..])?
            } else {
                let got = inp.read(self.fd, &mut self.rbuf)?;
                self.rpos = 0;
                self.rlen = got;
                let take = got.min(want);
                out[n..n + take].copy_from_slice(&self.rbuf[..take]);
                self.rpos = take;
                take
            };
            if got == 0 {
                break;
            }
            n += got;
        }
        Ok(n)
    }

    /// `ungetc`: push `c` back; false if there is no room.
    pub fn ungetc(&mut self, c: u8) -> bool {
        if self.nunget == UNGET {
            return false;
        }
        self.unget[self.nunget] = c;
        self.nunget += 1;
        true
    }

    /// `ftell`: the descriptor's offset less what was read ahead, plus
    /// what waits to be written.
    pub fn tell(&mut self, inp: &mut dyn Input) -> Result<i64, i32> {
        let at = inp.seek(self.fd, 0, 1)?;
        Ok(at - self.unread() as i64 + self.len as i64)
    }

    /// `fseek`: write out what is buffered, drop the input, move.
    pub fn seek(&mut self, offset: i64, whence: i32, out: &mut dyn Output, inp: &mut dyn Input) -> Result<i64, i32> {
        self.flush(out)?;
        // A move from the current position counts from what the program saw.
        let offset = if whence == 1 { offset - self.unread() as i64 } else { offset };
        self.rpos = 0;
        self.rlen = 0;
        self.nunget = 0;
        inp.seek(self.fd, offset, whence)
    }

    /// Bytes waiting in the buffer.
    pub fn pending(&self) -> usize {
        self.len
    }

    /// Write all of `bytes` to the descriptor (short writes are retried);
    /// the errno of a failed write.
    fn put_all(fd: i32, mut bytes: &[u8], out: &mut dyn Output) -> Result<(), i32> {
        while !bytes.is_empty() {
            let n = out.write(fd, bytes)?;
            if n == 0 {
                return Err(5); // EIO: the descriptor takes nothing.
            }
            bytes = &bytes[n.min(bytes.len())..];
        }
        Ok(())
    }

    /// Send what is buffered. On failure the buffer is dropped and the
    /// errno returned (the caller sets the error indicator).
    pub fn flush(&mut self, out: &mut dyn Output) -> Result<(), i32> {
        let n = core::mem::take(&mut self.len);
        Self::put_all(self.fd, &self.buf[..n], out)
    }

    /// `fwrite` of `bytes` (after [`Stream::drop_input`] if the stream was
    /// reading).
    pub fn write(&mut self, bytes: &[u8], out: &mut dyn Output) -> Result<(), i32> {
        if self.mode == Mode::Unbuffered {
            self.flush(out)?;
            return Self::put_all(self.fd, bytes, out);
        }
        if bytes.len() > BUFSIZ - self.len {
            self.flush(out)?;
            if bytes.len() >= BUFSIZ {
                return Self::put_all(self.fd, bytes, out);
            }
        }
        self.buf[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        if self.len == BUFSIZ || (self.mode == Mode::Line && bytes.contains(&b'\n')) {
            self.flush(out)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[derive(Default)]
    struct Rec {
        calls: Vec<(i32, Vec<u8>)>,
        max: Option<usize>,
        fail: Option<i32>,
    }

    impl Output for Rec {
        fn write(&mut self, fd: i32, bytes: &[u8]) -> Result<usize, i32> {
            if let Some(e) = self.fail {
                return Err(e);
            }
            let n = self.max.map_or(bytes.len(), |m| m.min(bytes.len()));
            self.calls.push((fd, bytes[..n].to_vec()));
            Ok(n)
        }
    }

    #[test]
    fn the_sdk_layout() {
        assert_eq!(core::mem::size_of::<File>(), 152);
        assert_eq!(core::mem::offset_of!(File, flags), 16);
        assert_eq!(core::mem::offset_of!(File, file), 18);
        assert_eq!(core::mem::offset_of!(File, bf), 24);
        assert_eq!(core::mem::offset_of!(File, lbfsize), 40);
        assert_eq!(core::mem::offset_of!(File, cookie), 48);
        assert_eq!(core::mem::offset_of!(File, write), 80);
        assert_eq!(core::mem::offset_of!(File, ub), 88);
        assert_eq!(core::mem::offset_of!(File, extra), 104);
        assert_eq!(core::mem::offset_of!(File, ubuf), 116);
        assert_eq!(core::mem::offset_of!(File, lb), 120);
        assert_eq!(core::mem::offset_of!(File, offset), 144);
    }

    #[test]
    fn line_buffering() {
        let mut s = Stream::new(1, Mode::Line);
        let mut r = Rec::default();
        s.write(b"hello, ", &mut r).unwrap();
        assert!(r.calls.is_empty());
        s.write(b"world\nmore", &mut r).unwrap();
        assert_eq!(r.calls, [(1, b"hello, world\nmore".to_vec())]);
        s.write(b"tail", &mut r).unwrap();
        assert_eq!(s.pending(), 4);
        s.flush(&mut r).unwrap();
        assert_eq!(r.calls[1], (1, b"tail".to_vec()));
        assert_eq!(s.pending(), 0);
    }

    #[test]
    fn unbuffered_and_full() {
        let mut r = Rec::default();
        let mut e = Stream::new(2, Mode::Unbuffered);
        e.write(b"a", &mut r).unwrap();
        e.write(b"b", &mut r).unwrap();
        assert_eq!(r.calls, [(2, b"a".to_vec()), (2, b"b".to_vec())]);
        let mut f = Stream::new(3, Mode::Full);
        f.write(b"x\n", &mut r).unwrap();
        assert_eq!(r.calls.len(), 2);
        f.write(&[b'y'; BUFSIZ - 2], &mut r).unwrap();
        assert_eq!(r.calls.len(), 3);
        assert_eq!(r.calls[2].1.len(), BUFSIZ);
    }

    #[test]
    fn large_writes_bypass_the_buffer_and_short_writes_are_retried() {
        let mut r = Rec { max: Some(1000), ..Rec::default() };
        let mut s = Stream::new(1, Mode::Line);
        s.write(b"head", &mut r).unwrap();
        s.write(&[b'z'; 3000], &mut r).unwrap();
        let total: usize = r.calls.iter().map(|c| c.1.len()).sum();
        assert_eq!(total, 3004);
        assert_eq!(r.calls[0].1, b"head");
    }

    /// A file in memory.
    struct Mem {
        data: Vec<u8>,
        at: usize,
        reads: usize,
    }

    impl Input for Mem {
        fn read(&mut self, _fd: i32, buf: &mut [u8]) -> Result<usize, i32> {
            self.reads += 1;
            let n = buf.len().min(self.data.len() - self.at);
            buf[..n].copy_from_slice(&self.data[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
        fn seek(&mut self, _fd: i32, offset: i64, whence: i32) -> Result<i64, i32> {
            let base = match whence {
                0 => 0,
                1 => self.at as i64,
                _ => self.data.len() as i64,
            };
            let to = base + offset;
            if to < 0 {
                return Err(22);
            }
            self.at = to as usize;
            Ok(to)
        }
    }

    #[test]
    fn reading_ungetc_tell_and_seek() {
        let data: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
        let mut m = Mem { data: data.clone(), at: 0, reads: 0 };
        let mut s = Stream::new(3, Mode::Full);
        assert_eq!(s.getc(&mut m), Ok(Some(0)));
        assert_eq!(m.reads, 1);
        assert_eq!(s.tell(&mut m), Ok(1));
        assert!(s.ungetc(b'z'));
        assert_eq!(s.tell(&mut m), Ok(0));
        let mut out = [0u8; 10];
        assert_eq!(s.read(&mut out, &mut m), Ok(10));
        assert_eq!(out[0], b'z');
        assert_eq!(&out[1..], &data[1..10]);
        // A large read: the rest of the buffer, then the descriptor.
        let mut big = alloc::vec![0u8; 2500];
        assert_eq!(s.read(&mut big, &mut m), Ok(2500));
        assert_eq!(&big[..], &data[10..2510]);
        assert_eq!(s.tell(&mut m), Ok(2510));
        // Seek back from the current position, then to the end.
        assert_eq!(s.seek(-10, 1, &mut Rec::default(), &mut m), Ok(2500));
        assert_eq!(s.getc(&mut m), Ok(Some(data[2500])));
        assert_eq!(s.seek(0, 2, &mut Rec::default(), &mut m), Ok(3000));
        assert_eq!(s.getc(&mut m), Ok(None));
        // Dropping read-ahead moves the descriptor back.
        let mut m = Mem { data: data.clone(), at: 0, reads: 0 };
        let mut s = Stream::new(3, Mode::Full);
        s.getc(&mut m).unwrap();
        s.drop_input(&mut m).unwrap();
        assert_eq!(m.at, 1);
        for _ in 0..UNGET {
            assert!(s.ungetc(1));
        }
        assert!(!s.ungetc(1));
    }

    #[test]
    fn errors_drop_the_buffer() {
        let mut r = Rec { fail: Some(9), ..Rec::default() };
        let mut s = Stream::new(7, Mode::Line);
        assert_eq!(s.write(b"x\n", &mut r), Err(9));
        assert_eq!(s.pending(), 0);
    }
}
