//! ulock, psynch and bsdthread_create (`specs/macos/bsd/sys_ulock_*.yaml`,
//! `psynch_*.yaml`, `bsdthread_create.yaml`), on the kernel's block and
//! unblock and the wait tables of `poly-mach`.

use crate::errno::*;
use crate::kernel::{ThreadStart, Wakeup, XnuKernel, unblock_all, with_mach};
use crate::wait::{after, block_for};
use poly_mach::psynch;
use poly_mach::ulock::{Key, Op, UL_UNFAIR_LOCK};

/// The key of the word at `addr` for operation `op`.
fn ulock_key(k: &dyn XnuKernel, op: Op, addr: u64) -> Result<Key, u32> {
    if op.shared() { k.shared_key(addr).map_err(|_| EFAULT) } else { Ok(Key::Private { task: k.task(), addr }) }
}

/// ulock_wait (timeout in microseconds) and ulock_wait2 (nanoseconds):
/// wait while the word at `addr` holds `value`. The word is read and the
/// waiter queued under the Mach lock, so a wake after a store is never
/// missed. With `ULF_NO_ERRNO` an error is the negative errno as the result.
pub fn ulock_wait(k: &mut dyn XnuKernel, op: u32, addr: u64, value: u64, timeout_ns: u64) -> Result<u64, u32> {
    let Some(o) = Op::decode(op) else { return Err(EINVAL) };
    let r = wait_word(k, o, addr, value, timeout_ns);
    match r {
        Err(e) if o.no_errno => Ok((-(i64::from(e))) as u64),
        r => r,
    }
}

fn wait_word(k: &mut dyn XnuKernel, o: Op, addr: u64, value: u64, timeout_ns: u64) -> Result<u64, u32> {
    let size = o.word_size();
    if addr == 0 || !addr.is_multiple_of(size) {
        return Err(EINVAL);
    }
    let key = ulock_key(k, o, addr)?;
    let deadline = (timeout_ns != 0).then(|| after(k, timeout_ns));
    let queued = with_mach(k, |m, k| {
        let mut b = [0u8; 8];
        let buf = &mut b[..size as usize];
        if o.shared() {
            // The kernel does not fault the page in for a shared wait.
            match k.load_resident(addr, buf) {
                Ok(true) => {}
                _ => return Err(EFAULT),
            }
        } else {
            k.copy_in(addr, buf).map_err(|_| EFAULT)?;
        }
        let word = u64::from_le_bytes(b);
        let expected = if size == 4 { value & 0xffff_ffff } else { value };
        if word != expected {
            return Ok(false);
        }
        // An unfair lock's word names its owner: a thread port.
        if o.kind == UL_UNFAIR_LOCK && !matches!(m.ipc.thread_of(k.task(), expected as u32), Ok(Some(_))) {
            return Err(EOWNERDEAD);
        }
        m.ulock.enqueue(key, k.thread());
        Ok(true)
    })?;
    if !queued {
        return Ok(0);
    }
    match block_for(
        k,
        deadline,
        |m, t| m.ulock.take(t),
        |m, t| {
            m.ulock.cancel(t);
        },
    ) {
        Ok(left) => Ok(u64::from(left)),
        Err(Wakeup::Signal) => Err(EINTR),
        Err(_) => Err(ETIMEDOUT),
    }
}

/// ulock_wake: wake one waiter of the word (all with `ULF_WAKE_ALL`);
/// `ENOENT` when nobody waits.
pub fn ulock_wake(k: &mut dyn XnuKernel, op: u32, addr: u64) -> Result<u64, u32> {
    let Some(o) = Op::decode(op) else { return Err(EINVAL) };
    let r = (|| {
        if addr == 0 {
            return Err(EINVAL);
        }
        let key = ulock_key(k, o, addr)?;
        let woken = with_mach(k, |m, _| m.ulock.wake(key, o.wake_all));
        if woken.is_empty() {
            return Err(ENOENT);
        }
        unblock_all(k, woken);
        Ok(0)
    })();
    match r {
        Err(e) if o.no_errno => Ok((-(i64::from(e))) as u64),
        r => r,
    }
}

fn psynch_key(k: &dyn XnuKernel, addr: u64) -> Key {
    Key::Private { task: k.task(), addr }
}

/// psynch_cvwait(cv, cvlsgen, cvugen, mutex, mugen, flags, sec, nsec):
/// consume a pre-posted signal or wait for one, at most the relative time
/// (none when both are 0). A timeout is `ETIMEDOUT` with the sequence state
/// above it.
pub fn cvwait(k: &mut dyn XnuKernel, a: &[u64; 8]) -> Result<u64, u32> {
    let (cv, cvlsgen, sec, nsec) = (a[0], a[1], a[6], a[7]);
    let key = psynch_key(k, cv);
    let timeout = sec.saturating_mul(1_000_000_000).saturating_add(nsec);
    let deadline = (timeout != 0).then(|| after(k, timeout));
    if let Some(r) = with_mach(k, |m, k| m.psynch.cvwait(key, cvlsgen, k.thread())) {
        return Ok(u64::from(r));
    }
    match block_for(
        k,
        deadline,
        |m, t| m.psynch.take(t),
        |m, t| {
            m.psynch.cancel(t);
        },
    ) {
        Ok(r) => Ok(u64::from(r)),
        Err(Wakeup::Signal) => Err(EINTR),
        Err(_) => Err(psynch::cv_error(cvlsgen, ETIMEDOUT)),
    }
}

/// psynch_cvsignal(cv, cvlsgen, cvugen, thread port, mutex, mugen, tid,
/// flags): wake one waiter (that thread, if it waits).
pub fn cvsignal(k: &mut dyn XnuKernel, a: &[u64; 8]) -> Result<u64, u32> {
    let key = psynch_key(k, a[0]);
    let woken = with_mach(k, |m, k| {
        let thread = m.ipc.thread_of(k.task(), a[3] as u32).ok().flatten();
        m.psynch.cvsignal(key, a[1], thread)
    });
    unblock_all(k, woken);
    Ok(0)
}

/// psynch_cvbroad(cv, cvlsgen, cvudgen, flags, mutex, mugen, tid): wake
/// every waiter of the word. No condition variable and no sequence at all
/// is `EINVAL`.
pub fn cvbroad(k: &mut dyn XnuKernel, a: &[u64; 8]) -> Result<u64, u32> {
    if a[0] == 0 && a[1] == 0 {
        return Err(EINVAL);
    }
    let key = psynch_key(k, a[0]);
    let woken = with_mach(k, |m, _| m.psynch.cvbroad(key, a[1]));
    unblock_all(k, woken);
    Ok(0)
}

/// psynch_mutexwait(mutex, mgen, ugen, tid, flags): wait until the mutex
/// is dropped to this thread. The word is not read.
pub fn mutexwait(k: &mut dyn XnuKernel, a: &[u64; 8]) -> Result<u64, u32> {
    let key = psynch_key(k, a[0]);
    with_mach(k, |m, k| m.psynch.mutexwait(key, k.thread()));
    match block_for(
        k,
        None,
        |m, t| m.psynch.take(t),
        |m, t| {
            m.psynch.cancel(t);
        },
    ) {
        Ok(r) => Ok(u64::from(r)),
        Err(_) => Err(EINTR),
    }
}

/// psynch_mutexdrop(mutex, mgen, ugen, tid, flags): hand the mutex to a
/// waiter; with nobody waiting it succeeds.
pub fn mutexdrop(k: &mut dyn XnuKernel, a: &[u64; 8]) -> Result<u64, u32> {
    let key = psynch_key(k, a[0]);
    let woken = with_mach(k, |m, _| m.psynch.mutexdrop(key, a[1] as u32));
    unblock_all(k, woken);
    Ok(0)
}

/// `PTHREAD_START_CUSTOM`: the caller made the stack and thread structure.
pub const PTHREAD_START_CUSTOM: u32 = 0x0100_0000;

/// bsdthread_create(func, arg, stack, pthread, flags): start a thread at
/// libpthread's registered entry point. Refused (`EINVAL`, the sheet): no
/// registered entry point, no start function, a caller-made stack without
/// its stack or thread structure, and every call without
/// `PTHREAD_START_CUSTOM`.
pub fn bsdthread_create(k: &mut dyn XnuKernel, a: &[u64; 8]) -> Result<u64, u32> {
    let s = ThreadStart { func: a[0], arg: a[1], stack: a[2], pthread: a[3], flags: a[4] as u32 };
    if !k.thread_entry_registered() || s.func == 0 || s.flags & PTHREAD_START_CUSTOM == 0 {
        return Err(EINVAL);
    }
    if s.stack == 0 || s.pthread == 0 {
        return Err(EINVAL);
    }
    k.create_thread(s)
}

/// bsdthread_register(thread_start, workqueue_start, flags, ...): the
/// first registration wins; a second one is `EINVAL` (the sheet).
pub fn bsdthread_register(k: &mut dyn XnuKernel, a: &[u64; 8]) -> Result<u64, u32> {
    if k.thread_entry_registered() {
        return Err(EINVAL);
    }
    k.register_thread_entry(a[0]).map(|()| 0)
}

/// bsdthread_terminate(stack, size, thread port, semaphore): unmap the
/// thread's stack (if a size is given) and end the calling thread.
pub fn bsdthread_terminate(k: &mut dyn XnuKernel, a: &[u64; 8]) -> Result<u64, u32> {
    let (stack, size) = (a[0], a[1]);
    if size != 0 {
        let page = k.page_size();
        let start = stack & !(page - 1);
        let end = stack.saturating_add(size).next_multiple_of(page);
        k.unmap(start, end);
    }
    k.end_thread();
    Ok(0)
}
