//! What the requests that only some terminals answer return
//! ([`Driver::device_ioctl`](crate::Driver::device_ioctl)): a serial
//! port's `struct serial_struct` (`TIOCGSERIAL`, `linux/serial.h`), a
//! virtual console's `struct vt_stat` (`VT_GETSTATE`, `linux/vt.h`) and the
//! other console requests of ioctl_console(2) a program uses to find out
//! what its terminal is.

use crate::ioctl::{KDGKBTYPE, TIOCGSERIAL, VT_GETSTATE, VT_OPENQRY};
use alloc::vec::Vec;

/// `PORT_16550A` (`linux/serial.h`): a 16550A UART, `type` of a port.
pub const PORT_16550A: i32 = 4;
/// `KB_101` (`linux/kd.h`): a 101-key keyboard.
pub const KB_101: u8 = 0x02;

/// A serial port as `TIOCGSERIAL` describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SerialInfo {
    /// `PORT_*` (`type`).
    pub kind: i32,
    /// The port's number: `ttyS<line>`.
    pub line: i32,
    /// I/O port base.
    pub port: u32,
    pub irq: i32,
    /// The FIFO's size in bytes.
    pub xmit_fifo_size: i32,
    /// The UART's input clock divided by 16 (the highest speed).
    pub baud_base: i32,
}

impl SerialInfo {
    /// `struct serial_struct` on x86-64 (`linux/serial.h`): `int type, line;
    /// unsigned int port; int irq, flags, xmit_fifo_size, custom_divisor,
    /// baud_base; unsigned short close_delay; char io_type;
    /// char reserved_char[1]; int hub6; unsigned short closing_wait,
    /// closing_wait2; unsigned char *iomem_base; unsigned short
    /// iomem_reg_shift; unsigned int port_high; unsigned long iomap_base;`,
    /// 72 bytes. `io_type` 0 is `SERIAL_IO_PORT`.
    pub fn to_bytes(&self) -> [u8; 72] {
        let mut b = [0u8; 72];
        b[0..4].copy_from_slice(&self.kind.to_le_bytes());
        b[4..8].copy_from_slice(&self.line.to_le_bytes());
        b[8..12].copy_from_slice(&self.port.to_le_bytes());
        b[12..16].copy_from_slice(&self.irq.to_le_bytes());
        b[20..24].copy_from_slice(&self.xmit_fifo_size.to_le_bytes());
        b[28..32].copy_from_slice(&self.baud_base.to_le_bytes());
        b
    }
}

/// The answers of a serial port: `TIOCGSERIAL`.
pub fn serial_ioctl(info: &SerialInfo, request: u32) -> Option<Vec<u8>> {
    (request == TIOCGSERIAL).then(|| info.to_bytes().to_vec())
}

/// The answers of virtual console `number` (1 for `tty1`), the only one
/// and the active one: `VT_GETSTATE` (`struct vt_stat { unsigned short
/// v_active, v_signal, v_state; }`, `v_state` a bit per open console),
/// `VT_OPENQRY` (the first console not open: none, -1) and `KDGKBTYPE`.
pub fn vt_ioctl(number: u16, request: u32) -> Option<Vec<u8>> {
    match request {
        VT_GETSTATE => {
            let mut b = Vec::with_capacity(6);
            b.extend_from_slice(&number.to_le_bytes());
            b.extend_from_slice(&0u16.to_le_bytes());
            b.extend_from_slice(&(1u16 << number).to_le_bytes());
            Some(b)
        }
        VT_OPENQRY => Some((-1i32).to_le_bytes().to_vec()),
        KDGKBTYPE => Some(alloc::vec![KB_101]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_struct_layout() {
        let info =
            SerialInfo { kind: PORT_16550A, line: 0, port: 0x3f8, irq: 4, xmit_fifo_size: 16, baud_base: 115_200 };
        let b = serial_ioctl(&info, TIOCGSERIAL).unwrap();
        assert_eq!(b.len(), 72);
        assert_eq!(&b[0..4], &4i32.to_le_bytes());
        assert_eq!(&b[8..12], &0x3f8u32.to_le_bytes());
        assert_eq!(&b[12..16], &4i32.to_le_bytes());
        assert_eq!(&b[28..32], &115_200i32.to_le_bytes());
        assert!(serial_ioctl(&info, VT_GETSTATE).is_none());
    }

    #[test]
    fn vt_state() {
        assert_eq!(vt_ioctl(1, VT_GETSTATE).unwrap(), [1, 0, 0, 0, 2, 0]);
        assert_eq!(vt_ioctl(1, VT_OPENQRY).unwrap(), (-1i32).to_le_bytes());
        assert_eq!(vt_ioctl(1, KDGKBTYPE).unwrap(), [KB_101]);
        assert!(vt_ioctl(1, TIOCGSERIAL).is_none());
    }
}
