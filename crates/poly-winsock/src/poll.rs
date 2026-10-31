//! `select` and `WSAPoll` over `\Device\Afd`'s readiness (Learn:
//! "select", "WSAPoll", "WSAPOLLFD"). The device reports `WSAPoll`'s bits
//! for each socket ([`from_readiness`]); `select`'s three sets are read
//! from them:
//!
//! | Set | Ready when |
//! |---|---|
//! | read | data or a connection to accept (`POLLRDNORM`), or the connection closed or reset (`POLLHUP`) |
//! | write | data can be sent, which includes a non-blocking connect that succeeded (`POLLWRNORM`) |
//! | except | a non-blocking connect that failed, or another pending error (`POLLERR`) |
//!
//! Out-of-band data is not supported by Poly's stack, so it never makes a
//! socket ready.

use crate::consts::*;

const RDNORM: u32 = POLLRDNORM as u16 as u32;
const WRNORM: u32 = POLLWRNORM as u16 as u32;
const ERR: u32 = POLLERR as u16 as u32;
const HUP: u32 = POLLHUP as u16 as u32;
const NVAL: u32 = POLLNVAL as u16 as u32;

/// The device's bits for a socket that is `readable` (data, a connection
/// to accept, or the end of the stream), `writable`, has an `error`, or
/// was hung up.
pub fn from_readiness(readable: bool, writable: bool, error: bool, hangup: bool) -> u32 {
    let mut r = 0;
    if readable {
        r |= RDNORM;
    }
    if writable {
        r |= WRNORM;
    }
    if error {
        r |= ERR;
    }
    if hangup {
        r |= HUP;
    }
    r
}

/// What `select` asks the device to watch for a socket in the sets.
pub fn select_interest(read: bool, write: bool, except: bool) -> u32 {
    let mut e = 0;
    if read {
        e |= RDNORM | HUP;
    }
    if write {
        e |= WRNORM;
    }
    if except {
        e |= ERR;
    }
    e
}

/// Whether a socket with the device's `revents` is in each `select` set.
pub fn select_ready(revents: u32) -> (bool, bool, bool) {
    (revents & (RDNORM | HUP) != 0, revents & WRNORM != 0 && revents & ERR == 0, revents & ERR != 0)
}

/// `WSAPoll`'s `events` as the device's interest: `POLLIN` asks for
/// `POLLRDNORM` (and `POLLRDBAND`, never set), `POLLOUT` for `POLLWRNORM`;
/// errors and hang-ups are always reported. Learn: any other bit
/// (`POLLPRI` included, which Windows does not support) makes the call
/// fail with `WSAEINVAL`.
pub fn wsapoll_interest(events: i16) -> Result<u32, u32> {
    let allowed = POLLRDNORM | POLLRDBAND | POLLWRNORM | POLLWRBAND;
    if events & !allowed != 0 {
        return Err(WSAEINVAL);
    }
    Ok(u32::from(events as u16) & (RDNORM | WRNORM))
}

/// The `revents` `WSAPoll` reports for the device's bits.
pub fn wsapoll_revents(events: i16, device: u32) -> i16 {
    let want = u32::from(events as u16) | ERR | HUP | NVAL;
    (device & want) as u16 as i16
}

/// The bit `WSAPoll` reports for a handle that is not a socket.
pub const fn nval() -> u32 {
    NVAL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_sets() {
        let i = select_interest(true, true, true);
        assert_eq!(i, 0x0100 | 0x0002 | 0x0010 | 0x0001);
        assert_eq!(select_ready(from_readiness(true, false, false, false)), (true, false, false));
        assert_eq!(select_ready(from_readiness(false, true, false, false)), (false, true, false));
        // A refused non-blocking connect: except, not write.
        assert_eq!(select_ready(from_readiness(false, true, true, true)), (true, false, true));
        assert_eq!(select_ready(0), (false, false, false));
    }

    #[test]
    fn wsapoll() {
        assert_eq!(wsapoll_interest(POLLIN), Ok(0x0100));
        assert_eq!(wsapoll_interest(POLLOUT | POLLRDNORM), Ok(0x0110));
        assert_eq!(wsapoll_interest(POLLPRI), Err(WSAEINVAL));
        assert_eq!(wsapoll_interest(POLLERR), Err(WSAEINVAL));
        let dev = from_readiness(true, true, false, true);
        assert_eq!(wsapoll_revents(POLLIN, dev), POLLRDNORM | POLLHUP);
        assert_eq!(wsapoll_revents(POLLOUT, dev), POLLWRNORM | POLLHUP);
        assert_eq!(wsapoll_revents(0, nval()), POLLNVAL);
    }
}
