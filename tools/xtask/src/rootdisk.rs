//! The persistent root disk (roadmap Phase 4, "bash, coreutils, Python and
//! a C compiler run from the persistent disk"; ADR 0059).
//!
//! An ext4 image holding a small root tree of the host's own programs and
//! what they need, made at test time and never committed:
//!
//! - bash, GNU coreutils ([`COREUTILS`]), grep and sed from `/usr/bin`;
//! - Python 3 (the interpreter `/usr/bin/python3` leads to) with its
//!   standard library directory, less the parts no program here uses
//!   ([`PYTHON_SKIP`]);
//! - the C compiler: the `gcc` driver, `cc1`, `collect2`, `as`, `ld`, the
//!   start files, libgcc, the C library's link-time files (`libc.so`,
//!   `libc_nonshared.a`, `crt1.o`, ...) and the headers a C program
//!   includes, found by following `#include` lines from [`HEADERS`];
//! - every shared object those programs (and Python's extension modules,
//!   and `ld`'s plugin) need: the `DT_NEEDED` closure and the `PT_INTERP`
//!   interpreter, found the way ld.so(8) searches (`dynamic.rs`), reading
//!   only ELF headers. Nothing is run on the host to find them.
//!
//! Each file goes to the path it has on the host (its directory with
//! symbolic links resolved, so `/lib/x86_64-linux-gnu/libc.so.6` lands in
//! `/usr/lib/x86_64-linux-gnu` when `/lib` is a link to `usr/lib`); symbolic
//! links are copied as links, and the host's top-level links (`/bin` ->
//! `usr/bin`, ...) are made too. A program the host lacks is left out with a
//! note, and the boot test skips what needs it.
//!
//! The image is written with `mke2fs -t ext4 -d` when e2fsprogs are
//! installed (from a staging directory of hard links, or copies, under
//! `target/`), otherwise with `poly-ext2`'s `format` and the driver itself.
//!
//! Boot ([`boot_args`]): BusyBox's init from an initramfs
//! (`initramfs::disk_root_image`) mounts the disk (`/dev/vda`) on
//! `/mnt/root`, binds `/dev`, `/dev/pts`, `/dev/shm`, `/proc`, `/sys` and
//! `/tmp` into it, and runs `chroot /mnt/root /bin/bash -l` on the console.
//! `cargo xtask run --disk-root` boots it interactively (the image
//! `target/poly-root-disk.img` is kept between runs; `--fresh-disk` makes
//! it again); [`boot_test`] types into that bash.

use crate::libsearch::{SearchPath, own_paths};
use poly_elf::{Dynamic, Elf};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod boot;
mod collect;
mod fixtures;
mod image;
mod tree;

pub use boot::*;
pub use collect::*;
pub use fixtures::*;
pub use image::*;
pub use tree::*;

/// The image `cargo xtask run --disk-root` boots (kept between runs).
pub const FILE_NAME: &str = "poly-root-disk.img";
/// The image the boot test makes afresh every time.
pub const TEST_FILE_NAME: &str = "poly-root-disk-test.img";

/// The GNU coreutils programs copied from `/usr/bin` (or `/bin`).
pub const COREUTILS: &[&str] = &[
    "basename",
    "cat",
    "chmod",
    "cp",
    "cut",
    "date",
    "dd",
    "df",
    "dirname",
    "du",
    "echo",
    "env",
    "expr",
    "false",
    "head",
    "id",
    "ln",
    "ls",
    "md5sum",
    "mkdir",
    "mktemp",
    "mv",
    "nproc",
    "od",
    "printf",
    "pwd",
    "readlink",
    "realpath",
    "rm",
    "rmdir",
    "seq",
    "sha256sum",
    "sleep",
    "sort",
    "stat",
    "stty",
    "sync",
    "tail",
    "tee",
    "test",
    "touch",
    "tr",
    "true",
    "tty",
    "uname",
    "uniq",
    "wc",
    "whoami",
    "yes",
];

/// Parts of Python's standard library directory left out: tests, GUI and
/// installer pieces, documentation data and the static library build.
pub const PYTHON_SKIP: &[&str] = &[
    "test",
    "idlelib",
    "tkinter",
    "turtledemo",
    "ensurepip",
    "lib2to3",
    "pydoc_data",
    "venv",
    "site-packages",
    "dist-packages",
];

/// The headers whose `#include` closure goes into the image: what an
/// ordinary C program includes.
pub const HEADERS: &[&str] = &[
    "assert.h",
    "ctype.h",
    "errno.h",
    "fcntl.h",
    "inttypes.h",
    "limits.h",
    "math.h",
    "signal.h",
    "stdarg.h",
    "stdbool.h",
    "stddef.h",
    "stdint.h",
    "stdio.h",
    "stdlib.h",
    "string.h",
    "strings.h",
    "time.h",
    "unistd.h",
    "sys/mman.h",
    "sys/stat.h",
    "sys/time.h",
    "sys/types.h",
    "sys/wait.h",
];

#[cfg(test)]
mod tests;
