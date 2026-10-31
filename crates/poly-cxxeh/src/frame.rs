//! The decisions for one frame (`__CxxFrameHandler3.yaml`, rules `state`,
//! `try_block_choice`, `in_catch_block`, `match_*`, `non_cxx_exceptions`,
//! `unwind_phase`, errors `terminate_noexcept_flag`).
//!
//! A frame is the function itself, one of its catch funclets, or one of
//! its unwind funclets: all of them name the same handler and FuncInfo,
//! and the IP-to-state map covers their code. The frame's state tells
//! which: catch blocks' code runs in the states `TryHigh+1..=CatchHigh` of
//! their try block, which the function's own code never has. In a catch
//! funclet's frame only the try blocks inside that catch block apply; the
//! try blocks around the whole try statement belong to the frame that
//! holds it (the function, or an outer catch funclet), whose state while
//! the catch block runs is kept by [`crate::catches`]. That way every try
//! block is looked at in the frame whose code it is, the catch's
//! continuation is in that frame, and unwinding a catch funclet's frame
//! stops at its own states.

use crate::consts::*;
use crate::tables::{Func, HandlerType, TryBlock};
use crate::throw::{Exception, Match, Thrown, match_clause};
use crate::{Bad, count};
use alloc::vec::Vec;

/// Which code a frame runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    /// The function's own code.
    Function,
    /// A catch block of try block `try_index`; `handler` is the clause
    /// whose funclet the frame's function is, if found.
    Catch { try_index: u32, handler: Option<u32> },
    /// An unwind action (a destructor funclet).
    Cleanup,
}

/// The kind of the frame at `state` whose function starts at `begin_rva`.
pub fn frame_kind(f: &Func, state: i32, begin_rva: u32) -> Result<FrameKind, Bad> {
    if begin_rva != 0 {
        for s in 0..f.states()? {
            if f.unwind_entry(s as i32)?.action == begin_rva {
                return Ok(FrameKind::Cleanup);
            }
        }
    }
    // The innermost try block whose catch states hold `state`: nested
    // catch ranges lie inside the outer ones and start later.
    let mut best: Option<(u32, TryBlock)> = None;
    for i in 0..count(f.info.num_try_blocks)? {
        let tb = f.try_block(i)?;
        if tb.in_catch(state) && best.is_none_or(|(_, b)| tb.try_high > b.try_high) {
            best = Some((i, tb));
        }
    }
    let Some((try_index, tb)) = best else { return Ok(FrameKind::Function) };
    let mut handler = None;
    for j in 0..count(tb.num_catches)? {
        if f.handler(&tb, j)?.handler == begin_rva {
            handler = Some(j);
            break;
        }
    }
    Ok(FrameKind::Catch { try_index, handler })
}

/// The try blocks that apply to a frame of `kind` at `state`, in map
/// order (inner first).
pub fn candidates(f: &Func, kind: FrameKind, state: i32) -> Result<Vec<u32>, Bad> {
    let mut out = Vec::new();
    let outer = match kind {
        FrameKind::Cleanup => return Ok(out),
        FrameKind::Function => None,
        FrameKind::Catch { try_index, .. } => Some(f.try_block(try_index)?),
    };
    for i in 0..count(f.info.num_try_blocks)? {
        let tb = f.try_block(i)?;
        if !tb.covers(state) {
            continue;
        }
        if let Some(o) = outer
            && !(tb.try_low > o.try_high && tb.catch_high <= o.catch_high)
        {
            continue;
        }
        out.push(i);
    }
    Ok(out)
}

/// What the search does with a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Search {
    /// No clause here takes it.
    Continue,
    /// `terminate()` now (a `noexcept` frame no clause of which matches).
    Terminate,
    /// Clause `handler_index` of try block `try_index` takes it.
    Catch { try_index: u32, handler_index: u32, handler: HandlerType, matched: Match },
}

/// The search's decision for a frame of `kind` at `state`, for `exc`
/// (a rethrow record already completed, if it could be).
pub fn search(f: &Func, kind: FrameKind, state: i32, exc: &Exception) -> Result<Search, Bad> {
    if kind == FrameKind::Cleanup {
        return Ok(Search::Continue);
    }
    if !exc.is_cxx() {
        // A non-C++ exception: not seen at all by a synchronous-only frame;
        // otherwise only a catch(...) without the C++-only adjective.
        if f.info.synchronous_only() {
            return Ok(Search::Continue);
        }
        for i in candidates(f, kind, state)? {
            let tb = f.try_block(i)?;
            for j in 0..count(tb.num_catches)? {
                let h = f.handler(&tb, j)?;
                if h.type_descriptor == 0 && h.adjectives & ADJ_CXX_ONLY == 0 {
                    return Ok(Search::Catch { try_index: i, handler_index: j, handler: h, matched: Match::CatchAll });
                }
            }
        }
        return Ok(Search::Continue);
    }
    if exc.is_rethrow() {
        // A rethrow while nothing is being handled matches nothing.
        return Ok(Search::Continue);
    }
    let thrown = Thrown::read(f.mem, exc)?;
    for i in candidates(f, kind, state)? {
        let tb = f.try_block(i)?;
        for j in 0..count(tb.num_catches)? {
            let h = f.handler(&tb, j)?;
            if let Some(m) = match_clause(f.mem, f.image_base, &h, exc, &thrown)? {
                return Ok(Search::Catch { try_index: i, handler_index: j, handler: h, matched: m });
            }
        }
    }
    if f.info.is_noexcept() {
        return Ok(Search::Terminate);
    }
    Ok(Search::Continue)
}

/// The unwind actions (funclet rvas, in order) that take a frame of `kind`
/// from state `from` down to `to`: while the state is above `to`, its
/// action, then its `ToState`. A catch funclet's frame stops when the
/// state leaves its catch block's states; an unwind funclet's frame has
/// none.
pub fn unwind_actions(f: &Func, kind: FrameKind, from: i32, to: i32) -> Result<Vec<u32>, Bad> {
    let mut out = Vec::new();
    let (low, high) = match kind {
        FrameKind::Cleanup => return Ok(out),
        FrameKind::Function => (i32::MIN, i32::MAX),
        FrameKind::Catch { try_index, .. } => {
            let tb = f.try_block(try_index)?;
            (tb.try_high, tb.catch_high)
        }
    };
    let mut s = from;
    let mut steps = 0u32;
    while s > to && s > low && s <= high {
        let e = f.unwind_entry(s)?;
        if e.action != 0 {
            out.push(e.action);
        }
        s = e.to_state;
        steps += 1;
        if steps > f.states()? {
            return Err(Bad::Loop);
        }
    }
    Ok(out)
}

/// The state of the frame that holds try block `tb` while one of its catch
/// blocks runs: the state around the try statement (`ToState` of
/// `TryLow`), so that neither the try block nor anything inside it applies
/// to it any more.
pub fn state_during_catch(f: &Func, tb: &TryBlock) -> i32 {
    f.unwind_entry(tb.try_low).map(|e| e.to_state).unwrap_or(tb.try_low.saturating_sub(1))
}

/// The handler of a catch funclet's frame whose `ParentFrameOffset` finds
/// the function's frame: the clause the funclet belongs to, or the try
/// block's first.
pub fn parent_frame_offset(f: &Func, kind: FrameKind) -> Result<Option<u32>, Bad> {
    let FrameKind::Catch { try_index, handler } = kind else { return Ok(None) };
    let tb = f.try_block(try_index)?;
    Ok(Some(f.handler(&tb, handler.unwrap_or(0))?.parent_frame_offset))
}
