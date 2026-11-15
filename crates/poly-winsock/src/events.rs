//! `WSAEventSelect` and `WSAEnumNetworkEvents` (Learn: "WSAEventSelect",
//! "WSAEnumNetworkEvents", "WSANETWORKEVENTS"; ADR 0155; the rules are the ones the spec sheets of
//! both functions observed on Windows).
//!
//! A program associates an event object and a mask of `FD_*` events with a
//! socket; the system sets the event when one of the events happens and
//! the program asks `WSAEnumNetworkEvents` which ones, with their error
//! codes. Poly's sockets report readiness (`select`'s device request); a
//! helper thread in `ws2_32` polls the selected sockets and feeds this
//! state machine, which decides what is recorded:
//!
//! | Event | Recorded when |
//! |---|---|
//! | `FD_ACCEPT` | a connection is waiting on a listening socket; again after `accept` if another one waits |
//! | `FD_CONNECT` | a non-blocking connect finished, with its error code |
//! | `FD_READ` | data arrives (also while earlier data is unread); again after any `recv` (a peek too) that leaves data; when selected for it while data is unread that no earlier record covered |
//! | `FD_WRITE` | once when selected for it on a writable socket (with the connection); again only after a send hit `WSAEWOULDBLOCK` and can be sent again |
//! | `FD_CLOSE` | the peer closed the connection (`FD_READ` is recorded with it when data is unread), error 0; the connection was reset: `WSAECONNABORTED`; never after a local `shutdown(SD_RECEIVE)` / `SD_BOTH` |
//!
//! Selecting the same event again keeps what was recorded and what was consumed (only an event that
//! is new in the mask is evaluated afresh). `FD_OOB` and the quality-of-service events never happen
//! on Poly's stack.

use crate::consts::{WSAECONNABORTED, WSAECONNREFUSED, WSAECONNRESET};

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
    /// Neither connecting nor listening (a datagram socket, a stream that is not connected yet, or
    /// one whose connect failed).
    Idle,
    Connecting,
    Connected,
    Listening,
    /// The connection was reset (`FD_CLOSE` was recorded): nothing more happens.
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
    /// `shutdown(SD_RECEIVE)` or `SD_BOTH` ran: the end of the connection is not news.
    local_rd_shut: bool,
    /// The connect failed: nothing is watched until the next one.
    failed: bool,
    /// The bytes that could be read at the last poll (a rise is data arriving).
    seen_bytes: u32,
    /// The look at the socket that was folded in last (a number from a counter that is taken
    /// before the look): an older look, finished later by another thread, is not folded in.
    pub last_look: u64,
    /// Events recorded and not yet taken by `WSAEnumNetworkEvents`.
    pub pending: u32,
    pub errors: [u32; FD_MAX_EVENTS],
}

/// The error `FD_CLOSE` carries for a connection that was reset: Windows reports
/// `WSAECONNABORTED` (while `recv` and `send` fail with `WSAECONNRESET`); another error stays.
fn close_error(so_error: u32) -> u32 {
    if so_error == 0 || so_error == WSAECONNRESET { WSAECONNABORTED } else { so_error }
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
            local_rd_shut: false,
            failed: false,
            seen_bytes: 0,
            last_look: 0,
            pending: 0,
            errors: [0; FD_MAX_EVENTS],
        }
    }

    /// `WSAEventSelect` again on the same socket: a new mask and event. Only an event that is new
    /// in the mask is evaluated afresh; what was consumed stays consumed.
    pub fn reselect(&mut self, mask: u32, event: isize) {
        let new = mask & !self.mask;
        if new & FD_READ != 0 {
            self.armed_read = true;
        }
        if new & FD_WRITE != 0 {
            self.armed_write = true;
        }
        if new & FD_ACCEPT != 0 {
            self.armed_accept = true;
        }
        self.mask = mask;
        self.event = event;
        self.pending &= mask;
        for (i, e) in self.errors.iter_mut().enumerate() {
            if mask & (1 << i) == 0 {
                *e = 0;
            }
        }
    }

    fn record(&mut self, event: u32, error: u32) -> bool {
        if self.mask & event != 0 {
            self.pending |= event;
            self.errors[bit(event)] = error;
            true
        } else {
            false
        }
    }

    /// A non-blocking connect returned `WSAEWOULDBLOCK`: `FD_CONNECT` follows.
    pub fn connecting(&mut self) {
        self.state = State::Connecting;
        self.failed = false;
        self.close_reported = false;
        self.local_rd_shut = false;
        self.armed_read = true;
        self.armed_write = true;
        self.seen_bytes = 0;
    }

    /// A connect returned at once (success): `FD_CONNECT` is recorded and the socket may be written to.
    pub fn connected(&mut self) {
        if self.state != State::Connected {
            self.connecting();
            self.state = State::Connected;
            self.record(FD_CONNECT, 0);
        }
    }

    /// `recv` ran (a peek too): `FD_READ` may be recorded again.
    pub fn after_recv(&mut self) {
        self.armed_read = true;
    }

    /// A `recv` left no data: a recorded `FD_READ` that was not taken is dropped (the event object
    /// stays signalled, Windows' own), and the next data counts as arriving.
    pub fn data_drained(&mut self) {
        self.pending &= !FD_READ;
        self.errors[bit(FD_READ)] = 0;
        self.seen_bytes = 0;
    }

    /// `send` failed with `WSAEWOULDBLOCK` (or an overlapped one is pending): `FD_WRITE` is
    /// recorded when sending is possible again.
    pub fn after_send_blocked(&mut self) {
        self.armed_write = true;
    }

    /// `accept` ran: `FD_ACCEPT` may be recorded again.
    pub fn after_accept(&mut self) {
        self.armed_accept = true;
    }

    /// `shutdown` ran on the socket (`receive`: the receiving side was shut down).
    pub fn local_shutdown(&mut self, receive: bool) {
        if receive {
            self.local_rd_shut = true;
        }
    }

    /// Whether the helper should wait for readability and for writability: data is always watched
    /// (a rise is news); writability only while it can be recorded.
    pub fn wants(&self) -> (bool, bool) {
        if self.failed {
            return (false, false);
        }
        match self.state {
            State::Listening => (self.armed_accept, false),
            State::Connecting => (false, true),
            State::Connected | State::Idle => (true, self.armed_write),
            State::Closed => (false, false),
        }
    }

    /// Whether the end of the connection and errors are still to be watched.
    pub fn watches_end(&self) -> bool {
        !self.failed && !matches!(self.state, State::Closed | State::Idle) && !self.close_reported
    }

    /// Fold one poll into the recorded events; true if an event is pending afterwards (the event
    /// object is to be set).
    pub fn step(&mut self, i: &Inputs) -> bool {
        match self.state {
            State::Listening => {
                if i.readable && self.armed_accept && self.record(FD_ACCEPT, 0) {
                    self.armed_accept = false;
                }
            }
            State::Connecting => {
                if i.error {
                    let code = if i.so_error != 0 { i.so_error } else { WSAECONNREFUSED };
                    self.record(FD_CONNECT, code);
                    // The socket may connect again: nothing is watched until it does.
                    self.state = State::Idle;
                    self.failed = true;
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
        let connected = self.state == State::Connected;
        if i.error && connected && !self.close_reported {
            self.record(FD_CLOSE, close_error(i.so_error));
            self.close_reported = true;
            self.state = State::Closed;
            return;
        }
        let arrival = i.bytes > self.seen_bytes;
        self.seen_bytes = i.bytes;
        let has_data = i.bytes > 0 || (i.readable && !i.hangup);
        if has_data && (self.armed_read || arrival) && self.record(FD_READ, 0) {
            self.armed_read = false;
        }
        if i.hangup && connected && !self.close_reported && !self.local_rd_shut {
            self.record(FD_CLOSE, 0);
            self.close_reported = true;
        }
        if i.writable && !i.hangup && self.armed_write && self.record(FD_WRITE, 0) {
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
    fill_network_events(&mut b, events, errors);
    b
}

/// Write a record into the program's `WSANETWORKEVENTS`: `lNetworkEvents`, and the error slot of
/// each event that happened; every other slot keeps what the program put there (Windows' own
/// behaviour).
pub fn fill_network_events(out: &mut [u8; NETWORK_EVENTS_SIZE], events: u32, errors: &[u32; FD_MAX_EVENTS]) {
    out[0..4].copy_from_slice(&events.to_le_bytes());
    for (i, e) in errors.iter().enumerate() {
        if events & (1 << i) != 0 {
            out[4 + 4 * i..8 + 4 * i].copy_from_slice(&e.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::*;

    const ALL: u32 = FD_READ | FD_WRITE | FD_ACCEPT | FD_CONNECT | FD_CLOSE;

    fn poll(readable: bool, writable: bool) -> Inputs {
        Inputs { readable, writable, ..Inputs::default() }
    }

    fn data(bytes: u32) -> Inputs {
        Inputs { readable: true, writable: true, bytes, ..Inputs::default() }
    }

    #[test]
    fn a_connected_socket_is_writable_at_once_and_again_only_after_a_blocked_send() {
        let mut r = Registration::new(FD_READ | FD_WRITE | FD_CLOSE, 7, false, true);
        assert!(r.step(&poll(false, true)));
        assert_eq!(r.take(), (FD_WRITE, [0; 10]));
        // Still writable: not recorded again until a send blocked.
        assert!(!r.step(&poll(false, true)));
        r.after_send_blocked();
        assert!(r.step(&poll(false, true)));
        assert_eq!(r.take().0, FD_WRITE);
    }

    #[test]
    fn read_is_recorded_on_arrival_even_while_data_is_unread_and_after_a_recv_that_leaves_data() {
        let mut r = Registration::new(FD_READ | FD_CLOSE, 7, false, true);
        assert!(r.step(&data(10)));
        assert_eq!(r.take().0, FD_READ);
        // The same data is not news again.
        assert!(!r.step(&data(10)));
        // 5 more bytes arrive while the first 10 are unread: recorded again.
        assert!(r.step(&data(15)));
        assert_eq!(r.take().0, FD_READ);
        assert!(!r.step(&data(15)));
        // A recv of 4 bytes that leaves data: recorded again.
        r.after_recv();
        assert!(r.step(&data(11)));
        assert_eq!(r.take().0, FD_READ);
        // Everything read: nothing; data again: recorded.
        r.after_recv();
        assert!(!r.step(&poll(false, true)));
        assert!(r.step(&data(2)));
    }

    #[test]
    fn a_recv_that_drains_the_data_drops_the_recorded_read() {
        let mut r = Registration::new(FD_READ | FD_CLOSE, 7, false, true);
        assert!(r.step(&data(10)));
        // The program reads it all before asking: the record has no FD_READ any more.
        r.after_recv();
        r.data_drained();
        assert_eq!(r.take(), (0, [0; 10]));
        // New data is news again.
        assert!(r.step(&data(3)));
        assert_eq!(r.take().0, FD_READ);
    }

    #[test]
    fn selecting_again_keeps_what_was_consumed_and_evaluates_what_is_new() {
        let mut r = Registration::new(FD_READ | FD_CLOSE, 7, false, true);
        assert!(r.step(&data(2)));
        r.take();
        // Re-selecting the same mask with data unread: not signalled.
        r.reselect(FD_READ | FD_CLOSE, 7);
        assert!(!r.step(&data(2)));
        // A mask without FD_READ: data arrives, nothing recorded; selecting FD_READ then records it.
        let mut r = Registration::new(FD_CLOSE, 7, false, true);
        assert!(!r.step(&data(2)));
        r.reselect(FD_READ, 7);
        assert!(r.step(&data(2)));
        assert_eq!(r.take().0, FD_READ);
        // FD_WRITE already reported: selecting it again does not report it again.
        let mut r = Registration::new(FD_WRITE, 7, false, true);
        assert!(r.step(&poll(false, true)));
        r.take();
        r.reselect(FD_WRITE, 7);
        assert!(!r.step(&poll(false, true)));
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
    fn a_connect_that_fails_records_the_error_and_watches_nothing_until_the_next_connect() {
        let mut r = Registration::new(ALL, 7, false, false);
        r.connecting();
        let i = Inputs { error: true, hangup: true, so_error: WSAECONNREFUSED, ..Inputs::default() };
        assert!(r.step(&i));
        let (ev, errs) = r.take();
        assert_eq!(ev, FD_CONNECT);
        assert_eq!(errs[bit(FD_CONNECT)], WSAECONNREFUSED);
        assert_eq!(r.wants(), (false, false));
        assert!(!r.watches_end());
        // Without an error code from the socket, a refusal.
        let mut r = Registration::new(FD_CONNECT, 7, false, false);
        r.connecting();
        r.step(&Inputs { error: true, ..Inputs::default() });
        assert_eq!(r.take().1[bit(FD_CONNECT)], WSAECONNREFUSED);
        // It can connect again.
        r.connecting();
        assert_eq!(r.wants(), (false, true));
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
    fn the_peer_closing_is_one_record_with_the_data_it_sent_before() {
        let mut r = Registration::new(FD_READ | FD_CLOSE, 7, false, true);
        // Data and the end of the stream together: both at once.
        let both = Inputs { readable: true, hangup: true, bytes: 3, ..Inputs::default() };
        assert!(r.step(&both));
        let (ev, errs) = r.take();
        assert_eq!(ev, FD_READ | FD_CLOSE);
        assert_eq!(errs[bit(FD_CLOSE)], 0);
        // Once only.
        assert!(!r.step(&both));
        assert!(!r.watches_end());
        // A close without data is FD_CLOSE alone.
        let mut r = Registration::new(FD_READ | FD_CLOSE, 7, false, true);
        assert!(r.step(&Inputs { hangup: true, ..Inputs::default() }));
        assert_eq!(r.take().0, FD_CLOSE);
    }

    #[test]
    fn a_local_shutdown_of_the_receiving_side_is_not_news() {
        let mut r = Registration::new(FD_CLOSE, 7, false, true);
        r.local_shutdown(true);
        assert!(!r.step(&Inputs { hangup: true, ..Inputs::default() }));
        let mut r = Registration::new(FD_CLOSE, 7, false, true);
        r.local_shutdown(false);
        assert!(r.step(&Inputs { hangup: true, ..Inputs::default() }));
    }

    #[test]
    fn a_reset_connection_records_close_with_connection_aborted() {
        let mut r = Registration::new(FD_CLOSE, 7, false, true);
        assert!(r.step(&Inputs { error: true, hangup: true, ..Inputs::default() }));
        let (ev, errs) = r.take();
        assert_eq!(ev, FD_CLOSE);
        assert_eq!(errs[bit(FD_CLOSE)], WSAECONNABORTED);
        let mut r = Registration::new(FD_CLOSE, 7, false, true);
        r.step(&Inputs { error: true, so_error: WSAECONNRESET, ..Inputs::default() });
        assert_eq!(r.take().1[bit(FD_CLOSE)], WSAECONNABORTED);
        // Another error is reported as it is.
        let mut r = Registration::new(FD_CLOSE, 7, false, true);
        r.step(&Inputs { error: true, so_error: WSAETIMEDOUT, ..Inputs::default() });
        assert_eq!(r.take().1[bit(FD_CLOSE)], WSAETIMEDOUT);
    }

    #[test]
    fn the_helper_watches_data_always_and_the_rest_only_while_it_can_be_recorded() {
        let mut r = Registration::new(FD_READ | FD_WRITE, 7, false, true);
        assert_eq!(r.wants(), (true, true));
        r.step(&data(3));
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
    fn the_network_events_record_has_learns_layout_and_leaves_other_slots_alone() {
        let mut e = [0u32; 10];
        e[bit(FD_CONNECT)] = WSAECONNREFUSED;
        let b = encode_network_events(FD_CONNECT, &e);
        assert_eq!(b.len(), 44);
        assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 0x10);
        assert_eq!(u32::from_le_bytes(b[4 + 4 * 4..8 + 4 * 4].try_into().unwrap()), WSAECONNREFUSED);
        assert_eq!(bit(FD_CLOSE), 5);
        assert_eq!(bit(FD_READ), 0);
        // The caller's own values stay in the slots of events that did not happen.
        let mut out = [0x77u8; NETWORK_EVENTS_SIZE];
        fill_network_events(&mut out, FD_READ, &[0; 10]);
        assert_eq!(&out[0..4], &[1, 0, 0, 0]);
        assert_eq!(&out[4..8], &[0, 0, 0, 0]);
        assert_eq!(&out[8..12], &[0x77; 4]);
    }
}
