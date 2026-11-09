//! `/bin/native-child`: what `native-hello` spawns (ADR 0125).
//!
//! - `native-child exit <n>`: print a line, exit with status `n`.
//! - `native-child panic`: panic (poly-rt's handler exits with 101).
//! - `native-child fault`: write to an unmapped address (a page fault kills it).
//! - `native-child echo-handle <h>`: on the socket handle `h`, receive a
//!   byte with a handle, read that handle to its end and send back
//!   `got: <what it read>`.
//! - `native-child wait`: sleep until it is ended.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::time::Duration;
use poly_rt::fs::File;
use poly_rt::io::{Read, Write};
use poly_rt::net::UnixStream;
use poly_rt::{env, println, time};

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
        ["echo-handle", h] => echo_handle(h.parse().unwrap_or(u32::MAX)),
        ["wait"] => loop {
            let _ = time::sleep(Duration::from_secs(3600));
        },
        other => {
            println!("native-child: unknown arguments {other:?}");
            2
        }
    }
}

fn echo_handle(h: u32) -> i32 {
    let mut sock = UnixStream::from_handle(h);
    let (mut buf, mut handles) = ([0u8; 16], Vec::new());
    match sock.recv_with_handles(&mut buf, &mut handles) {
        Ok(1) if handles.len() == 1 => {}
        other => {
            println!("native-child: no byte with a handle ({other:?}, {} handles)", handles.len());
            return 3;
        }
    }
    let mut passed = File::from_handle(handles[0]);
    let mut got = Vec::from(&b"got: "[..]);
    if passed.read_to_end(&mut got).is_err() || sock.write_all(&got).is_err() {
        return 4;
    }
    0
}
