//! The x86-64 Linux system call table: every call's number and name, and
//! the handler of each implemented one.
//!
//! Names and numbers live in the generated `numbers.rs`, kept identical to
//! the coverage map (`coverage/linux/syscall/x86_64.json`) by a test.
//! Handlers are registered in [`IMPLEMENTED`]; the dispatcher looks them up
//! by number in a table built at compile time.

use crate::args::Args;
use crate::errno::SysResult;
use crate::kernel::LinuxKernel;
use crate::sys::{fd, futex, io, limits, mem, path, process, signal, system};

pub use crate::numbers::{SYSCALLS, nr};

/// A system call handler: arguments in, `rax` value or errno out.
pub type Handler = fn(&mut dyn LinuxKernel, &Args) -> SysResult;

/// One more than the highest system call number.
pub const TABLE_LEN: usize = SYSCALLS[SYSCALLS.len() - 1].0 as usize + 1;

/// Every implemented system call. Adding a call means adding a line here
/// (and tests); the coverage map is updated once the kernel runs it.
pub const IMPLEMENTED: &[(u64, Handler)] = &[
    (nr::READ, io::read),
    (nr::WRITE, io::write),
    (nr::CLOSE, io::close),
    (nr::FSTAT, io::fstat),
    (nr::MMAP, mem::mmap),
    (nr::MPROTECT, mem::mprotect),
    (nr::MUNMAP, mem::munmap),
    (nr::BRK, mem::brk),
    (nr::RT_SIGACTION, signal::rt_sigaction),
    (nr::RT_SIGPROCMASK, signal::rt_sigprocmask),
    (nr::RT_SIGRETURN, signal::rt_sigreturn),
    (nr::IOCTL, io::ioctl),
    (nr::READV, io::readv),
    (nr::WRITEV, io::writev),
    (nr::GETPID, process::getpid),
    (nr::EXIT, process::exit),
    (nr::KILL, signal::kill),
    (nr::UNAME, system::uname),
    (nr::GETUID, process::getuid),
    (nr::GETGID, process::getgid),
    (nr::GETEUID, process::geteuid),
    (nr::GETEGID, process::getegid),
    (nr::GETPPID, process::getppid),
    (nr::RT_SIGPENDING, signal::rt_sigpending),
    (nr::RT_SIGTIMEDWAIT, signal::rt_sigtimedwait),
    (nr::RT_SIGQUEUEINFO, signal::rt_sigqueueinfo),
    (nr::RT_SIGSUSPEND, signal::rt_sigsuspend),
    (nr::SIGALTSTACK, signal::sigaltstack),
    (nr::ARCH_PRCTL, process::arch_prctl),
    (nr::GETTID, process::gettid),
    (nr::TKILL, signal::tkill),
    (nr::FUTEX, futex::futex),
    (nr::SET_TID_ADDRESS, process::set_tid_address),
    (nr::RESTART_SYSCALL, signal::restart_syscall),
    (nr::CLOCK_GETTIME, system::clock_gettime),
    (nr::EXIT_GROUP, process::exit_group),
    (nr::TGKILL, signal::tgkill),
    (nr::SET_ROBUST_LIST, futex::set_robust_list),
    (nr::GET_ROBUST_LIST, futex::get_robust_list),
    (nr::RT_TGSIGQUEUEINFO, signal::rt_tgsigqueueinfo),
    (nr::GETRANDOM, system::getrandom),
];

/// The file system calls and resource limits (ADR 0043), registered like
/// [`IMPLEMENTED`].
pub const FILE_SYSTEM: &[(u64, Handler)] = &[
    (nr::STAT, path::stat),
    (nr::LSTAT, path::lstat),
    (nr::OPEN, path::open),
    (nr::LSEEK, fd::lseek),
    (nr::PREAD64, fd::pread64),
    (nr::PWRITE64, fd::pwrite64),
    (nr::ACCESS, path::access),
    (nr::DUP, fd::dup),
    (nr::DUP2, fd::dup2),
    (nr::FCNTL, fd::fcntl),
    (nr::FSYNC, fd::fsync),
    (nr::FDATASYNC, fd::fdatasync),
    (nr::TRUNCATE, path::truncate),
    (nr::FTRUNCATE, fd::ftruncate),
    (nr::GETDENTS, fd::getdents),
    (nr::GETCWD, path::getcwd),
    (nr::CHDIR, path::chdir),
    (nr::FCHDIR, path::fchdir),
    (nr::RENAME, path::rename),
    (nr::MKDIR, path::mkdir),
    (nr::RMDIR, path::rmdir),
    (nr::CREAT, path::creat),
    (nr::LINK, path::link),
    (nr::UNLINK, path::unlink),
    (nr::SYMLINK, path::symlink),
    (nr::READLINK, path::readlink),
    (nr::CHMOD, path::chmod),
    (nr::FCHMOD, path::fchmod),
    (nr::UMASK, path::umask),
    (nr::GETRLIMIT, limits::getrlimit),
    (nr::SYSINFO, limits::sysinfo),
    (nr::STATFS, path::statfs),
    (nr::FSTATFS, fd::fstatfs),
    (nr::SETRLIMIT, limits::setrlimit),
    (nr::SYNC, fd::sync),
    (nr::GETDENTS64, fd::getdents64),
    (nr::OPENAT, path::openat),
    (nr::MKDIRAT, path::mkdirat),
    (nr::NEWFSTATAT, path::newfstatat),
    (nr::UNLINKAT, path::unlinkat),
    (nr::RENAMEAT, path::renameat),
    (nr::LINKAT, path::linkat),
    (nr::SYMLINKAT, path::symlinkat),
    (nr::READLINKAT, path::readlinkat),
    (nr::FCHMODAT, path::fchmodat),
    (nr::FACCESSAT, path::faccessat),
    (nr::DUP3, fd::dup3),
    (nr::PRLIMIT64, limits::prlimit64),
    (nr::SYNCFS, fd::syncfs),
    (nr::RENAMEAT2, path::renameat2),
    (nr::STATX, path::statx),
    (nr::FACCESSAT2, path::faccessat2),
];

static HANDLERS: [Option<Handler>; TABLE_LEN] = build();

const fn build() -> [Option<Handler>; TABLE_LEN] {
    let mut table: [Option<Handler>; TABLE_LEN] = [None; TABLE_LEN];
    let lists = [IMPLEMENTED, FILE_SYSTEM];
    let mut l = 0;
    while l < lists.len() {
        let list = lists[l];
        let mut i = 0;
        while i < list.len() {
            let (number, handler) = list[i];
            assert!(table[number as usize].is_none(), "system call registered twice");
            table[number as usize] = Some(handler);
            i += 1;
        }
        l += 1;
    }
    table
}

/// The handler for system call `number`, if it is implemented.
pub fn handler(number: u64) -> Option<Handler> {
    HANDLERS.get(usize::try_from(number).ok()?).copied().flatten()
}

/// The name of system call `number` (`None` for numbers no call has).
pub fn name(number: u64) -> Option<&'static str> {
    SYSCALLS.binary_search_by_key(&number, |&(n, _)| n).ok().map(|i| SYSCALLS[i].1)
}

/// The number of the system call called `name`.
pub fn number(name: &str) -> Option<u64> {
    SYSCALLS.iter().find(|&&(_, n)| n == name).map(|&(nr, _)| nr)
}

/// The implemented system calls as `(number, name)`, in number order.
pub fn implemented() -> impl Iterator<Item = (u64, &'static str)> {
    SYSCALLS.iter().copied().filter(|&(nr, _)| handler(nr).is_some())
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::string::String;
    use std::vec::Vec;

    const COVERAGE_MAP: &str = include_str!("../../../coverage/linux/syscall/x86_64.json");
    const GENERATED: &str = include_str!("numbers.rs");

    /// `(number, symbol)` for each entry of the coverage map. The file has
    /// one `{"symbol":"...","number":N,...}` object per line; this reads
    /// just those two fields so the crate needs no JSON dependency.
    fn coverage_entries() -> Vec<(u64, String)> {
        let field = |line: &str, key: &str| -> Option<String> {
            let start = line.find(key)? + key.len();
            let rest = &line[start..];
            let end = rest.find([',', '}'])?;
            Some(rest[..end].trim_matches('"').into())
        };
        let mut entries: Vec<(u64, String)> = COVERAGE_MAP
            .lines()
            .filter(|l| l.contains("\"symbol\""))
            .map(|l| {
                let symbol = field(l, "\"symbol\":").expect("symbol");
                let number = field(l, "\"number\":").expect("number").parse().expect("numeric number");
                (number, symbol)
            })
            .collect();
        entries.sort();
        entries
    }

    /// The text `numbers.rs` must have for the given entries.
    fn render(entries: &[(u64, String)]) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        s.push_str("// GENERATED from coverage/linux/syscall/x86_64.json by the test\n");
        s.push_str("// `table::tests::numbers_match_coverage_map`. Do not edit by hand: run\n");
        s.push_str("// `POLY_BLESS=1 cargo test -p poly-personality-linux numbers_match_coverage_map`.\n");
        s.push_str("// System call names and numbers are ABI facts (see CLEAN_ROOM.md).\n\n");
        s.push_str("/// Every x86-64 Linux system call as `(number, name)`, sorted by number.\n");
        writeln!(s, "pub const SYSCALLS: [(u64, &str); {}] = [", entries.len()).unwrap();
        for (n, sym) in entries {
            writeln!(s, "    ({n}, \"{sym}\"),").unwrap();
        }
        s.push_str("];\n\n/// System call numbers, named after the call in upper case.\npub mod nr {\n");
        for (n, sym) in entries {
            writeln!(s, "    pub const {}: u64 = {n};", sym.to_uppercase()).unwrap();
        }
        s.push_str("}\n");
        s
    }

    #[test]
    fn numbers_match_coverage_map() {
        let entries = coverage_entries();
        assert_eq!(entries.len(), 370, "the coverage map changed size; check the import");
        let expected = render(&entries);
        if expected != GENERATED {
            if std::env::var_os("POLY_BLESS").is_some() {
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/numbers.rs");
                std::fs::write(&path, &expected).expect("write numbers.rs");
                std::panic!("regenerated {}; rebuild and run the tests again", path.display());
            }
            std::panic!("src/numbers.rs is out of date with the coverage map; rerun with POLY_BLESS=1");
        }
        // And the compiled table agrees, both ways.
        assert_eq!(SYSCALLS.len(), entries.len());
        for ((n, sym), &(tn, tname)) in entries.iter().zip(SYSCALLS.iter()) {
            assert_eq!((*n, sym.as_str()), (tn, tname));
            assert_eq!(name(*n), Some(tname));
            assert_eq!(number(tname), Some(*n));
        }
    }

    #[test]
    fn well_known_numbers() {
        assert_eq!((nr::READ, nr::WRITE, nr::CLOSE, nr::MMAP, nr::BRK), (0, 1, 3, 9, 12));
        assert_eq!((nr::GETPID, nr::EXIT, nr::ARCH_PRCTL, nr::EXIT_GROUP), (39, 60, 158, 231));
        assert_eq!(name(400), None, "336-423 is a hole in the x86-64 table");
        assert_eq!(name(u64::MAX), None);
        assert_eq!(TABLE_LEN, 473);
    }

    #[test]
    fn handlers_are_registered_by_number() {
        for &(number, _) in IMPLEMENTED.iter().chain(FILE_SYSTEM) {
            assert!(name(number).is_some(), "handler for unknown number {number}");
            assert!(handler(number).is_some());
        }
        assert_eq!(implemented().count(), IMPLEMENTED.len() + FILE_SYSTEM.len());
        assert!(handler(nr::SWAPON).is_none());
        assert!(handler(TABLE_LEN as u64).is_none());
        assert!(handler(u64::MAX).is_none());
    }

    #[test]
    fn implemented_set() {
        let names: Vec<&str> =
            implemented().filter(|&(nr, _)| !FILE_SYSTEM.iter().any(|f| f.0 == nr)).map(|(_, n)| n).collect();
        let expected = [
            "read",
            "write",
            "close",
            "fstat",
            "mmap",
            "mprotect",
            "munmap",
            "brk",
            "rt_sigaction",
            "rt_sigprocmask",
            "rt_sigreturn",
            "ioctl",
            "readv",
            "writev",
            "getpid",
            "exit",
            "kill",
            "uname",
            "getuid",
            "getgid",
            "geteuid",
            "getegid",
            "getppid",
            "rt_sigpending",
            "rt_sigtimedwait",
            "rt_sigqueueinfo",
            "rt_sigsuspend",
            "sigaltstack",
            "arch_prctl",
            "gettid",
            "tkill",
            "futex",
            "set_tid_address",
            "restart_syscall",
            "clock_gettime",
            "exit_group",
            "tgkill",
            "set_robust_list",
            "get_robust_list",
            "rt_tgsigqueueinfo",
            "getrandom",
        ];
        assert_eq!(names, expected);
    }
}
