//! The visible tests of the BSD memory, ulock, psynch and bsdthread_create
//! entries (`specs/macos/bsd/*.tests`), transliterated from C and run against
//! the host kernel model; each one's output must equal the arm64 output
//! recorded on macOS.

mod common;

use common::*;
use poly_personality_xnu::sys::*;

const EINVAL: i64 = 22;
const ENOMEM: i64 = 12;
const EFAULT: i64 = 14;
const ENOENT: i64 = 2;
const ETIMEDOUT: i64 = 60;
const UL_COMPARE_AND_WAIT: u64 = 1;
const UL_UNFAIR_LOCK: u64 = 2;
const UL_COMPARE_AND_WAIT_SHARED: u64 = 3;
const ULF_WAKE_ALL: u64 = 0x100;
const ULF_NO_ERRNO: u64 = 0x0100_0000;
const MS_ASYNC: u64 = 1;
const MS_INVALIDATE: u64 = 2;
const MS_SYNC: u64 = 0x10;

fn dir(entry: &str) -> String {
    format!("bsd/{entry}.tests")
}

// ----- memory -----

#[test]
fn mprotect_change() {
    run(&dir("mprotect"), "mprotect_change", |p| {
        let m = p.anon(4 * PS, PROT_READ | PROT_WRITE);
        p.poke(m, 1);
        p.poke(m + PS, 2);
        let r = p.bsd(MPROTECT, &[m, PS, PROT_READ]);
        p.check("RW to RO returns 0", r.err == 0 && r.ret == 0, r.err);
        let v = p.peek(m);
        p.check("the page still reads back", v == 1, i64::from(v));
        let r = p.bsd(MPROTECT, &[m, PS, PROT_READ | PROT_WRITE]);
        p.check_ok("RO to RW again", r);
        p.poke(m, 5);
        let v = p.peek(m);
        p.check("and it can be written", v == 5, i64::from(v));
        let r = p.bsd(MPROTECT, &[m + PS, PS, 0]);
        p.check_ok("a page to PROT_NONE", r);
        let r = p.bsd(MPROTECT, &[m + PS, PS, PROT_READ | PROT_WRITE]);
        p.check_ok("PROT_NONE back to RW", r);
        let v = p.peek(m + PS);
        p.check("the content of the page survived", v == 2, i64::from(v));
        let r = p.bsd(MPROTECT, &[m + 2 * PS, 1, PROT_READ]);
        p.check_ok("a length of 1 covers the page", r);
        let r = p.bsd(MPROTECT, &[m, 4 * PS, PROT_READ | PROT_WRITE]);
        p.check_ok("a range of all four pages", r);
        let r = p.bsd(MPROTECT, &[m, 0, PROT_READ]);
        p.check_ok("a length of 0", r);
    });
}

#[test]
fn mprotect_errors() {
    run(&dir("mprotect"), "mprotect_errors", |p| {
        let m = p.anon(2 * PS, PROT_READ | PROT_WRITE);
        let r = p.bsd(MPROTECT, &[m + 1, PS, PROT_READ]);
        p.check_err("an unaligned address: EINVAL", r, EINVAL);
        let r = p.bsd(MPROTECT, &[m, PS.wrapping_neg(), PROT_READ]);
        p.check_err("a negative length: EINVAL", r, EINVAL);
        let r = p.bsd(MPROTECT, &[0x100000, PS, PROT_READ]);
        p.check_err("pages that are not mapped: ENOMEM", r, ENOMEM);
        let r = p.bsd(MPROTECT, &[m + PS, 3 * PS, PROT_READ]);
        p.check_err("a range that leaves the mapping: ENOMEM", r, ENOMEM);
        p.munmap(m, 2 * PS);
        let r = p.bsd(MPROTECT, &[m, PS, PROT_READ]);
        p.check_err("an unmapped mapping: ENOMEM", r, ENOMEM);
    });
}

#[test]
fn madvise_advice() {
    run(&dir("madvise"), "madvise_advice", |p| {
        let m = p.anon(2 * PS, PROT_READ | PROT_WRITE);
        p.poke(m, 9);
        p.poke(m + PS, 8);
        for advice in 0..=9u64 {
            let r = p.bsd(MADVISE, &[m, PS, advice]);
            p.check("advice accepted", r.err == 0 && r.ret == 0, advice as i64 * 1000 + r.err);
        }
        let (a, b) = (p.peek(m), p.peek(m + PS));
        p.check("and the memory is as it was", a == 9 && b == 8, i64::from(a));
        let r = p.bsd(MADVISE, &[m, PS, 12]);
        p.check_err("advice 12: EINVAL", r, EINVAL);
        let r = p.bsd(MADVISE, &[m, PS, u64::MAX]);
        p.check_err("advice -1: EINVAL", r, EINVAL);
        let r = p.bsd(MADVISE, &[m, PS, 10]);
        p.value("advice 10 (MADV_PAGEOUT): errno", r.err);
    });
}

#[test]
fn madvise_ranges() {
    run(&dir("madvise"), "madvise_ranges", |p| {
        let m = p.anon(2 * PS, PROT_READ | PROT_WRITE);
        let r = p.bsd(MADVISE, &[0x100000, PS, 0]);
        p.check_err("pages that are not mapped: ENOMEM", r, ENOMEM);
        let r = p.bsd(MADVISE, &[m, 0, 0]);
        p.check_ok("a length of 0", r);
        let r = p.bsd(MADVISE, &[m, 2 * PS, 0]);
        p.check_ok("the whole mapping", r);
        let r = p.bsd(MADVISE, &[m, 5 * PS, 0]);
        p.value("a range of 5 pages over a mapping of 2: errno", r.err);
        let r = p.bsd(MADVISE, &[m + 1, PS, 0]);
        p.value("an unaligned address: errno", r.err);
        p.munmap(m, 2 * PS);
        let r = p.bsd(MADVISE, &[m, PS, 0]);
        p.value("a hole where a mapping was unmapped: errno", r.err);
    });
}

#[test]
fn minherit_errors() {
    run(&dir("minherit"), "minherit_errors", |p| {
        let m = p.anon(PS, PROT_READ | PROT_WRITE);
        let r = p.bsd(MINHERIT, &[m, PS, 0]);
        p.check_ok("SHARE", r);
        let r = p.bsd(MINHERIT, &[m, PS, 1]);
        p.check_ok("COPY", r);
        let r = p.bsd(MINHERIT, &[m, PS, 2]);
        p.check_ok("NONE", r);
        let r = p.bsd(MINHERIT, &[m, PS, 3]);
        p.check_err("DONATE_COPY (3): EINVAL", r, EINVAL);
        let r = p.bsd(MINHERIT, &[m, PS, 4]);
        p.check_err("4: EINVAL", r, EINVAL);
        let r = p.bsd(MINHERIT, &[m, PS, 5]);
        p.check_err("5: EINVAL", r, EINVAL);
        let r = p.bsd(MINHERIT, &[m + 1, PS, 1]);
        p.value("an unaligned address: errno", r.err);
        let r = p.bsd(MINHERIT, &[0x100000, PS, 1]);
        p.value("an address that is not mapped: errno", r.err);
    });
}

#[test]
fn minherit_none() {
    run(&dir("minherit"), "minherit_none", |p| {
        let m = p.anon(PS, PROT_READ | PROT_WRITE);
        p.poke(m, 1);
        let r = p.bsd(MINHERIT, &[m, PS, 2]);
        p.check_ok("VM_INHERIT_NONE", r);
        let pid = p.fork(move |c| {
            c.peek(m);
            42
        });
        let st = p.wait(pid);
        p.check("the child dies when it touches the region", !w_exited(st), i64::from(st));
        p.value("signal", i64::from(w_termsig(st)));
        let v = p.peek(m);
        p.check("the parent still has it", v == 1, i64::from(v));
    });
}

#[test]
fn msync_flags() {
    run(&dir("msync"), "msync_flags", |p| {
        let m = p.shared_file(PS);
        let r = p.bsd(MSYNC, &[m, PS, MS_SYNC]);
        p.check_ok("MS_SYNC", r);
        let r = p.bsd(MSYNC, &[m, PS, MS_ASYNC]);
        p.check_ok("MS_ASYNC", r);
        let r = p.bsd(MSYNC, &[m, PS, 0]);
        p.check_ok("flags 0", r);
        let r = p.bsd(MSYNC, &[m, PS, MS_INVALIDATE]);
        p.check_ok("MS_INVALIDATE", r);
        let r = p.bsd(MSYNC, &[m, PS, MS_SYNC | MS_ASYNC]);
        p.check_err("MS_SYNC|MS_ASYNC: EINVAL", r, EINVAL);
        let a = p.anon(PS, PROT_READ | PROT_WRITE);
        let r = p.bsd(MSYNC, &[a, PS, MS_SYNC]);
        p.check_ok("anonymous memory", r);
        let r = p.bsd(MSYNC, &[m, PS, 0x100]);
        p.value("flag bit 0x100: errno", r.err);
    });
}

#[test]
fn msync_ranges() {
    run(&dir("msync"), "msync_ranges", |p| {
        let m = p.anon(2 * PS, PROT_READ | PROT_WRITE);
        let r = p.bsd(MSYNC, &[0x100000, PS, MS_SYNC]);
        p.check_err("pages that are not mapped: ENOMEM", r, ENOMEM);
        let r = p.bsd(MSYNC, &[m + 1, PS, MS_SYNC]);
        p.check_err("an unaligned address: EINVAL", r, EINVAL);
        let r = p.bsd(MSYNC, &[m, 0, MS_SYNC]);
        p.check_err("a length of 0: EINVAL", r, EINVAL);
        let r = p.bsd(MSYNC, &[m, 2 * PS, MS_ASYNC]);
        p.check_ok("the whole mapping", r);
    });
}

#[test]
fn mincore_anonymous() {
    run(&dir("mincore"), "mincore_anonymous", |p| {
        let m = p.anon(3 * PS, PROT_READ | PROT_WRITE);
        p.poke(m, 1);
        p.poke(m + PS, 1);
        let vec = p.var(8);
        p.poke64(vec, 0xeeee_eeee_eeee_eeee);
        let r = p.bsd(MINCORE, &[m, 3 * PS, vec]);
        p.check("returns 0", r.err == 0 && r.ret == 0, r.err);
        let v: Vec<u8> = (0..8).map(|i| p.peek(vec + i)).collect();
        p.check("touched pages are in core", v[0] & 1 != 0 && v[1] & 1 != 0, i64::from(v[0]));
        p.check(
            "all three are anonymous (0x80)",
            v[0] & 0x80 != 0 && v[1] & 0x80 != 0 && v[2] & 0x80 != 0,
            i64::from(v[2]),
        );
        p.check("exactly three bytes are written", v[3] == 0xee, i64::from(v[3]));
        p.value("vector byte of a touched page", i64::from(v[0]));
        p.value("vector byte of an untouched page", i64::from(v[2]));
        p.poke64(vec, 0xeeee_eeee_eeee_eeee);
        p.bsd(MINCORE, &[m, 1, vec]);
        let (a, b) = (p.peek(vec), p.peek(vec + 1));
        p.check("a length of 1 fills one byte", a != 0xee && b == 0xee, i64::from(b));
    });
}

#[test]
fn mincore_errors() {
    run(&dir("mincore"), "mincore_errors", |p| {
        let m = p.anon(PS, PROT_READ | PROT_WRITE);
        p.poke(m, 1);
        let vec = p.var(4);
        let r = p.bsd(MINCORE, &[m, PS, 8]);
        p.check_err("a vector at address 8: EFAULT", r, EFAULT);
        let r = p.bsd(MINCORE, &[m, 0, vec]);
        p.check_ok("a length of 0", r);
        let r = p.bsd(MINCORE, &[0x100000, PS, vec]);
        p.value("pages that are not mapped: errno", r.err);
        let r = p.bsd(MINCORE, &[m + 1, PS, vec]);
        p.value("an unaligned address: errno", r.err);
        let r = p.bsd(MINCORE, &[m, PS, vec]);
        p.check_ok("a good call afterwards", r);
    });
}

#[test]
fn mlock_fork() {
    run(&dir("mlock"), "mlock_fork", |p| {
        let m = p.anon(PS, PROT_READ | PROT_WRITE);
        p.poke(m, 5);
        let r = p.bsd(MLOCK, &[m, PS]);
        p.check_ok("mlock", r);
        let pid = p.fork(move |c| {
            let ok = c.peek(m) == 5 && c.bsd(MUNLOCK, &[m, PS]).err == 0;
            c.poke(m, 6);
            if ok { 0 } else { 1 }
        });
        let st = p.wait(pid);
        p.check("the child saw the content and could munlock", w_exited(st) && w_exitstatus(st) == 0, i64::from(st));
        let v = p.peek(m);
        p.check("its write did not reach the parent", v == 5, i64::from(v));
    });
}

#[test]
fn mlock_ranges() {
    run(&dir("mlock"), "mlock_ranges", |p| {
        let m = p.anon(PS, PROT_READ | PROT_WRITE);
        let r = p.bsd(MLOCK, &[m, 0]);
        p.check_ok("a length of 0", r);
        let r = p.bsd(MLOCK, &[m, 1]);
        p.check_ok("a length of 1", r);
        let r = p.bsd(MLOCK, &[m, 1 << 40]);
        p.check_err("a length of 1 << 40: ENOMEM", r, ENOMEM);
        let r = p.bsd(MLOCK, &[0x100000, PS]);
        p.value("pages that are not mapped: errno", r.err);
        let r = p.bsd(MLOCK, &[m + 1, PS]);
        p.value("an unaligned address: errno", r.err);
    });
}

// ----- ulock -----

/// `static volatile u32 word = 5;`
fn word(p: &mut P) -> u64 {
    let w = p.var(4);
    p.poke32(w, 5);
    w
}

/// A word a forked child and its parent both see, holding 5.
fn shared_word(p: &mut P) -> u64 {
    let w = p.shared_anon(PS);
    p.poke32(w, 5);
    w
}

/// The child touches the shared word's page, then waits on it (3 s) and
/// exits with the wait's errno, 0 if it was woken.
fn start_waiter(p: &mut P, w: u64, op: u64) -> u64 {
    p.fork(move |c| {
        c.peek32(w);
        let r = c.bsd(ULOCK_WAIT, &[op, w, 5, 3_000_000]);
        r.err as i32
    })
}

/// Wake until a waiter was there to wake: the result of that wake.
fn wake_when_waiting(p: &mut P, w: u64, op: u64) -> SysRet {
    let mut k = SysRet { ret: 0, err: ENOENT };
    for _ in 0..2000 {
        if k.err != ENOENT {
            break;
        }
        k = p.bsd(ULOCK_WAKE, &[op, w, 0]);
        if k.err == ENOENT {
            p.nap_ms(5);
        }
    }
    k
}

#[test]
fn ulock_wait_errors() {
    run(&dir("sys_ulock_wait"), "ulock_wait_errors", |p| {
        let w = word(p);
        let r = p.bsd(ULOCK_WAIT, &[0, w, 5, 1000]);
        p.check_err("operation 0: EINVAL", r, EINVAL);
        let r = p.bsd(ULOCK_WAIT, &[99, w, 5, 1000]);
        p.check_err("operation 99: EINVAL", r, EINVAL);
        let r = p.bsd(ULOCK_WAIT, &[UL_COMPARE_AND_WAIT, w + 1, 5, 1000]);
        p.check_err("an unaligned address: EINVAL", r, EINVAL);
        let r = p.bsd(ULOCK_WAIT, &[UL_COMPARE_AND_WAIT, 0, 5, 1000]);
        p.check_err("a NULL address: EINVAL", r, EINVAL);
        let r = p.bsd(ULOCK_WAIT, &[UL_COMPARE_AND_WAIT, 8, 5, 1000]);
        p.check_err("the address 8: EFAULT", r, EFAULT);
        let r = p.bsd(ULOCK_WAIT, &[UL_UNFAIR_LOCK, w, 5, 1000]);
        p.value("UL_UNFAIR_LOCK with an owner that is not a thread: errno", r.err);
        let r = p.bsd(ULOCK_WAIT, &[UL_COMPARE_AND_WAIT, w, 5, 1000]);
        p.check_err("a good wait afterwards", r, ETIMEDOUT);
    });
}

#[test]
fn ulock_wait_nonresident_page() {
    run(&dir("sys_ulock_wait"), "ulock_wait_nonresident_page", |p| {
        let w = shared_word(p);
        let pid = p.fork(move |c| {
            let first = c.bsd(ULOCK_WAIT, &[UL_COMPARE_AND_WAIT_SHARED, w, 5, 50000]);
            c.value("child: first wait, page not touched: errno", first.err);
            c.peek32(w);
            let second = c.bsd(ULOCK_WAIT, &[UL_COMPARE_AND_WAIT_SHARED, w, 5, 50000]);
            c.value("child: wait after reading the word: errno", second.err);
            i32::from(first.err == EFAULT) | if second.err == ETIMEDOUT { 2 } else { 0 }
        });
        let st = p.wait(pid);
        p.check(
            "the untouched page: EFAULT; after a read: ETIMEDOUT",
            w_exited(st) && w_exitstatus(st) == 3,
            i64::from(st),
        );
        let r = p.bsd(ULOCK_WAIT, &[UL_COMPARE_AND_WAIT_SHARED, w, 5, 50000]);
        p.check_err("the parent, which touched the page: ETIMEDOUT", r, ETIMEDOUT);
    });
}

#[test]
fn ulock_wait_woken() {
    run(&dir("sys_ulock_wait"), "ulock_wait_woken", |p| {
        let w = shared_word(p);
        let child = start_waiter(p, w, UL_COMPARE_AND_WAIT_SHARED);
        let k = wake_when_waiting(p, w, UL_COMPARE_AND_WAIT_SHARED);
        p.check("the wake found the waiter and returned 0", k.err == 0 && k.ret == 0, k.err);
        let st = p.wait(child);
        p.check("the waiter's wait ended without an error", w_exited(st) && w_exitstatus(st) == 0, i64::from(st));
        let r = p.bsd(ULOCK_WAKE, &[UL_COMPARE_AND_WAIT_SHARED, w, 0]);
        p.check_err("with nobody waiting: the next wake is ENOENT", r, ENOENT);
    });
}

#[test]
fn ulock_wake_all() {
    run(&dir("sys_ulock_wake"), "ulock_wake_all", |p| {
        let w = shared_word(p);
        let a = start_waiter(p, w, UL_COMPARE_AND_WAIT_SHARED);
        let b = start_waiter(p, w, UL_COMPARE_AND_WAIT_SHARED);
        let mut k = SysRet { ret: 0, err: ENOENT };
        for i in 0..400 {
            p.nap_ms(10);
            if i < 30 {
                continue;
            }
            k = p.bsd(ULOCK_WAKE, &[UL_COMPARE_AND_WAIT_SHARED | ULF_WAKE_ALL, w, 0]);
            if k.err == 0 {
                break;
            }
        }
        p.check("one wake call with ULF_WAKE_ALL returned 0", k.err == 0 && k.ret == 0, k.err);
        let (sa, sb) = (p.wait(a), p.wait(b));
        p.check("the first waiter's wait ended without an error", w_exited(sa) && w_exitstatus(sa) == 0, i64::from(sa));
        p.check("and the second's", w_exited(sb) && w_exitstatus(sb) == 0, i64::from(sb));
    });
}

#[test]
fn ulock_wake_nobody() {
    run(&dir("sys_ulock_wake"), "ulock_wake_nobody", |p| {
        let w = word(p);
        let r = p.bsd(ULOCK_WAKE, &[UL_COMPARE_AND_WAIT, w, 0]);
        p.check_err("nobody waiting: ENOENT", r, ENOENT);
        let r = p.bsd(ULOCK_WAKE, &[UL_COMPARE_AND_WAIT | ULF_WAKE_ALL, w, 0]);
        p.check_err("ULF_WAKE_ALL, nobody waiting: ENOENT", r, ENOENT);
        let r = p.bsd(ULOCK_WAKE, &[UL_COMPARE_AND_WAIT | ULF_NO_ERRNO, w, 0]);
        p.check("ULF_NO_ERRNO: -ENOENT as the result, no carry", r.err == 0 && r.ret == -ENOENT, r.ret);
        let r = p.bsd(ULOCK_WAKE, &[UL_COMPARE_AND_WAIT_SHARED, w, 0]);
        p.check_err("the shared operation too: ENOENT", r, ENOENT);
    });
}

#[test]
fn ulock_wait2_errors() {
    run(&dir("sys_ulock_wait2"), "ulock_wait2_errors", |p| {
        let w = word(p);
        let r = p.bsd(ULOCK_WAIT2, &[99, w, 5, 1000, 0, 0]);
        p.check_err("operation 99: EINVAL", r, EINVAL);
        let r = p.bsd(ULOCK_WAIT2, &[UL_COMPARE_AND_WAIT, 0, 5, 1000, 0, 0]);
        p.check_err("a NULL address: EINVAL", r, EINVAL);
        let r = p.bsd(ULOCK_WAIT2, &[UL_COMPARE_AND_WAIT, w + 1, 5, 1000, 0, 0]);
        p.check_err("an unaligned address: EINVAL", r, EINVAL);
        let r = p.bsd(ULOCK_WAIT2, &[UL_COMPARE_AND_WAIT, 8, 5, 1000, 0, 0]);
        p.check_err("the address 8: EFAULT", r, EFAULT);
    });
}

#[test]
fn ulock_wait2_timeout() {
    run(&dir("sys_ulock_wait2"), "ulock_wait2_timeout", |p| {
        let w = word(p);
        let r = p.bsd(ULOCK_WAIT2, &[UL_COMPARE_AND_WAIT, w, 5, 20_000_000, 0, 0]);
        p.check_err("the word equals the value: ETIMEDOUT", r, ETIMEDOUT);
        let r = p.bsd(ULOCK_WAIT2, &[UL_COMPARE_AND_WAIT, w, 6, 20_000_000, 0, 0]);
        p.check("the word differs: 0 at once", r.err == 0 && r.ret == 0, if r.err != 0 { r.err } else { r.ret });
        let r = p.bsd(ULOCK_WAIT2, &[UL_COMPARE_AND_WAIT | ULF_NO_ERRNO, w, 5, 20_000_000, 0, 0]);
        p.check("ULF_NO_ERRNO: -ETIMEDOUT, no carry", r.err == 0 && r.ret == -ETIMEDOUT, r.ret);
        let r = p.bsd(ULOCK_WAIT2, &[UL_COMPARE_AND_WAIT_SHARED, w, 5, 20_000_000, 0, 0]);
        p.check_err("the shared operation: ETIMEDOUT", r, ETIMEDOUT);
        let t0 = p.now();
        p.bsd(ULOCK_WAIT2, &[UL_COMPARE_AND_WAIT, w, 5, 20_000_000, 0, 0]);
        let t1 = p.now();
        p.check("the wait took at least 15 ms", t1 - t0 >= 24000 * 15, (t1 - t0) as i64);
    });
}

// ----- psynch -----

/// `static volatile u32 words[64];`
fn words(p: &mut P) -> u64 {
    p.var(256)
}
fn cvwait(p: &mut P, cv: u64, mutex: u64, sec: u64, nsec: u64) -> SysRet {
    p.bsd(PSYNCH_CVWAIT, &[cv, 0x100, 0, mutex, 0, 0, sec, nsec])
}
fn cvbroad(p: &mut P, cv: u64) -> SysRet {
    p.bsd(PSYNCH_CVBROAD, &[cv, 0x100, 0, 0, 0, 0, 0, 0])
}
fn cvsignal(p: &mut P, cv: u64) -> SysRet {
    p.bsd(PSYNCH_CVSIGNAL, &[cv, 0x100, 0, 0, 0, 0, 0, 0])
}
fn mxwait(p: &mut P, mutex: u64) -> SysRet {
    p.bsd(PSYNCH_MUTEXWAIT, &[mutex, 0, 0, 0, 0, 0, 0, 0])
}
fn mxdrop(p: &mut P, mutex: u64) -> SysRet {
    p.bsd(PSYNCH_MUTEXDROP, &[mutex, 0, 0, 0, 0, 0, 0, 0])
}

#[test]
fn cvbroad_no_waiters() {
    run(&dir("psynch_cvbroad"), "cvbroad_no_waiters", |p| {
        let w = words(p);
        let r = cvbroad(p, w);
        p.check("returns without an error", r.err == 0, r.err);
        let r = cvbroad(p, w);
        p.check_ok("again", r);
        let r = cvbroad(p, w + 64);
        p.check_ok("on another word", r);
    });
}

#[test]
fn cvbroad_with_waiter() {
    run(&dir("psynch_cvbroad"), "cvbroad_with_waiter", |p| {
        let w = words(p);
        let r = cvbroad(p, w);
        p.check_ok("broadcast on one word", r);
        let r = cvwait(p, w + 64, w + 32, 0, 10_000_000);
        p.check("a wait on another word still times out", r.err != 0 && (r.err & 0xff) == ETIMEDOUT, r.err);
    });
}

#[test]
fn cvsignal_no_waiters() {
    run(&dir("psynch_cvsignal"), "cvsignal_no_waiters", |p| {
        let w = words(p);
        let r = cvsignal(p, w);
        p.check_ok("returns without an error", r);
        let r = cvsignal(p, w);
        p.check_ok("again", r);
        let r = cvsignal(p, w + 64);
        p.check_ok("on another word", r);
    });
}

#[test]
fn cvsignal_then_wait() {
    run(&dir("psynch_cvsignal"), "cvsignal_then_wait", |p| {
        let w = words(p);
        let r = cvsignal(p, w);
        p.check_ok("signal", r);
        let t0 = p.now();
        let r = cvwait(p, w, w + 32, 0, 100_000_000);
        let t1 = p.now();
        p.check("the wait afterwards returns without an error", r.err == 0, r.err);
        p.check("and at once (well under the 100 ms timeout)", t1 - t0 < 24000 * 20, (t1 - t0) as i64);
        p.value("the wait's result", r.ret);
        let r = cvwait(p, w, w + 32, 0, 10_000_000);
        p.check("the next wait times out", r.err != 0 && (r.err & 0xff) == ETIMEDOUT, r.err);
    });
}

#[test]
fn cvwait_timeout() {
    run(&dir("psynch_cvwait"), "cvwait_timeout", |p| {
        let w = words(p);
        let r = cvwait(p, w, w + 32, 0, 20_000_000);
        p.check(
            "a wait that nobody signals fails with ETIMEDOUT in the low byte",
            r.err != 0 && (r.err & 0xff) == ETIMEDOUT,
            r.err,
        );
        p.value("the whole error", r.err);
        let r = cvwait(p, w, w + 32, 0, 20_000_000);
        p.check("again", r.err != 0 && (r.err & 0xff) == ETIMEDOUT, r.err);
    });
}

#[test]
fn cvwait_waits() {
    run(&dir("psynch_cvwait"), "cvwait_waits", |p| {
        let w = words(p);
        let t0 = p.now();
        cvwait(p, w, w + 32, 0, 20_000_000);
        let t1 = p.now();
        p.check("20 ms: at least 15 ms went by", t1 - t0 >= 24000 * 15, (t1 - t0) as i64);
        p.check("and not a second", t1 - t0 < 24_000_000, (t1 - t0) as i64);
        let t2 = p.now();
        cvwait(p, w, w + 32, 0, 1_000_000);
        let t3 = p.now();
        p.check("1 ms: less than 15 ms", t3 - t2 < 24000 * 15, (t3 - t2) as i64);
    });
}

#[test]
fn mutexdrop_forked() {
    run(&dir("psynch_mutexdrop"), "mutexdrop_forked", |p| {
        let w = words(p);
        let pid = p.fork(move |c| i32::from(mxdrop(c, w + 32).err != 0));
        let st = p.wait(pid);
        p.check("the child's drop succeeded", w_exited(st) && w_exitstatus(st) == 0, i64::from(st));
    });
}

#[test]
fn mutexdrop_null() {
    run(&dir("psynch_mutexdrop"), "mutexdrop_null", |p| {
        let w = words(p);
        let r = mxdrop(p, 0);
        p.value("a NULL mutex: errno", r.err);
        let r = mxdrop(p, w + 32);
        p.check_ok("a good one afterwards", r);
    });
}

fn mutexwait_blocks(p: &mut P, mutex: u64) {
    let pid = p.fork(move |c| {
        mxwait(c, mutex);
        0
    });
    p.nap_ms(150);
    let running = p.wait_nohang(pid).is_none();
    p.check("a wait that nobody ends has not returned after 150 ms", running, 0);
    p.kill(pid, 9);
    let st = p.wait(pid);
    p.check("it ends with the signal", st == 9, i64::from(st));
}

#[test]
fn mutexwait_blocks_test() {
    run(&dir("psynch_mutexwait"), "mutexwait_blocks", |p| {
        let w = words(p);
        mutexwait_blocks(p, w + 32);
    });
}

#[test]
fn mutexwait_null_blocks() {
    run(&dir("psynch_mutexwait"), "mutexwait_null_blocks", |p| mutexwait_blocks(p, 0));
}

// ----- bsdthread_create -----

#[test]
fn create_einval_no_stack() {
    run(&dir("bsdthread_create"), "create_einval_no_stack", |p| {
        let r = p.bsd(BSDTHREAD_CREATE, &[0x1000, 0, 0, 0, 0, 0]);
        p.check_err("a start function only: EINVAL", r, EINVAL);
        let r = p.bsd(BSDTHREAD_CREATE, &[0x1000, 7, 0, 0, 0, 0]);
        p.check_err("with an argument: EINVAL", r, EINVAL);
    });
}

#[test]
fn create_einval_zeros() {
    run(&dir("bsdthread_create"), "create_einval_zeros", |p| {
        let r = p.bsd(BSDTHREAD_CREATE, &[0; 6]);
        p.check_err("all arguments 0: EINVAL", r, EINVAL);
        let r = p.bsd(BSDTHREAD_CREATE, &[0; 6]);
        p.check_err("all arguments 0 again: EINVAL", r, EINVAL);
    });
}
