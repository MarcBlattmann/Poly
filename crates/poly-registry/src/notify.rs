//! Change notifications (`NtNotifyChangeKey`, Learn: "ZwNotifyChangeKey",
//! "RegNotifyChangeKeyValue").
//!
//! A watch belongs to an open instance ([`OpenId`]). The first
//! `NtNotifyChangeKey` on it sets the filter and whether the subtree counts;
//! later calls keep them and only queue another request (Learn:
//! "CompletionFilter and WatchTree are set on the first call and apply to
//! all subsequent calls using the same KeyHandle"). Each request is
//! one-time: the next matching change completes every queued request of
//! the watch ([`Fired`], `STATUS_SUCCESS`), and the caller asks again for
//! the next one. Changes while no request is queued are not remembered
//! (the Windows XP behaviour Learn describes as old; to be confirmed by
//! the spec agent).
//!
//! What fires which filter:
//!
//! - `REG_NOTIFY_CHANGE_NAME`: a subkey created or deleted (the parent
//!   changed).
//! - `REG_NOTIFY_CHANGE_LAST_SET`: a value created, changed (to different
//!   type or data) or deleted.
//! - `REG_NOTIFY_CHANGE_ATTRIBUTES`: `NtSetInformationKey`
//!   (`KeyWriteTimeInformation`), to be confirmed by the spec agent.
//! - `REG_NOTIFY_CHANGE_SECURITY`: never (no security descriptors yet).
//!
//! With `WatchTree` a change anywhere below the watched key counts. When
//! the open instance ends, its queued requests complete with
//! `STATUS_NOTIFY_CLEANUP` ([`Registry::close`]); when the watched key is
//! deleted, with `STATUS_KEY_DELETED` (to be confirmed by the spec agent).
//! The status a satisfied request completes with is to be confirmed too.

use crate::consts::*;
use crate::tree::{KeyId, OpenId, Registry, Result};
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::*;

/// A watch on an open instance.
#[derive(Clone, Debug)]
pub(crate) struct Watch {
    pub(crate) filter: u32,
    pub(crate) subtree: bool,
    /// Queued requests (the caller's cookies), oldest first.
    pub(crate) pending: VecDeque<u64>,
}

/// A notification request that completed: the caller signals its event and
/// writes its I/O status block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fired {
    pub open: OpenId,
    /// The cookie the request was queued with.
    pub request: u64,
    pub status: NtStatus,
}

impl Registry {
    /// Queue a notification request `request` on `open`. A filter with no
    /// `REG_NOTIFY_CHANGE_*` bit, or with bits outside
    /// [`REG_LEGAL_CHANGE_FILTER`], is `STATUS_INVALID_PARAMETER` (to be
    /// confirmed by the spec agent).
    pub fn notify(&mut self, open: OpenId, filter: u32, subtree: bool, request: u64) -> Result<()> {
        if filter & !REG_LEGAL_CHANGE_FILTER != 0 || filter & 0xF == 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }
        self.key_of(open)?;
        let o = self.opens.get_mut(open.index, open.generation).ok_or(STATUS_INVALID_HANDLE)?;
        let w = o.watch.get_or_insert(Watch { filter, subtree, pending: VecDeque::new() });
        w.pending.push_back(request);
        Ok(())
    }

    /// Take back a queued request that never reached its caller (the
    /// service failed after queueing it). Returns whether it was queued.
    pub fn cancel_request(&mut self, open: OpenId, request: u64) -> bool {
        let Some(w) = self.opens.get_mut(open.index, open.generation).and_then(|o| o.watch.as_mut()) else {
            return false;
        };
        let before = w.pending.len();
        w.pending.retain(|&r| r != request);
        before != w.pending.len()
    }

    /// Requests queued on `open`.
    pub fn pending_requests(&self, open: OpenId) -> usize {
        self.opens.get(open.index, open.generation).and_then(|o| o.watch.as_ref()).map_or(0, |w| w.pending.len())
    }

    /// The next completed request.
    pub fn pop_fired(&mut self) -> Option<Fired> {
        self.fired.pop_front()
    }

    /// `key` changed in a way `what` (one `REG_NOTIFY_CHANGE_*` bit)
    /// describes: complete the requests of every watch it satisfies.
    pub(crate) fn changed(&mut self, key: KeyId, what: u32) {
        let mut hits: Vec<OpenId> = Vec::new();
        for (id, o) in self.opens.iter() {
            let Some(w) = &o.watch else { continue };
            if w.pending.is_empty() || w.filter & what == 0 {
                continue;
            }
            if o.key == key || (w.subtree && self.is_at_or_below(key, o.key)) {
                hits.push(OpenId::new(id));
            }
        }
        for open in hits {
            self.complete(open, STATUS_SUCCESS);
        }
    }

    /// Complete every request on watches of `key` itself with `status`.
    pub(crate) fn complete_watches_of(&mut self, key: KeyId, status: NtStatus) {
        let opens: Vec<OpenId> =
            self.opens.iter().filter(|(_, o)| o.key == key).map(|(id, _)| OpenId::new(id)).collect();
        for open in opens {
            self.complete(open, status);
        }
    }

    fn complete(&mut self, open: OpenId, status: NtStatus) {
        let Some(w) = self.opens.get_mut(open.index, open.generation).and_then(|o| o.watch.as_mut()) else { return };
        let drained: Vec<u64> = w.pending.drain(..).collect();
        for request in drained {
            self.fired.push_back(Fired { open, request, status });
        }
    }
}
