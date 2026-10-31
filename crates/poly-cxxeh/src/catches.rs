//! The catch blocks in progress on one thread (`__CxxFrameHandler3.yaml`,
//! rules `rethrow`, `rethrow_without_exception`, `new_exception_in_catch`,
//! `nested_independent`, `state_after_catch`; `_CxxThrowException.yaml`,
//! `rethrow_taken_by_seh`).
//!
//! A catch block runs while the frames between it and the throw are still
//! on the stack below it, so the frame that holds its try block (the
//! *target frame*) is found again by a stack walk of a later exception, at
//! the address in its try block it called from. Its state then is not that
//! address's but the state around the try statement, which this list
//! keeps. The list also says which exception a rethrow means (the
//! innermost catch block still running) and when an exception object's
//! life ends:
//!
//! - when its catch block ends, unless another running catch block still
//!   has it (a rethrow taken inside the catch block);
//! - when its catch block is left by an unwind (a new exception, or one an
//!   `__except` outside takes), unless that unwind goes to a C++ catch
//!   block that takes the very same object (a rethrow taken further out).
//!
//! An entry stays (marked abandoned) after its catch block was left by an
//! unwind, until its target frame is unwound or a later catch block's
//! target frame is at or above it: the target frame's own handler is asked
//! after the catch block's frames and still needs the state.

use crate::throw::Exception;
use alloc::vec::Vec;

/// One catch block in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveCatch {
    pub id: u64,
    /// The establisher frame of the frame that holds the try block.
    pub frame: u64,
    /// That frame's function (its start address), to tell a function from
    /// a funclet should two frames ever share an address.
    pub function: u64,
    /// The target frame's state while the catch block runs.
    pub state: i32,
    /// The exception being handled.
    pub exception: Exception,
    /// The catch block was left by an unwind.
    pub abandoned: bool,
}

/// The catch blocks in progress, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catches {
    list: Vec<ActiveCatch>,
    next_id: u64,
}

impl Catches {
    pub const fn new() -> Catches {
        Catches { list: Vec::new(), next_id: 1 }
    }

    pub fn entries(&self) -> &[ActiveCatch] {
        &self.list
    }

    /// A catch block starts for the try block in `frame`: everything at or
    /// below that frame has been unwound, so older entries there go.
    pub fn push(&mut self, frame: u64, function: u64, state: i32, exception: Exception) -> u64 {
        self.prune(frame, true);
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.list.push(ActiveCatch { id, frame, function, state, exception, abandoned: false });
        id
    }

    /// `frame` is being unwound: entries for frames below it (and, unless it
    /// is the unwind's target, at it) are gone.
    pub fn prune(&mut self, frame: u64, including: bool) {
        self.list.retain(|c| if including { c.frame > frame } else { c.frame >= frame });
    }

    /// The state of the frame `frame` of `function` while a catch block of
    /// one of its try blocks runs.
    pub fn state_of(&self, frame: u64, function: u64) -> Option<i32> {
        self.list.iter().rev().find(|c| c.frame == frame && c.function == function).map(|c| c.state)
    }

    /// The exception being handled: the innermost catch block still
    /// running.
    pub fn current(&self) -> Option<&Exception> {
        self.list.iter().rev().find(|c| !c.abandoned).map(|c| &c.exception)
    }

    pub fn get(&self, id: u64) -> Option<&ActiveCatch> {
        self.list.iter().find(|c| c.id == id)
    }

    /// Another catch block still running (not `id`) has this object.
    fn held_elsewhere(&self, id: u64, object: u64) -> bool {
        self.list.iter().any(|c| c.id != id && !c.abandoned && c.exception.is_cxx() && c.exception.object() == object)
    }

    /// The catch block `id` ended: it goes (and whatever was left above it).
    /// The exception to destroy, if its life ends now.
    pub fn finish(&mut self, id: u64) -> Option<Exception> {
        let pos = self.list.iter().position(|c| c.id == id)?;
        let e = self.list[pos].exception;
        self.list.truncate(pos);
        (destroyable(&e) && !self.held_elsewhere(id, e.object())).then_some(e)
    }

    /// The catch block `id` is left by an unwind; `taken_by` is the
    /// exception a C++ catch block further out takes, if the unwind goes to
    /// one. The exception to destroy, if its life ends now.
    pub fn abandon(&mut self, id: u64, taken_by: Option<&Exception>) -> Option<Exception> {
        let c = self.list.iter_mut().find(|c| c.id == id && !c.abandoned)?;
        c.abandoned = true;
        let e = c.exception;
        if !destroyable(&e) {
            return None;
        }
        if taken_by.is_some_and(|t| t.is_cxx() && t.object() == e.object()) {
            return None;
        }
        (!self.held_elsewhere(id, e.object())).then_some(e)
    }
}

/// A C++ exception with an object to destroy.
fn destroyable(e: &Exception) -> bool {
    e.is_cxx() && e.object() != 0 && e.throw_info() != 0
}
