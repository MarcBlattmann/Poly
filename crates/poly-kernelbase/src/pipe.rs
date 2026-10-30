//! Anonymous pipes (Learn: "CreatePipe", "Anonymous Pipes", "Anonymous
//! Pipe Operations"), ADR 0093.
//!
//! Learn: "Anonymous pipes are implemented using a named pipe with a
//! unique name". Poly has no named pipes yet; its stand-in is the NT
//! personality's `\Device\PolyPipe\<name>` byte stream (ADR 0067), which
//! ends for its readers once every open with write access has closed
//! (`ReadFile` then fails with `ERROR_BROKEN_PIPE`, as Learn's `ReadFile`
//! page says of an anonymous pipe whose write handle was closed).
//! `CreatePipe` opens a new, uniquely named stream twice: the read end
//! with read access, the write end with write access. `nSize` (a buffer
//! size suggestion) is not used: Poly's stream grows as needed, so a write
//! never waits for a reader (Poly's choice).

use alloc::vec::Vec;

/// `FILE_READ_DATA`, `FILE_WRITE_DATA`, `FILE_READ_ATTRIBUTES`,
/// `FILE_WRITE_ATTRIBUTES` (`winnt.h`), `SYNCHRONIZE`.
const FILE_READ_DATA: u32 = 0x0001;
const FILE_WRITE_DATA: u32 = 0x0002;
const FILE_READ_ATTRIBUTES: u32 = 0x0080;
const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
const SYNCHRONIZE: u32 = 0x0010_0000;

/// The read end's access.
pub const READ_ACCESS: u32 = FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE;
/// The write end's access.
pub const WRITE_ACCESS: u32 = FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES | SYNCHRONIZE;

/// The NT name of the `serial`-th pipe of process `pid`.
pub fn nt_name(pid: u32, serial: u32) -> Vec<u16> {
    use core::fmt::Write;
    let mut s = alloc::string::String::from("\\Device\\PolyPipe\\Anonymous.");
    let _ = write!(s, "{pid:08x}.{serial:08x}");
    s.encode_utf16().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_per_process_and_serial() {
        let a = alloc::string::String::from_utf16(&nt_name(0x24, 1)).unwrap();
        assert_eq!(a, "\\Device\\PolyPipe\\Anonymous.00000024.00000001");
        assert_ne!(nt_name(0x24, 2), nt_name(0x24, 1));
        assert_ne!(nt_name(0x25, 1), nt_name(0x24, 1));
        assert_eq!(READ_ACCESS & FILE_WRITE_DATA, 0);
        assert_eq!(WRITE_ACCESS & FILE_READ_DATA, 0);
    }
}
