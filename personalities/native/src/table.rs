//! The native call table: every number of [`crate::abi::nr`], its name and
//! its handler.

use crate::abi::nr;
use crate::sys::{self, Handler};

/// `(number, name, handler)`, in number order.
pub const TABLE: [(u64, &str, Handler); 16] = [
    (nr::EXIT, "exit", sys::exit),
    (nr::WRITE, "write", sys::write),
    (nr::READ, "read", sys::read),
    (nr::OPEN, "open", sys::open),
    (nr::CLOSE, "close", sys::close),
    (nr::DUP, "dup", sys::dup),
    (nr::SEEK, "seek", sys::seek),
    (nr::MAP, "map", sys::map),
    (nr::UNMAP, "unmap", sys::unmap),
    (nr::PROTECT, "protect", sys::protect),
    (nr::CLOCK, "clock", sys::clock),
    (nr::SLEEP, "sleep", sys::sleep),
    (nr::YIELD, "yield", sys::yield_now),
    (nr::SPAWN, "spawn", sys::spawn),
    (nr::WAIT, "wait", sys::wait),
    (nr::PROCESS_ID, "process_id", sys::process_id),
];

/// The handler of call `number`, if there is one.
pub fn handler(number: u64) -> Option<Handler> {
    TABLE.get(usize::try_from(number).ok()?).map(|&(_, _, h)| h)
}

/// The name of call `number`.
pub fn name(number: u64) -> Option<&'static str> {
    TABLE.get(usize::try_from(number).ok()?).map(|&(_, n, _)| n)
}
