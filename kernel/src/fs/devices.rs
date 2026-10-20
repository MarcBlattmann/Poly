//! The character devices in `/dev`, and the directories every boot has.
//!
//! | Node | Number | Device |
//! |---|---|---|
//! | `/dev/console` | 5, 1 | [`Console`] (writes to the kernel console, reads end of file) until `crate::tty` puts the console terminal here |
//! | `/dev/tty` | 5, 0 | the same, until `crate::tty` puts the caller's controlling terminal here |
//! | `/dev/null` | 1, 3 | `poly_vfs::device::Null` |
//! | `/dev/zero` | 1, 5 | `poly_vfs::device::Zero` |
//! | `/dev/full` | 1, 7 | `poly_vfs::device::Full` |
//! | `/dev/random` | 1, 8 | `crate::random::RandomDevice` (random(4)) |
//! | `/dev/urandom` | 1, 9 | the same generator, larger reads |
//!
//! And the links `/dev/fd` -> `/proc/self/fd`, `/dev/stdin`, `/dev/stdout`,
//! `/dev/stderr` -> `/proc/self/fd/0`, `1`, `2` (proc_pid_fd(5)); procfs
//! itself and the tmpfs on `/dev/shm` are mounted by `fs::proc::mount_all`.
//!
//! The numbers are the ones `console(4)`, `tty(4)`, `null(4)`, `zero(4)`
//! and `full(4)` give, so device nodes an initramfs brings along (a Linux
//! one has `/dev/console`) reach the same devices; nodes it already has are
//! left alone.
//!
//! **Terminals.** `crate::tty` (ADR 0046) registers terminals for (5, 0)
//! and (5, 1) in place of [`Console`] once the devices are installed;
//! [`Console`] (writes to the kernel log, reads end of file) stays what
//! traced test runs get on fds 0-2.

use crate::log;
use crate::sync::SpinLock;
use alloc::sync::Arc;
use poly_vfs::device::{self, CharDevice, Full, Null, Zero};
use poly_vfs::{At, Cred, DeviceId, Errno, FileType, FsContext, Result, Vfs};

/// `/dev/console` and `/dev/tty`: what a program writes goes to the kernel
/// console through the same path as the kernel log (the serial port now,
/// the screen's terminal as well once it exists), and the shell redraws its
/// prompt after it like after any log line. Bytes are UTF-8; a character
/// split across writes is put together, and invalid bytes print as U+FFFD.
pub struct Console {
    carry: SpinLock<Utf8Carry>,
}

impl Console {
    pub const fn new() -> Console {
        Console { carry: SpinLock::new(Utf8Carry { buf: [0; 4], len: 0 }) }
    }
}

impl core::fmt::Debug for Console {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Console").finish_non_exhaustive()
    }
}

impl CharDevice for Console {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        // End of file: see the module documentation.
        Ok(0)
    }

    fn write(&self, _: u64, buf: &[u8], _: bool) -> Result<usize> {
        self.carry.lock().feed(buf, &mut |s| log::_print(format_args!("{s}")));
        Ok(buf.len())
    }
}

/// The start of a UTF-8 character that the last write ended in the middle
/// of (at most 3 bytes).
#[derive(Debug, Default)]
pub struct Utf8Carry {
    buf: [u8; 4],
    len: usize,
}

impl Utf8Carry {
    /// Turn `bytes`, which follow what was carried, into text for `out`:
    /// every complete character, U+FFFD for each invalid sequence, and an
    /// incomplete character at the end kept for the next call.
    pub fn feed(&mut self, mut bytes: &[u8], out: &mut dyn FnMut(&str)) {
        // Finish the character the previous write left open.
        while self.len > 0 {
            let Some(&b) = bytes.first() else { return };
            self.buf[self.len] = b;
            match core::str::from_utf8(&self.buf[..=self.len]) {
                Ok(s) => {
                    out(s);
                    self.len = 0;
                    bytes = &bytes[1..];
                }
                Err(e) if e.error_len().is_none() => {
                    self.len += 1;
                    bytes = &bytes[1..];
                }
                Err(_) => {
                    // `b` does not continue it: the carried bytes are invalid,
                    // and `b` is looked at again below.
                    out("\u{fffd}");
                    self.len = 0;
                }
            }
        }
        loop {
            match core::str::from_utf8(bytes) {
                Ok(s) => {
                    if !s.is_empty() {
                        out(s);
                    }
                    return;
                }
                Err(e) => {
                    let (good, rest) = bytes.split_at(e.valid_up_to());
                    if let Ok(s) = core::str::from_utf8(good)
                        && !s.is_empty()
                    {
                        out(s);
                    }
                    match e.error_len() {
                        None => {
                            // An incomplete character at the end: at most 3 bytes.
                            self.buf[..rest.len()].copy_from_slice(rest);
                            self.len = rest.len();
                            return;
                        }
                        Some(n) => {
                            out("\u{fffd}");
                            bytes = &rest[n..];
                        }
                    }
                }
            }
        }
    }
}

/// A node to make in `/dev`, with the permissions Linux systems give it.
struct Node {
    name: &'static [u8],
    id: DeviceId,
    perm: u16,
}

const NODES: [Node; 7] = [
    Node { name: b"/dev/console", id: device::CONSOLE, perm: 0o600 },
    Node { name: b"/dev/tty", id: device::TTY, perm: 0o666 },
    Node { name: b"/dev/null", id: device::NULL, perm: 0o666 },
    Node { name: b"/dev/zero", id: device::ZERO, perm: 0o666 },
    Node { name: b"/dev/full", id: device::FULL, perm: 0o666 },
    Node { name: b"/dev/random", id: device::RANDOM, perm: 0o666 },
    Node { name: b"/dev/urandom", id: device::URANDOM, perm: 0o666 },
];

/// Symbolic links every `/dev` gets (proc_pid_fd(5): "/dev/fd ... link to
/// /proc/self/fd"; "/dev/stdin, /dev/stdout, and /dev/stderr, which
/// respectively link to the files 0, 1, and 2 in /proc/self/fd").
const LINKS: [(&[u8], &[u8]); 4] = [
    (b"/dev/fd", b"/proc/self/fd"),
    (b"/dev/stdin", b"/proc/self/fd/0"),
    (b"/dev/stdout", b"/proc/self/fd/1"),
    (b"/dev/stderr", b"/proc/self/fd/2"),
];

/// Directories every root gets: `/dev` for the nodes, `/tmp` (sticky, as
/// `hier(7)` has it) and `/proc` (where procfs will be mounted).
const DIRS: [(&[u8], u16); 3] = [(b"/dev", 0o755), (b"/tmp", 0o1777), (b"/proc", 0o555)];

/// Register the devices and make the directories and nodes that are not
/// there yet.
pub fn install(vfs: &Vfs, ctx: &FsContext) {
    let console: Arc<dyn CharDevice> = Arc::new(Console::new());
    let devices: [(DeviceId, Arc<dyn CharDevice>); 7] = [
        (device::CONSOLE, console.clone()),
        (device::TTY, console),
        (device::NULL, Arc::new(Null)),
        (device::ZERO, Arc::new(Zero)),
        (device::FULL, Arc::new(Full)),
        (device::RANDOM, Arc::new(crate::random::RandomDevice::RANDOM)),
        (device::URANDOM, Arc::new(crate::random::RandomDevice::URANDOM)),
    ];
    for (id, dev) in devices {
        vfs.register_char_device(id, dev).expect("each device number is registered once");
    }
    // Modes exactly as given, not masked by a umask.
    let mut ctx = ctx.clone();
    ctx.set_umask(0);
    let root = Cred::root();
    let mut made = 0;
    for (path, perm) in DIRS {
        made += created(vfs.mkdir(&ctx, &root, At::Cwd, path, perm), path);
    }
    for n in &NODES {
        made += created(vfs.mknod(&ctx, &root, At::Cwd, n.name, FileType::CharDevice, n.perm, n.id), n.name);
    }
    for (path, target) in LINKS {
        made += created(vfs.symlink(&ctx, &root, target, At::Cwd, path), path);
    }
    crate::kprintln!(
        "fs: devices console (5,1), tty (5,0), null (1,3), zero (1,5), full (1,7), random (1,8), urandom (1,9); \
         {made} of {} directories, nodes and links made",
        DIRS.len() + NODES.len() + LINKS.len()
    );
}

/// 1 if the call made `path`, 0 if it was there already; anything else is
/// logged.
fn created(r: Result<()>, path: &[u8]) -> usize {
    match r {
        Ok(()) => 1,
        Err(Errno::EEXIST) => 0,
        Err(e) => {
            crate::kprintln!("fs: {}: {}", alloc::string::String::from_utf8_lossy(path), e.name());
            0
        }
    }
}
