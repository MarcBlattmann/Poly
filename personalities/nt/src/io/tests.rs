//! Host tests of NT file I/O through the services ([`crate::dispatch_with`])
//! on a real tmpfs VFS, against the fake kernel of `crate::tests`. The
//! kernel's part of asynchronous I/O — a worker running queued requests and
//! the delivery of completions — is played by [`run_worker`] and [`drain`].

extern crate std;

use super::consts::*;
use super::{IoState, default_volumes};
use crate::kernel::NtKernel;
use crate::services::Service;
use crate::tests::Fake;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_ntobj::status::*;
use poly_ntobj::{NtStatus, OBJ_CASE_INSENSITIVE};
use poly_vfs::{At, Clock, Cred, OpenFlags, Timespec, Tmpfs, Vfs};

const CI: u32 = OBJ_CASE_INSENSITIVE;
const RW: u32 = FILE_GENERIC_READ | FILE_GENERIC_WRITE;
const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
const SYNC: u32 = FILE_SYNCHRONOUS_IO_NONALERT;

#[derive(Debug)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Timespec {
        Timespec::new(1_000_000_000, 0)
    }
}

struct T {
    f: Fake,
    vfs: &'static Vfs,
}

fn setup() -> T {
    let vfs: &'static Vfs = Box::leak(Box::new(Vfs::new(Arc::new(Tmpfs::new(Arc::new(FixedClock))))));
    let mut f = Fake::new();
    let ctx = vfs.root_context().unwrap();
    let vols = default_volumes(vfs, &ctx);
    f.sys.io.attach(&mut f.sys.objects, vfs, &vols).unwrap();
    T { f, vfs }
}

impl T {
    /// `NtCreateFile` of `path` (an NT path): (status, handle, information).
    fn create(
        &mut self,
        path: &str,
        access: u32,
        share: u32,
        disp: u32,
        options: u32,
        attrs: u32,
    ) -> (NtStatus, u64, u64) {
        let out = self.f.alloc(8);
        let iosb = self.f.alloc(16);
        let oa = self.f.attrs(path, CI);
        let s = self.f.call(
            Service::CreateFile,
            &[
                out,
                u64::from(access),
                oa,
                iosb,
                0,
                u64::from(attrs),
                u64::from(share),
                u64::from(disp),
                u64::from(options),
                0,
                0,
            ],
        );
        (s, self.f.u64_at(out), self.f.u64_at(iosb + 8))
    }

    /// Create or open a file for synchronous read/write, sharing everything.
    fn file(&mut self, path: &str) -> u64 {
        let (s, h, _) = self.create(path, RW | DELETE, SHARE_ALL, FILE_OPEN_IF, SYNC | FILE_NON_DIRECTORY_FILE, 0);
        assert_eq!(s, STATUS_SUCCESS, "{path}");
        h
    }

    fn close(&mut self, h: u64) {
        assert_eq!(self.f.call(Service::Close, &[h]), STATUS_SUCCESS);
    }

    fn write(&mut self, h: u64, data: &[u8], offset: Option<i64>) -> (NtStatus, u64) {
        let buf = self.f.put(data);
        let iosb = self.f.alloc(16);
        let off = offset.map_or(0, |o| self.f.put_u64(o as u64));
        let s = self.f.call(Service::WriteFile, &[h, 0, 0, 0, iosb, buf, data.len() as u64, off, 0]);
        (s, self.f.u64_at(iosb + 8))
    }

    fn read(&mut self, h: u64, len: usize, offset: Option<i64>) -> (NtStatus, Vec<u8>) {
        let buf = self.f.alloc(len.max(1));
        let iosb = self.f.alloc(16);
        let off = offset.map_or(0, |o| self.f.put_u64(o as u64));
        let s = self.f.call(Service::ReadFile, &[h, 0, 0, 0, iosb, buf, len as u64, off, 0]);
        let n = self.f.u64_at(iosb + 8) as usize;
        (s, self.f.bytes_at(buf, n.min(len)))
    }

    fn query(&mut self, h: u64, class: u32, len: usize) -> (NtStatus, Vec<u8>) {
        let buf = self.f.alloc(len);
        let iosb = self.f.alloc(16);
        let s = self.f.call(Service::QueryInformationFile, &[h, iosb, buf, len as u64, u64::from(class)]);
        let n = self.f.u64_at(iosb + 8) as usize;
        (s, self.f.bytes_at(buf, n.min(len)))
    }

    fn set(&mut self, h: u64, class: u32, data: &[u8]) -> NtStatus {
        let buf = self.f.put(data);
        let iosb = self.f.alloc(16);
        self.f.call(Service::SetInformationFile, &[h, iosb, buf, data.len() as u64, u64::from(class)])
    }

    fn lock(&mut self, h: u64, offset: u64, length: u64, key: u32, fail_now: bool, exclusive: bool) -> NtStatus {
        let iosb = self.f.alloc(16);
        let o = self.f.put_u64(offset);
        let l = self.f.put_u64(length);
        self.f.call(
            Service::LockFile,
            &[h, 0, 0, 0, iosb, o, l, u64::from(key), u64::from(fail_now), u64::from(exclusive)],
        )
    }

    fn unlock(&mut self, h: u64, offset: u64, length: u64, key: u32) -> NtStatus {
        let iosb = self.f.alloc(16);
        let o = self.f.put_u64(offset);
        let l = self.f.put_u64(length);
        self.f.call(Service::UnlockFile, &[h, iosb, o, l, u64::from(key)])
    }

    /// The VFS's view: whether `path` exists.
    fn exists(&self, path: &[u8]) -> bool {
        let ctx = self.vfs.root_context().unwrap();
        self.vfs.lookup(&ctx, &Cred::root(), At::Cwd, path, false).is_ok()
    }

    fn vfs_create(&self, path: &[u8], data: &[u8]) {
        let ctx = self.vfs.root_context().unwrap();
        let f = self.vfs.open(&ctx, &Cred::root(), At::Cwd, path, OpenFlags::CREAT | OpenFlags::RDWR, 0o644).unwrap();
        f.write(data).unwrap();
    }

    /// Run device `dev`'s queue the way the kernel's worker would.
    fn run_worker(&mut self, dev: u32) -> usize {
        let mut n = 0;
        while let Some(w) = self.f.sys.io.next_work(dev) {
            let r = w.run();
            self.f.sys.io.work_done(w.irp, r);
            n += 1;
        }
        n
    }

    /// Deliver completions the way the kernel does: memory first, then
    /// signals, then APCs.
    fn drain(&mut self) -> Vec<super::Completion> {
        let mut out = Vec::new();
        while let Some(c) = self.f.sys.io.pop_completion() {
            if let Some((addr, data)) = &c.data {
                self.f.mem.write_bytes(*addr, data).unwrap();
            }
            if c.iosb != 0 {
                self.f.mem.write_bytes(c.iosb, &c.iosb_bytes()).unwrap();
            }
            if let Some((t, apc)) = self.f.sys.io.finish(&mut self.f.sys.objects, &c) {
                self.f.apcs.push((t, apc));
            }
            out.push(c);
        }
        out
    }
}

fn wide_str(b: &[u8]) -> String {
    String::from_utf16(&b.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()).unwrap()
}

/// A `FILE_RENAME_INFORMATION` for `name`.
fn rename_info(replace: bool, root: u64, name: &str) -> Vec<u8> {
    let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut b = alloc::vec![0u8; 20];
    b[0] = u8::from(replace);
    b[8..16].copy_from_slice(&root.to_le_bytes());
    b[16..20].copy_from_slice(&(units.len() as u32).to_le_bytes());
    b.extend_from_slice(&units);
    b
}

mod create;
mod dir;
mod info;
mod locks_async;
mod namespace;
mod process;
mod npfs;
