//! `WSAEventSelect` and `WSAEnumNetworkEvents` (Learn: "WSAEventSelect",
//! "WSAEnumNetworkEvents", "WSANETWORKEVENTS"; ADR 0155).
//!
//! A program associates an event object and a mask of `FD_*` events with a
//! socket; the system sets the event when one of the events happens and
//! the program asks `WSAEnumNetworkEvents` which ones, with their error
//! codes. Poly's sockets report readiness (`select`'s device request); a
//! helper thread in `ws2_32` polls the selected sockets and feeds this
//! state machine, which decides what Learn says is recorded:
//!
//! | Event | Recorded when |
//! |---|---|
//! | `FD_ACCEPT` | a connection is waiting on a listening socket; again after `accept` |
//! | `FD_CONNECT` | a non-blocking connect finished, with its error code |
//! | `FD_READ` | data (or a datagram) can be read; again after `recv` if more is there |
//! | `FD_WRITE` | data can be sent: after the connection is made, and again after a send failed with `WSAEWOULDBLOCK` |
//! | `FD_CLOSE` | the peer closed the connection (after the data was read) or the connection failed (with the error code) |
//!
//! `FD_OOB` and the quality-of-service events never happen on Poly's stack.

use crate::consts::{WSAECONNREFUSED, WSAECONNRESET};

pub const FD_READ: u32 = 0x01;
pub const FD_WRITE: u32 = 0x02;
pub const FD_OOB: u32 = 0x04;
pub const FD_ACCEPT: u32 = 0x08;
pub const FD_CONNECT: u32 = 0x10;
pub const FD_CLOSE: u32 = 0x20;
/// `FD_ALL_EVENTS` (`winsock2.h`): every bit a mask may name.
pub const FD_ALL_EVENTS: u32 = 0x3FF;
/// `FD_MAX_EVENTS`: the length of `WSANETWORKEVENTS.iErrorCode`.
pub const FD_MAX_EVENTS: usize = 10;
/// `sizeof(WSANETWORKEVENTS)`: `long lNetworkEvents; int iErrorCode[10];`.
pub const NETWORK_EVENTS_SIZE: usize = 4 + 4 * FD_MAX_EVENTS;

/// The bit index of a single `FD_*` event (`FD_READ_BIT` ...).
pub fn bit(event: u32) -> usize {
    event.trailing_zeros() as usize
}

/// What a socket is doing, as far as events are concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Neither connecting nor listening (a datagram socket, or a stream that is not connected yet).
    Idle,
    Connecting,
    Connected,
    Listening,
    /// The connection ended (`FD_CLOSE` was or is being recorded).
    Closed,
}

/// One poll of the socket by the helper thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub readable: bool,
    pub writable: bool,
    pub error: bool,
    pub hangup: bool,
    /// Bytes that can be read now (`FIONREAD`).
    pub bytes: u32,
    /// The pending socket error (`SO_ERROR`), 0 if none.
    pub so_error: u32,
}

/// A socket's association with an event object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub mask: u32,
    /// The event object handle, set when something is recorded.
    pub event: isize,
    pub state: State,
    armed_read: bool,
    armed_write: bool,
    armed_accept: bool,
    close_reported: bool,
    /// Events recorded and not yet taken by `WSAEnumNetworkEvents`.
    pub pending: u32,
    pub errors: [u32; FD_MAX_EVENTS],
}

impl Registration {
    /// `WSAEventSelect(s, event, mask)` on a socket that is `listening`, `connected` (a stream
    /// with a peer) or neither.
    pub fn new(mask: u32, event: isize, listening: bool, connected: bool) -> Registration {
        Registration {
            mask,
            event,
            state: if listening {
                State::Listening
            } else if connected {
                State::Connected
            } else {
                State::Idle
            },
            armed_read: true,
            armed_write: true,
            armed_accept: true,
            close_reported: false,
            pending: 0,
            errors: [0; FD_MAX_EVENTS],
        }
    }

    /// `WSAEventSelect` again on the same socket: a new mask and event; events are evaluated afresh.
    pub fn reselect(&mut self, mask: u32, event: isize) {
        self.mask = mask;
        self.event = event;
        self.armed_read = true;
        self.armed_write = true;
        self.armed_accept = true;
        self.pending = 0;
        self.errors = [0; FD_MAX_EVENTS];
    }

    fn record(&mut self, event: u32, error: u32) {
        if self.mask & event != 0 {
            self.pending |= event;
            self.errors[bit(event)] = error;
        }
    }

    /// A non-blocking connect returned `WSAEWOULDBLOCK`: `FD_CONNECT` follows.
    pub fn connecting(&mut self) {
        self.state = State::Connecting;
    }

    /// A connect returned at once (success): `FD_CONNECT` is recorded and the socket may be written to.
    pub fn connected(&mut self) {
        if self.state != State::Connected {
            self.state = State::Connected;
            self.record(FD_CONNECT, 0);
            self.armed_write = true;
        }
    }

    /// `recv` ran: `FD_READ` may be recorded again.
    pub fn after_recv(&mut self) {
        self.armed_read = true;
    }

    /// `send` failed with `WSAEWOULDBLOCK`: `FD_WRITE` is recorded when sending is possible again.
    pub fn after_send_blocked(&mut self) {
        self.armed_write = true;
    }

    /// `accept` ran: `FD_ACCEPT` may be recorded again.
    pub fn after_accept(&mut self) {
        self.armed_accept = true;
    }

    /// Whether the helper should wait for readability and for writability: an event that is
    /// recorded only once (until the program acts) is not waited for again meanwhile.
    pub fn wants(&self) -> (bool, bool) {
        match self.state {
            State::Listening => (self.armed_accept, false),
            State::Connecting => (false, true),
            State::Connected | State::Idle => (self.armed_read, self.armed_write),
            State::Closed => (false, false),
        }
    }

    /// Fold one poll into the recorded events; true if an event is pending afterwards (the event
    /// object is to be set).
    pub fn step(&mut self, i: &Inputs) -> bool {
        let fail_code = |code: u32, default: u32| if code != 0 { code } else { default };
        match self.state {
            State::Listening => {
                if i.readable && self.armed_accept {
                    self.record(FD_ACCEPT, 0);
                    self.armed_accept = false;
                }
            }
            State::Connecting => {
                if i.error {
                    self.record(FD_CONNECT, fail_code(i.so_error, WSAECONNREFUSED));
                    self.state = State::Closed;
                } else if i.writable {
                    self.record(FD_CONNECT, 0);
                    self.state = State::Connected;
                    self.armed_write = true;
                    self.data(i);
                }
            }
            State::Connected | State::Idle => self.data(i),
            State::Closed => {}
        }
        self.pending != 0
    }

    /// Data, writability and the end of a connection.
    fn data(&mut self, i: &Inputs) {
        if i.error && self.state == State::Connected && !self.close_reported {
            self.record(FD_CLOSE, if i.so_error != 0 { i.so_error } else { WSAECONNRESET });
            self.close_reported = true;
            self.state = State::Closed;
            return;
        }
        let has_data = i.bytes > 0;
        if (has_data || (i.readable && !i.hangup)) && self.armed_read {
            self.record(FD_READ, 0);
            self.armed_read = false;
        }
        if i.hangup && !has_data && self.state == State::Connected && !self.close_reported {
            self.record(FD_CLOSE, i.so_error);
            self.close_reported = true;
            self.state = State::Closed;
        }
        if i.writable && !i.hangup && self.armed_write {
            self.record(FD_WRITE, 0);
            self.armed_write = false;
        }
    }

    /// `WSAEnumNetworkEvents`: the events recorded since the last call with their error codes; the
    /// record is cleared.
    pub fn take(&mut self) -> (u32, [u32; FD_MAX_EVENTS]) {
        let r = (self.pending, self.errors);
        self.pending = 0;
        self.errors = [0; FD_MAX_EVENTS];
        r
    }
}

/// `WSAEventSelect`'s argument checks: a non-zero mask needs an event object, a mask may only name
/// the `FD_*` bits. `Err` is the WSA error.
pub fn check_select(event: isize, mask: u32) -> Result<(), u32> {
    use crate::consts::WSAEINVAL;
    if mask & !FD_ALL_EVENTS != 0 || (mask != 0 && event == 0) {
        return Err(WSAEINVAL);
    }
    Ok(())
}

/// The bytes of a `WSANETWORKEVENTS`.
pub fn encode_network_events(events: u32, errors: &[u32; FD_MAX_EVENTS]) -> [u8; NETWORK_EVENTS_SIZE] {
    let mut b = [0u8; NETWORK_EVENTS_SIZE];
    b[0..4].copy_from_slice(&events.to_le_bytes());
    for (i, e) in errors.iter().enumerate() {
        b[4 + 4 * i..8 + 4 * i].copy_from_slice(&e.to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::*;

    const ALL: u32 = FD_READ | FD_WRITE | FD_ACCEPT | FD_CONNECT | FD_CLOSE;

    fn poll(readable: bool, writable: bool) -> Inputs {
        Inputs { readable, writable, ..Inputs::default() }
    }

    #[test]
    fn a_connected_socket_is_writable_at_once_and_readable_when_data_comes() {
        let mut r = Registration::new(FD_READ | FD_WRITE | FD_CLOSE, 7, false, true);
        assert!(r.step(&poll(false, true)));
        assert_eq!(r.take(), (FD_WRITE, [0; 10]));
        // Still writable: not recorded again until a send blocked.
        assert!(!r.step(&poll(false, true)));
        r.after_send_blocked();
        assert!(r.step(&poll(false, true)));
        assert_eq!(r.take().0, FD_WRITE);
        // Data arrives.
        let d = Inputs { readable: true, writable: true, bytes: 100, ..Inputs::default() };
        assert!(r.step(&d));
        assert_eq!(r.take().0, FD_READ);
        // Not again until recv ran, then again if data is still there.
        assert!(!r.step(&d));
        r.after_recv();
        assert!(r.step(&d));
        assert_eq!(r.take().0, FD_READ);
    }

    #[test]
    fn the_mask_filters_what_is_recorded() {
        let mut r = Registration::new(FD_CLOSE, 7, false, true);
        let d = Inputs { readable: true, writable: true, bytes: 5, ..Inputs::default() };
        assert!(!r.step(&d));
        assert_eq!(r.take().0, 0);
        // But the armed state moved: with FD_READ selected later it is armed afresh.
        r.reselect(FD_READ, 7);
        assert!(r.step(&d));
        assert_eq!(r.take().0, FD_READ);
    }

    #[test]
    fn a_listener_records_accept_once_until_accept_ran() {
        let mut r = Registration::new(FD_ACCEPT, 7, true, false);
        assert!(!r.step(&poll(false, false)));
        assert!(r.step(&poll(true, false)));
        assert_eq!(r.take().0, FD_ACCEPT);
        assert!(!r.step(&poll(true, false)));
        r.after_accept();
        assert!(r.step(&poll(true, false)));
    }

    #[test]
    fn a_connect_that_finishes_records_connect_then_write() {
        let mut r = Registration::new(ALL, 7, false, false);
        r.connecting();
        assert!(!r.step(&poll(false, false)));
        assert!(r.step(&poll(false, true)));
        let (ev, errs) = r.take();
        assert_eq!(ev, FD_CONNECT | FD_WRITE);
        assert_eq!(errs, [0; 10]);
        assert_eq!(r.state, State::Connected);
    }

    #[test]
    fn a_connect_that_fails_records_the_error() {
        let mut r = Registration::new(ALL, 7, false, false);
        r.connecting();
        let i = Inputs { error: true, so_error: WSAECONNREFUSED, ..Inputs::default() };
        assert!(r.step(&i));
        let (ev, errs) = r.take();
        assert_eq!(ev, FD_CONNECT);
        assert_eq!(errs[bit(FD_CONNECT)], WSAECONNREFUSED);
        assert_eq!(r.state, State::Closed);
        // Without an error code from the socket, a refusal.
        let mut r = Registration::new(FD_CONNECT, 7, false, false);
        r.connecting();
        r.step(&Inputs { error: true, ..Inputs::default() });
        assert_eq!(r.take().1[bit(FD_CONNECT)], WSAECONNREFUSED);
    }

    #[test]
    fn a_connect_that_returned_at_once_records_connect_when_connected() {
        let mut r = Registration::new(ALL, 7, false, false);
        r.connected();
        assert_eq!(r.take().0, FD_CONNECT);
        assert!(r.step(&poll(false, true)));
        assert_eq!(r.take().0, FD_WRITE);
    }

    #[test]
    fn the_peer_closing_is_recorded_after_the_data_was_read() {
        let mut r = Registration::new(FD_READ | FD_CLOSE, 7, false, true);
        // Data and the end of the stream together: the data first.
        let both = Inputs { readable: true, hangup: true, bytes: 10, ..Inputs::default() };
        assert!(r.step(&both));
        assert_eq!(r.take().0, FD_READ);
        assert!(!r.step(&both));
        // Read: only the end remains.
        let end = Inputs { readable: true, hangup: true, ..Inputs::default() };
        assert!(r.step(&end));
        let (ev, errs) = r.take();
        assert_eq!(ev, FD_CLOSE);
        assert_eq!(errs[bit(FD_CLOSE)], 0);
        // Recorded once.
        assert!(!r.step(&end));
    }

    #[test]
    fn a_reset_connection_records_close_with_the_error() {
        let mut r = Registration::new(FD_CLOSE, 7, false, true);
        assert!(r.step(&Inputs { error: true, hangup: true, ..Inputs::default() }));
        let (ev, errs) = r.take();
        assert_eq!(ev, FD_CLOSE);
        assert_eq!(errs[bit(FD_CLOSE)], WSAECONNRESET);
        let mut r = Registration::new(FD_CLOSE, 7, false, true);
        r.step(&Inputs { error: true, so_error: WSAECONNABORTED, ..Inputs::default() });
        assert_eq!(r.take().1[bit(FD_CLOSE)], WSAECONNABORTED);
    }

    #[test]
    fn the_helper_waits_only_for_what_can_still_be_recorded() {
        let mut r = Registration::new(FD_READ | FD_WRITE, 7, false, true);
        assert_eq!(r.wants(), (true, true));
        r.step(&Inputs { readable: true, writable: true, bytes: 3, ..Inputs::default() });
        assert_eq!(r.wants(), (false, false));
        r.after_recv();
        assert_eq!(r.wants(), (true, false));
        r.after_send_blocked();
        assert_eq!(r.wants(), (true, true));
        let mut l = Registration::new(FD_ACCEPT, 7, true, false);
        assert_eq!(l.wants(), (true, false));
        l.step(&poll(true, false));
        assert_eq!(l.wants(), (false, false));
        let mut c = Registration::new(FD_CONNECT, 7, false, false);
        c.connecting();
        assert_eq!(c.wants(), (false, true));
    }

    #[test]
    fn take_clears_and_arguments_are_checked() {
        let mut r = Registration::new(FD_READ, 7, false, true);
        r.step(&Inputs { readable: true, bytes: 1, ..Inputs::default() });
        assert_eq!(r.take().0, FD_READ);
        assert_eq!(r.take(), (0, [0; 10]));
        assert_eq!(check_select(0, 0), Ok(()));
        assert_eq!(check_select(5, FD_READ | FD_CLOSE), Ok(()));
        assert_eq!(check_select(0, FD_READ), Err(WSAEINVAL));
        assert_eq!(check_select(5, 0x4000), Err(WSAEINVAL));
    }

    #[test]
    fn the_network_events_record_has_learns_layout() {
        let mut e = [0u32; 10];
        e[bit(FD_CONNECT)] = WSAECONNREFUSED;
        let b = encode_network_events(FD_CONNECT, &e);
        assert_eq!(b.len(), 44);
        assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 0x10);
        assert_eq!(u32::from_le_bytes(b[4 + 4 * 4..8 + 4 * 4].try_into().unwrap()), WSAECONNREFUSED);
        assert_eq!(bit(FD_CLOSE), 5);
        assert_eq!(bit(FD_READ), 0);
    }
}
