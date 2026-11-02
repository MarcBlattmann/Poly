//! `/bin/native-child`: what `native-hello` spawns (ADR 0125).
//!
//! - `native-child exit <n>`: print a line, exit with status `n`.
//! - `native-child panic`: panic (poly-rt's handler exits with 101).
//! - `native-child fault`: write to an unmapped address (a page fault kills it).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use poly_rt::{env, println};

poly_rt::entry!(main);

fn main() -> i32 {
    let args: Vec<&str> = env::args_str().collect();
    match args.get(1..).unwrap_or_default() {
        ["exit", n] => {
            let code = n.parse().unwrap_or(255);
            println!("native-child: exiting with {code}");
            code
        }
        ["panic"] => panic!("native-child panics on purpose"),
        ["fault"] => {
            // The first page is never mapped; hidden from the optimiser.
            let unmapped = core::hint::black_box(0x10usize) as *mut u32;
            // SAFETY: not sound, on purpose: the store faults and the
            // kernel kills the process, which is what the test checks.
            unsafe { core::ptr::write_volatile(unmapped, 1) };
            1
        }
        other => {
            println!("native-child: unknown arguments {other:?}");
            2
        }
    }
}
