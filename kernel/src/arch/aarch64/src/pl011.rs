//! The Arm PrimeCell UART (PL011), as described by its Technical Reference
//! Manual (Arm DDI 0183): register offsets, the bits the stub uses, and the
//! baud rate divisors.

/// Data register: write a byte to send, read a received byte.
pub const UARTDR: usize = 0x000;
/// Flag register.
pub const UARTFR: usize = 0x018;
/// Integer baud rate divisor.
pub const UARTIBRD: usize = 0x024;
/// Fractional baud rate divisor.
pub const UARTFBRD: usize = 0x028;
/// Line control register.
pub const UARTLCR_H: usize = 0x02c;
/// Control register.
pub const UARTCR: usize = 0x030;
/// Interrupt mask set/clear register.
pub const UARTIMSC: usize = 0x038;
/// Interrupt clear register.
pub const UARTICR: usize = 0x044;

/// `UARTFR`: the UART is transmitting (shift register not empty).
pub const FR_BUSY: u32 = 1 << 3;
/// `UARTFR`: the receive FIFO is empty.
pub const FR_RXFE: u32 = 1 << 4;
/// `UARTFR`: the transmit FIFO is full.
pub const FR_TXFF: u32 = 1 << 5;

/// `UARTLCR_H`: FIFOs enabled.
pub const LCR_H_FEN: u32 = 1 << 4;
/// `UARTLCR_H`: 8 data bits (WLEN = 0b11).
pub const LCR_H_WLEN_8: u32 = 0b11 << 5;

/// `UARTCR`: UART enable.
pub const CR_UARTEN: u32 = 1 << 0;
/// `UARTCR`: transmit enable.
pub const CR_TXE: u32 = 1 << 8;
/// `UARTCR`: receive enable.
pub const CR_RXE: u32 = 1 << 9;

/// `UARTICR`: clear every interrupt.
pub const ICR_ALL: u32 = 0x7ff;

/// `(UARTIBRD, UARTFBRD)` for `baud` from a UART clock of `clock_hz`: the
/// divisor is `clock / (16 * baud)`, its integer part in IBRD and its
/// fraction times 64, rounded, in FBRD (TRM, "Fractional baud rate divisor").
/// `None` if `baud` is 0 or the divisor does not fit (IBRD is 16 bits and
/// may not be 0).
pub fn baud_divisors(clock_hz: u32, baud: u32) -> Option<(u16, u8)> {
    if baud == 0 {
        return None;
    }
    // 64 * clock / (16 * baud) = 4 * clock / baud, rounded to nearest.
    let sixty_fourths = (4 * u64::from(clock_hz) + u64::from(baud) / 2) / u64::from(baud);
    let integer = u16::try_from(sixty_fourths >> 6).ok().filter(|&i| i != 0)?;
    Some((integer, (sixty_fourths & 63) as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The TRM's worked example: 3.6864 MHz and 115200 baud is exactly 2.
    #[test]
    fn exact_divisor() {
        assert_eq!(baud_divisors(3_686_400, 115_200), Some((2, 0)));
    }

    /// QEMU's `virt` clocks the UART at 24 MHz: 13.0208..., fraction 1.33/64.
    #[test]
    fn divisor_for_24_mhz() {
        assert_eq!(baud_divisors(24_000_000, 115_200), Some((13, 1)));
    }

    #[test]
    fn rejects_what_does_not_fit() {
        assert_eq!(baud_divisors(24_000_000, 0), None);
        // Divisor below 1: the clock is too slow for the rate.
        assert_eq!(baud_divisors(1_000_000, 115_200), None);
        assert_eq!(baud_divisors(u32::MAX, 1), None);
    }
}
