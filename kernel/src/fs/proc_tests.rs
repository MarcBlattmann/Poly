//! Kernel tests of `/proc`, `/sys` and `/dev` as the kernel itself (and so
//! the shell's `cat` and `ls`) sees them: from a kernel thread, which has
//! no process and so no `/proc/self`. The view of a Linux process is
//! tested in `linux/files_tests.rs`.

use super::{Files, OpenFlags};
use crate::kernel_test;
use alloc::string::String;

fn text(files: &Files, path: &str) -> Result<String, &'static str> {
    let (data, more) = files.read_to_end(path.as_bytes(), 64 * 1024).map_err(|e| {
        crate::kprintln!("proc test: {path}: {}", e.name());
        e.name()
    })?;
    if more {
        return Err("a /proc file is longer than 64 KiB");
    }
    String::from_utf8(data).map_err(|_| "not UTF-8")
}

kernel_test! {
    fn proc_system_files_read_from_the_kernel() -> TestResult {
        let files = Files::new();
        let mem = text(&files, "/proc/meminfo")?;
        let total_kb: u64 = mem
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("MemTotal:"))
            .and_then(|v| v.trim().strip_suffix(" kB"))
            .and_then(|v| v.trim().parse().ok())
            .ok_or("no MemTotal line")?;
        let frames = crate::frames::FRAMES.lock().as_ref().map_or(0, |f| f.total_frames());
        if total_kb != frames * 4 {
            crate::kprintln!("proc test: MemTotal {total_kb} kB, {frames} frames");
            return Err("MemTotal is not the frame allocator's total");
        }
        let cpu = text(&files, "/proc/cpuinfo")?;
        if !cpu.starts_with("processor\t: 0\n") || !cpu.contains("\nflags\t\t: fpu") || !cpu.contains(" lm") {
            crate::kprintln!("proc test: cpuinfo:\n{cpu}");
            return Err("cpuinfo lacks the processor, its flags or long mode");
        }
        let uptime = text(&files, "/proc/uptime")?;
        let secs: f64 = uptime.split_whitespace().next().and_then(|s| s.parse().ok()).ok_or("bad uptime")?;
        if secs <= 0.0 {
            return Err("uptime is not positive");
        }
        for (path, start) in [
            ("/proc/version", "Linux version "),
            ("/proc/sys/kernel/osrelease", "6.1.0-poly\n"),
            ("/proc/sys/kernel/random/poolsize", "4096\n"),
            ("/sys/devices/system/cpu/possible", "0\n"),
            ("/sys/devices/virtual/mem/urandom/dev", "1:9\n"),
        ] {
            if !text(&files, path)?.starts_with(start) {
                crate::kprintln!("proc test: {path} does not start with {start:?}");
                return Err("a system file has the wrong contents");
            }
        }
        // /proc/mounts is self/mounts: a kernel thread has no self.
        if files.readlinkat(super::AT_FDCWD, b"/proc/mounts").as_deref() != Ok(b"self/mounts".as_slice()) {
            return Err("/proc/mounts is not a link to self/mounts");
        }
        if files.readlinkat(super::AT_FDCWD, b"/proc/self") != Err(poly_vfs::Errno::ENOENT) {
            return Err("a kernel thread has a /proc/self");
        }
        Ok(())
    }
}

kernel_test! {
    fn proc_lists_directories_and_is_read_only() -> TestResult {
        let files = Files::new();
        let names: alloc::vec::Vec<_> = files.read_dir(b"/proc").map_err(|e| e.name())?.into_iter().map(|e| e.0).collect();
        for want in [&b"self"[..], b"cpuinfo", b"meminfo", b"sys", b"mounts"] {
            if !names.iter().any(|n| n == want) {
                return Err("/proc does not list a fixed file");
            }
        }
        let sys: alloc::vec::Vec<_> = files.read_dir(b"/sys").map_err(|e| e.name())?.into_iter().map(|e| e.0).collect();
        if !sys.iter().any(|n| n == b"devices") || !sys.iter().any(|n| n == b"class") {
            return Err("/sys lacks devices or class");
        }
        if files.open(b"/proc/meminfo", OpenFlags::WRONLY, 0) != Err(poly_vfs::Errno::EROFS)
            || files.open(b"/sys/devices/system/cpu/online", OpenFlags::WRONLY, 0) != Err(poly_vfs::Errno::EROFS)
        {
            return Err("procfs or sysfs accepted a write");
        }
        let st = files.statfs(b"/proc").map_err(|e| e.name())?;
        if st.magic != poly_vfs::PROC_SUPER_MAGIC || files.statfs(b"/sys").map_err(|e| e.name())?.magic != poly_vfs::SYSFS_MAGIC {
            return Err("wrong statfs magic numbers");
        }
        Ok(())
    }
}

kernel_test! {
    fn dev_has_random_devices_links_and_shm() -> TestResult {
        let files = Files::new();
        let (a, _) = files.read_to_end(b"/dev/urandom", 32).map_err(|e| e.name())?;
        let (b, _) = files.read_to_end(b"/dev/urandom", 32).map_err(|e| e.name())?;
        if a.len() != 32 || a == b {
            return Err("/dev/urandom did not give fresh bytes");
        }
        let (r, more) = files.read_to_end(b"/dev/random", 600).map_err(|e| e.name())?;
        if r.len() != 600 || !more {
            return Err("/dev/random reads came out wrong");
        }
        for (link, target) in [
            (&b"/dev/fd"[..], &b"/proc/self/fd"[..]),
            (b"/dev/stdin", b"/proc/self/fd/0"),
            (b"/dev/stdout", b"/proc/self/fd/1"),
            (b"/dev/stderr", b"/proc/self/fd/2"),
        ] {
            if files.readlinkat(super::AT_FDCWD, link).as_deref() != Ok(target) {
                return Err("a /dev link is missing or wrong");
            }
        }
        let st = files.stat(b"/dev/random").map_err(|e| e.name())?;
        if st.kind() != Some(poly_vfs::FileType::CharDevice) || st.rdev != poly_vfs::device::RANDOM || st.perm() != 0o666 {
            return Err("/dev/random is not the (1, 8) node, mode 0666");
        }
        let shm = files.stat(b"/dev/shm").map_err(|e| e.name())?;
        if shm.perm() != 0o1777 || files.statfs(b"/dev/shm").map_err(|e| e.name())?.magic != poly_vfs::TMPFS_MAGIC {
            return Err("/dev/shm is not a sticky tmpfs");
        }
        Ok(())
    }
}
