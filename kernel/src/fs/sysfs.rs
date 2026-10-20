//! A minimal sysfs (sysfs(5)): the CPUs, the character devices the kernel
//! has, and the top-level directories programs expect. Built once at boot
//! as a read-only [`StaticFs`](poly_procfs::StaticFs).
//!
//! ```text
//! /sys/devices/system/cpu/{online,possible,present}   "0-3\n": the CPUs running (no hot-plug)
//! /sys/devices/system/cpu/cpu<n>/
//! /sys/devices/virtual/mem/{null,zero,full,random,urandom}/dev   "major:minor\n"
//! /sys/devices/virtual/tty/{console,tty}/dev
//! /sys/dev/char/<major>:<minor> -> ../../devices/virtual/<class>/<name>
//! /sys/class/{mem,tty}/<name>   -> ../../devices/virtual/<class>/<name>
//! /sys/{block,bus,firmware,fs,kernel,module,power}/   empty for now
//! ```
//!
//! The `dev` files and the `/sys/dev/char` links are sysfs(5)'s example
//! (`/sys/dev/char/1:3` -> `../../devices/virtual/mem/null`, whose `dev`
//! reads `1:3`).

use alloc::format;
use alloc::sync::Arc;
use poly_procfs::{StaticFs, StaticFsBuilder};
use poly_vfs::Timespec;
use poly_vfs::device::{CONSOLE, FULL, NULL, RANDOM, TTY, URANDOM, ZERO};

pub fn build() -> Arc<StaticFs> {
    let boot = Timespec::new((crate::time::realtime_ns() / crate::time::NANOS_PER_SEC) as i64, 0);
    let mut b = StaticFsBuilder::new("sysfs", poly_vfs::SYSFS_MAGIC, boot);
    // The CPUs that run, in the list format of `cpu/online` ("0", "0-3",
    // ...). They are up by the time this is built, and CPUs do not come
    // and go, so the three lists are the same.
    let cpus = crate::smp::online_cpus();
    let list = format!("{cpus}\n");
    for name in ["online", "possible", "present"] {
        b.text(&format!("devices/system/cpu/{name}"), &list);
    }
    for cpu in cpus.iter() {
        b.dir(&format!("devices/system/cpu/cpu{cpu}"));
    }
    let devices = [
        ("mem", "null", NULL),
        ("mem", "zero", ZERO),
        ("mem", "full", FULL),
        ("mem", "random", RANDOM),
        ("mem", "urandom", URANDOM),
        ("tty", "tty", TTY),
        ("tty", "console", CONSOLE),
    ];
    for (class, name, id) in devices {
        let dir = format!("devices/virtual/{class}/{name}");
        b.text(&format!("{dir}/dev"), &format!("{}:{}\n", id.major, id.minor));
        b.link(&format!("dev/char/{}:{}", id.major, id.minor), &format!("../../{dir}"));
        b.link(&format!("class/{class}/{name}"), &format!("../../{dir}"));
    }
    b.dir("dev/block");
    for dir in ["block", "bus", "firmware", "fs", "kernel", "module", "power"] {
        b.dir(dir);
    }
    b.build()
}
