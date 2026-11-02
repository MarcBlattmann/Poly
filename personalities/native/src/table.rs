//! The native call table: every number of [`crate::abi::nr`], its name and
//! its handler.

use crate::abi::nr;
use crate::sys::{self, Handler};

/// `(number, name, handler)`, in number order.
pub const TABLE: [(u64, &str, Handler); 45] = [
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
    (nr::CONTROL, "control", sys::control),
    (nr::POLL, "poll", sys::poll),
    (nr::MAP_HANDLE, "map_handle", sys::map_handle),
    (nr::MEMORY_OBJECT, "memory_object", sys::memory_object),
    (nr::SEAL, "seal", sys::seal),
    (nr::SET_NONBLOCKING, "set_nonblocking", sys::set_nonblocking),
    (nr::HANDLE_INFO, "handle_info", sys::handle_info),
    (nr::STAT, "stat", sys::stat),
    (nr::RENAME, "rename", sys::rename),
    (nr::REMOVE, "remove", sys::remove),
    (nr::MAKE_DIR, "make_dir", sys::make_dir),
    (nr::SET_MODE, "set_mode", sys::set_mode),
    (nr::READ_DIR, "read_dir", sys::read_dir),
    (nr::SYNC, "sync", sys::sync),
    (nr::PIPE, "pipe", sys::pipe),
    (nr::MAKE_FIFO, "make_fifo", sys::make_fifo),
    (nr::SOCKET, "socket", sys::socket),
    (nr::SOCKET_PAIR, "socket_pair", sys::socket_pair),
    (nr::BIND, "bind", sys::bind),
    (nr::LISTEN, "listen", sys::listen),
    (nr::ACCEPT, "accept", sys::accept),
    (nr::CONNECT, "connect", sys::connect),
    (nr::SEND, "send", sys::send),
    (nr::RECV, "recv", sys::recv),
    (nr::SOCKET_PEER, "socket_peer", sys::socket_peer),
    (nr::PROCESS_HANDLE, "process_handle", sys::process_handle),
    (nr::SIGNAL, "signal", sys::signal),
    (nr::NET_INTERFACES, "net_interfaces", sys::net_interfaces),
    (nr::TRUNCATE, "truncate", sys::truncate),
];

/// The handler of call `number`, if there is one.
pub fn handler(number: u64) -> Option<Handler> {
    TABLE.get(usize::try_from(number).ok()?).map(|&(_, _, h)| h)
}

/// The name of call `number`.
pub fn name(number: u64) -> Option<&'static str> {
    TABLE.get(usize::try_from(number).ok()?).map(|&(_, n, _)| n)
}
