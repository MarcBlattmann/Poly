//! Notifications (ADR 0117): the toasts on the screen and the history,
//! fed by `org.freedesktop.Notifications` calls on the session bus.
//!
//! - **Toasts** stack in the top right corner below the ridge, newest on
//!   top, at most [`MAX_TOASTS`] at a time (more wait their turn). Each
//!   goes after its timeout: the sender's, or Alpine Shell's default of
//!   7 s when it asks for the server's default (-1); never for 0, nor for
//!   a critical one asking for the default (the specification says
//!   critical notifications should not expire by themselves). A click
//!   dismisses one.
//! - **`replaces_id`** updates a notification in place (the same ID, the
//!   timer started again); an unknown one gets a new ID.
//! - **The history** keeps the last [`HISTORY`] notifications (not
//!   `transient` ones), newest first; what came while it was not open
//!   counts as unread, shown on the ridge.
//! - Closing reasons for `NotificationClosed`: expired, dismissed, closed
//!   by `CloseNotification`.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use poly_dbus::notifications::{Notify, Urgency, reason};

/// Toasts on the screen at once.
pub const MAX_TOASTS: usize = 3;
/// Notifications kept in the history.
pub const HISTORY: usize = 50;
/// The server's default timeout (Alpine Shell's notices).
pub const DEFAULT_TIMEOUT_MS: u64 = 7000;

/// A notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub urgency: Urgency,
    pub actions: Vec<(String, String)>,
    /// When it came (ms on the caller's clock).
    pub arrived_ms: u64,
    pub transient: bool,
    /// Stays when an action is invoked.
    pub resident: bool,
    /// The sender asked for this timeout: -1 default, 0 never, else ms.
    pub expire_timeout: i32,
}

impl Notification {
    /// How long it stays on the screen, `None`: until dismissed.
    pub fn lifetime_ms(&self) -> Option<u64> {
        match self.expire_timeout {
            0 => None,
            t if t < 0 && self.urgency == Urgency::Critical => None,
            t if t < 0 => Some(DEFAULT_TIMEOUT_MS),
            t => Some(t as u64),
        }
    }
}

/// A toast on the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u32,
    /// When it goes, if it does.
    pub expires_ms: Option<u64>,
}

/// The notification service's state.
#[derive(Debug, Default)]
pub struct Notifications {
    next_id: u32,
    /// Every notification not closed yet, by ID.
    live: Vec<Notification>,
    /// On the screen, newest first.
    toasts: Vec<Toast>,
    /// Waiting for room, oldest first.
    waiting: VecDeque<u32>,
    history: VecDeque<Notification>,
    unread: usize,
    closed: Vec<(u32, u32)>,
    invoked: Vec<(u32, String)>,
    /// Something to draw changed.
    changed: bool,
}

impl Notifications {
    pub fn new() -> Notifications {
        Notifications { next_id: 1, ..Notifications::default() }
    }

    /// A `Notify` call at `now_ms`: the notification's ID.
    pub fn notify(&mut self, n: Notify, now_ms: u64) -> u32 {
        let replace = (n.replaces_id != 0 && self.live.iter().any(|l| l.id == n.replaces_id)).then_some(n.replaces_id);
        let id = replace.unwrap_or_else(|| {
            let id = self.next_id.max(1);
            self.next_id = id.wrapping_add(1).max(1);
            id
        });
        let note = Notification {
            id,
            app_name: n.app_name,
            summary: n.summary,
            body: n.body,
            urgency: n.urgency,
            actions: n.actions,
            arrived_ms: now_ms,
            transient: n.transient,
            resident: n.resident,
            expire_timeout: n.expire_timeout,
        };
        if !note.transient {
            self.history.retain(|h| h.id != id);
            self.history.push_front(note.clone());
            self.history.truncate(HISTORY);
            if replace.is_none() {
                self.unread += 1;
            }
        }
        let lifetime = note.lifetime_ms();
        match self.live.iter_mut().find(|l| l.id == id) {
            Some(l) => *l = note,
            None => self.live.push(note),
        }
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.expires_ms = lifetime.map(|l| now_ms + l);
        } else if !self.waiting.contains(&id) {
            self.waiting.push_back(id);
        }
        self.fill(now_ms);
        self.changed = true;
        id
    }

    /// Show waiting notifications while there is room.
    fn fill(&mut self, now_ms: u64) {
        while self.toasts.len() < MAX_TOASTS {
            let Some(id) = self.waiting.pop_front() else { break };
            let Some(n) = self.live.iter().find(|l| l.id == id) else { continue };
            let expires_ms = n.lifetime_ms().map(|l| now_ms + l);
            self.toasts.insert(0, Toast { id, expires_ms });
        }
    }

    /// Close notification `id` for `why` (a `reason`): off the screen,
    /// `NotificationClosed` due. False if it was not open.
    pub fn close(&mut self, id: u32, why: u32, now_ms: u64) -> bool {
        let Some(at) = self.live.iter().position(|l| l.id == id) else { return false };
        self.live.remove(at);
        self.toasts.retain(|t| t.id != id);
        self.waiting.retain(|&w| w != id);
        self.closed.push((id, why));
        self.fill(now_ms);
        self.changed = true;
        true
    }

    /// The toast was clicked: dismissed.
    pub fn dismiss(&mut self, id: u32, now_ms: u64) -> bool {
        self.close(id, reason::DISMISSED, now_ms)
    }

    /// Whether notification `id` offers action `key`.
    pub fn has_action(&self, id: u32, key: &str) -> bool {
        self.live.iter().any(|l| l.id == id && l.actions.iter().any(|a| a.0 == key))
    }

    /// Action `key` of notification `id` was chosen: `ActionInvoked` is
    /// due, and the notification goes unless it is resident.
    pub fn invoke(&mut self, id: u32, key: &str, now_ms: u64) -> bool {
        let Some(n) = self.live.iter().find(|l| l.id == id) else { return false };
        let resident = n.resident;
        self.invoked.push((id, key.into()));
        if !resident {
            self.close(id, reason::DISMISSED, now_ms);
        }
        self.changed = true;
        true
    }

    /// `(id, key)` of the actions invoked since the last call.
    pub fn take_invoked(&mut self) -> Vec<(u32, String)> {
        core::mem::take(&mut self.invoked)
    }

    /// Expire what is due at `now_ms`; true if something went.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let due: Vec<u32> =
            self.toasts.iter().filter(|t| t.expires_ms.is_some_and(|e| e <= now_ms)).map(|t| t.id).collect();
        for id in &due {
            self.close(*id, reason::EXPIRED, now_ms);
        }
        !due.is_empty()
    }

    /// When the next toast expires.
    pub fn next_deadline(&self) -> Option<u64> {
        self.toasts.iter().filter_map(|t| t.expires_ms).min()
    }

    /// `(id, reason)` of the notifications closed since the last call.
    pub fn take_closed(&mut self) -> Vec<(u32, u32)> {
        core::mem::take(&mut self.closed)
    }

    /// Whether something to draw changed since the last call.
    pub fn take_changed(&mut self) -> bool {
        core::mem::take(&mut self.changed)
    }

    /// The toasts, newest first, with their notifications.
    pub fn toasts(&self) -> Vec<&Notification> {
        self.toasts.iter().filter_map(|t| self.live.iter().find(|l| l.id == t.id)).collect()
    }

    pub fn history(&self) -> impl Iterator<Item = &Notification> {
        self.history.iter()
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn unread(&self) -> usize {
        self.unread
    }

    /// The history was looked at.
    pub fn mark_read(&mut self) {
        if self.unread != 0 {
            self.unread = 0;
            self.changed = true;
        }
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        self.unread = 0;
        self.changed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(summary: &str, timeout: i32) -> Notify {
        Notify { app_name: "t".into(), summary: summary.into(), expire_timeout: timeout, ..Notify::default() }
    }

    #[test]
    fn toasts_stack_and_expire() {
        let mut n = Notifications::new();
        let a = n.notify(note("a", -1), 0);
        let b = n.notify(note("b", 1000), 10);
        assert_eq!((a, b), (1, 2));
        assert_eq!(n.toasts().iter().map(|t| t.summary.as_str()).collect::<Vec<_>>(), ["b", "a"], "newest on top");
        assert_eq!(n.next_deadline(), Some(1010));
        assert!(!n.tick(1009));
        assert!(n.tick(1010));
        assert_eq!(n.take_closed(), [(b, reason::EXPIRED)]);
        // The default is 7 s.
        assert!(!n.tick(6999));
        assert!(n.tick(7000));
        assert_eq!(n.take_closed(), [(a, reason::EXPIRED)]);
        assert!(n.toasts().is_empty());
        // Both stay in the history, newest first, unread.
        assert_eq!(n.history().map(|h| h.summary.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(n.unread(), 2);
        n.mark_read();
        assert_eq!(n.unread(), 0);
    }

    #[test]
    fn at_most_three_at_a_time() {
        let mut n = Notifications::new();
        for i in 0..5 {
            n.notify(note(&alloc::format!("{i}"), 0), i);
        }
        assert_eq!(n.toasts().iter().map(|t| t.summary.as_str()).collect::<Vec<_>>(), ["2", "1", "0"]);
        assert_eq!(n.next_deadline(), None, "timeout 0: never");
        // Dismissing one makes room for the next waiting.
        assert!(n.dismiss(1, 100));
        assert_eq!(n.take_closed(), [(1, reason::DISMISSED)]);
        assert_eq!(n.toasts()[0].summary, "3");
        // Closing a waiting one by call: it never shows.
        assert!(n.close(5, reason::CLOSED, 100));
        assert!(!n.close(5, reason::CLOSED, 100), "only once");
        assert_eq!(n.toasts().len(), 3);
    }

    #[test]
    fn replacing_and_critical_and_transient() {
        let mut n = Notifications::new();
        let id = n.notify(note("progress 10%", 2000), 0);
        let mut again = note("progress 50%", 2000);
        again.replaces_id = id;
        assert_eq!(n.notify(again, 500), id, "the same ID");
        assert_eq!(n.toasts().len(), 1);
        assert_eq!(n.toasts()[0].summary, "progress 50%");
        assert_eq!(n.next_deadline(), Some(2500), "the timer started again");
        assert_eq!(n.history_len(), 1);
        assert_eq!(n.unread(), 1);
        // Replacing an unknown ID makes a new one.
        let mut stray = note("x", 0);
        stray.replaces_id = 99;
        assert_ne!(n.notify(stray, 0), 99);
        // Critical with the default timeout stays.
        let mut c = note("disk full", -1);
        c.urgency = Urgency::Critical;
        let cid = n.notify(c, 0);
        assert!(n.live.iter().find(|l| l.id == cid).unwrap().lifetime_ms().is_none());
        // Transient ones are not kept.
        let mut t = note("volume 40%", -1);
        t.transient = true;
        n.notify(t, 0);
        assert!(n.history().all(|h| h.summary != "volume 40%"));
        n.clear_history();
        assert_eq!((n.history_len(), n.unread()), (0, 0));
    }
}
