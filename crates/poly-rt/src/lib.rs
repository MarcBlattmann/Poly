//! `poly-rt`: the runtime of native Poly programs (ADR 0125).
//!
//! A native program is a `no_std` + `alloc` Rust binary for the
//! `x86_64-unknown-none` target, linked by `rust-lld` as a static,
//! position-dependent ELF executable (`cargo xtask` builds the ones in
//! `userland/native/`). This crate gives it what `std` would:
//!
//! - the entry point `_start`, which reads the arguments and environment
//!   off the initial stack and calls the program's `main` ([`entry!`]),
//!   then exits with what `main` returns;
//! - Poly's ABI note, which makes the kernel run the program in the
//!   native personality;
//! - the system calls ([`sys`]), and on them files ([`fs`]), output
//!   ([`io`], [`print!`], [`println!`], [`eprintln!`]), memory ([`mem`]),
//!   time ([`time`]), processes ([`process`]) and [`env`];
//! - a global allocator: `poly-heap`'s slabs over pages from `map`
//!   ([`heap`]);
//! - a panic handler that prints the panic and exits with status 101.
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//! extern crate alloc;
//! use poly_rt::println;
//!
//! poly_rt::entry!(main);
//!
//! fn main() -> i32 {
//!     println!("hello from {}", poly_rt::process::id());
//!     0
//! }
//! ```
//!
//! On the host (`cargo test`), every system call fails with
//! [`Error::Unsupported`] and the entry point, allocator and panic handler
//! are left out, so the logic around them is tested like any crate.

#![no_std]

extern crate alloc;

pub mod env;
pub mod fs;
pub mod heap;
pub mod io;
pub mod mem;
pub mod process;
pub mod sys;
pub mod time;

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod start;

pub use poly_personality_native::abi;
pub use poly_personality_native::abi::Error;

/// A result with a native [`Error`].
pub type Result<T> = core::result::Result<T, Error>;

/// Name the program's `main`, a `fn() -> i32` whose value is the exit
/// status, for the entry point to call.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[unsafe(no_mangle)]
        extern "Rust" fn __poly_rt_main() -> i32 {
            $main()
        }
    };
}

/// Print to the standard output (one `write` per call).
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::io::_print($crate::abi::STDOUT, format_args!($($arg)*)) };
}

/// Print a line to the standard output (one `write` per call).
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => { $crate::io::_print($crate::abi::STDOUT, format_args!("{}\n", format_args!($($arg)*))) };
}

/// Print to the standard error.
#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => { $crate::io::_print($crate::abi::STDERR, format_args!($($arg)*)) };
}

/// Print a line to the standard error.
#[macro_export]
macro_rules! eprintln {
    () => { $crate::eprint!("\n") };
    ($($arg:tt)*) => { $crate::io::_print($crate::abi::STDERR, format_args!("{}\n", format_args!($($arg)*))) };
}
