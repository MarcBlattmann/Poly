//! futex, set_robust_list, get_robust_list, restart_syscall and the
//! thread-exit duties against the fake kernel. Other waiting threads are
//! simulated by queueing them in the fake's futex table directly (they
//! would have called FUTEX_WAIT earlier); the calling thread waits through
//! the system call and is woken by "meanwhile" steps of other threads.

extern crate std;

use super::fake::*;
use crate::abi::futex::*;
use crate::abi::signal::*;
use crate::abi::{SigAction, Timespec};
use crate::errno::Errno;
use crate::futex::FutexKey;
use crate::kernel::{Clock, Deadline, SigSet};
use crate::signal::ExitToUser;
use crate::signal::restart::RestartBlock;
use crate::{Linux, nr};
use std::vec::Vec;

const WORD: u64 = DATA + 0x100;
const WORD2: u64 = DATA + 0x104;
const TS: u64 = DATA + 0x200;
const PRIVATE: u64 = FUTEX_PRIVATE_FLAG as u64;

fn key(addr: u64) -> FutexKey {
    FutexKey::Private { space: 100, addr }
}

fn op(cmd: u32) -> u64 {
    cmd as u64 | PRIVATE
}

fn timeout(k: &mut FakeKernel, sec: i64, nsec: i64) -> u64 {
    k.poke(TS, &Timespec { sec, nsec }.to_bytes());
    TS
}

/// Simulated waiters: thread `tid` blocked on `addr` with `bitset`.
fn waiting(k: &mut FakeKernel, addr: u64, tids: impl IntoIterator<Item = i32>, bitset: u32) {
    for tid in tids {
        assert!(k.futexes.enqueue(key(addr), tid, bitset));
    }
}

fn woken(k: &mut FakeKernel) -> Vec<i32> {
    let v = k.unblocked.iter().copied().collect();
    k.unblocked.clear();
    v
}

#[test]
fn argument_errors() {
    let mut k = FakeKernel::new();
    k.poke(WORD, &7u32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 8]), Err(Errno::EAGAIN), "value differs");
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD + 2, op(FUTEX_WAIT), 7]), Err(Errno::EINVAL), "unaligned");
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD + 1, op(FUTEX_WAKE), 1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FUTEX, &[UNMAPPED, op(FUTEX_WAIT), 7]), Err(Errno::EFAULT));
    let bad = timeout(&mut k, 0, 1_000_000_000);
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 7, bad]), Err(Errno::EINVAL));
    let bad = timeout(&mut k, -1, 0);
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 7, bad]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 7, UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT_BITSET), 7, 0, 0, 0]), Err(Errno::EINVAL), "bitset 0");
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAKE_BITSET), 1, 0, 0, 0]), Err(Errno::EINVAL));
    for cmd in [FUTEX_FD, FUTEX_LOCK_PI, FUTEX_UNLOCK_PI, FUTEX_TRYLOCK_PI, FUTEX_WAIT_REQUEUE_PI].into_iter().chain([
        FUTEX_CMP_REQUEUE_PI,
        FUTEX_LOCK_PI2,
        14,
        0x7f,
    ]) {
        assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(cmd), 0]), Err(Errno::ENOSYS), "op {cmd}");
    }
    let realtime = FUTEX_CLOCK_REALTIME as u64;
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAKE) | realtime, 1]), Err(Errno::ENOSYS));
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT) | realtime, 8]), Err(Errno::EAGAIN), "allowed");
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_REQUEUE), 1, (-1i64) as u64, WORD2]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_REQUEUE), 1, 1, WORD2 + 1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAKE_OP), 1, 1, WORD2, 0xf000_0000]), Err(Errno::ENOSYS));
    assert!(k.futexes.is_empty() && k.blocks == 0);
}

#[test]
fn wait_until_another_thread_wakes() {
    let mut k = FakeKernel::new();
    k.add_thread(101);
    k.poke(WORD, &1u32.to_le_bytes());
    k.meanwhile(|k| {
        k.as_thread(101, |k| {
            k.poke(WORD, &0u32.to_le_bytes());
            assert_eq!(sys(k, nr::FUTEX, &[WORD, op(FUTEX_WAKE), 1]), Ok(1));
        });
    });
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 1, 0]), Ok(0));
    assert_eq!(k.deadlines, [None]);
    assert!(k.futexes.is_empty());
    // A wake with nobody waiting.
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAKE), 5]), Ok(0));
}

#[test]
fn spurious_unblocks_do_not_end_a_wait() {
    let mut k = FakeKernel::new();
    k.add_thread(101);
    k.poke(WORD, &1u32.to_le_bytes());
    k.unblocked.insert(100); // left over from an earlier wait
    k.meanwhile(|k| {
        k.as_thread(101, |k| sys(k, nr::FUTEX, &[WORD, op(FUTEX_WAKE), 1]).unwrap());
    });
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 1, 0]), Ok(0));
    assert_eq!(k.blocks, 2, "blocked again after the stale unblock");
}

#[test]
fn timeouts() {
    let mut k = FakeKernel::new();
    k.monotonic = Timespec { sec: 42, nsec: 600_000_000 };
    let t = timeout(&mut k, 1, 500_000_000);
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 0, t]), Err(Errno::ETIMEDOUT));
    let mono = Deadline { clock: Clock::Monotonic, at: Timespec { sec: 44, nsec: 100_000_000 } };
    assert_eq!(k.deadlines.last(), Some(&Some(mono)), "FUTEX_WAIT: relative, on CLOCK_MONOTONIC");
    assert!(k.futexes.is_empty(), "the timed-out waiter left the queue");
    let realtime = FUTEX_CLOCK_REALTIME as u64;
    sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT) | realtime, 0, t]).unwrap_err();
    let rt = Deadline { clock: Clock::Realtime, at: Timespec { sec: 1_700_000_001, nsec: 500_000_123 } };
    assert_eq!(k.deadlines.last(), Some(&Some(rt)));
    let t = timeout(&mut k, 50, 7);
    let any = FUTEX_BITSET_MATCH_ANY as u64;
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT_BITSET), 0, t, 0, any]), Err(Errno::ETIMEDOUT));
    let abs = Deadline { clock: Clock::Monotonic, at: Timespec { sec: 50, nsec: 7 } };
    assert_eq!(k.deadlines.last(), Some(&Some(abs)), "FUTEX_WAIT_BITSET: absolute");
    sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT_BITSET) | realtime, 0, t, 0, any]).unwrap_err();
    assert_eq!(k.deadlines.last().unwrap().unwrap().clock, Clock::Realtime);
}

#[test]
fn bitsets_choose_waiters() {
    let mut k = FakeKernel::new();
    waiting(&mut k, WORD, [1, 2], 0b01);
    waiting(&mut k, WORD, [3], 0b10);
    waiting(&mut k, WORD, [4], u32::MAX);
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAKE_BITSET), 9, 0, 0, 0b10]), Ok(2));
    assert_eq!(woken(&mut k), [3, 4]);
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAKE), 1]), Ok(1));
    assert_eq!(woken(&mut k), [1], "first come, first served");
    // The calling thread waits with a bitset; a non-matching wake passes it by.
    k.add_thread(101);
    k.meanwhile(|k| {
        k.as_thread(101, |k| {
            assert_eq!(sys(k, nr::FUTEX, &[WORD, op(FUTEX_WAKE_BITSET), 9, 0, 0, 0b100]), Ok(0));
            assert_eq!(sys(k, nr::FUTEX, &[WORD, op(FUTEX_WAKE_BITSET), 9, 0, 0, 0b1000]), Ok(1));
        });
    });
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT_BITSET), 0, 0, 0, 0b1000]), Ok(0));
    assert!(woken(&mut k).is_empty(), "the calling thread consumed its own unblock");
    assert_eq!(k.futexes.waiters(key(WORD)).collect::<Vec<_>>(), [2], "bitset 0b01 was never matched");
}

#[test]
fn requeue_and_cmp_requeue() {
    let mut k = FakeKernel::new();
    waiting(&mut k, WORD, 1..=6, 1);
    waiting(&mut k, WORD2, [9], 1);
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_REQUEUE), 1, 2, WORD2]), Ok(3), "returns woken + moved");
    assert_eq!(woken(&mut k), [1]);
    assert_eq!(k.futexes.waiters(key(WORD)).collect::<Vec<_>>(), [4, 5, 6]);
    assert_eq!(k.futexes.waiters(key(WORD2)).collect::<Vec<_>>(), [9, 2, 3]);
    k.poke(WORD, &5u32.to_le_bytes());
    let cmp = |k: &mut FakeKernel, w: u64, r: u64, val3: u64| {
        sys(k, nr::FUTEX, &[WORD, op(FUTEX_CMP_REQUEUE), w, r, WORD2, val3])
    };
    assert_eq!(cmp(&mut k, 1, 9, 4), Err(Errno::EAGAIN), "the word changed");
    assert!(woken(&mut k).is_empty());
    assert_eq!(cmp(&mut k, 1, 9, 5), Ok(3), "woken plus requeued");
    assert_eq!(woken(&mut k), [4]);
    assert_eq!(k.futexes.waiters(key(WORD2)).collect::<Vec<_>>(), [9, 2, 3, 5, 6]);
    assert_eq!(cmp(&mut k, 0, 0, 5), Ok(0));
    assert_eq!(sys(&mut k, nr::FUTEX, &[UNMAPPED, op(FUTEX_CMP_REQUEUE), 1, 1, WORD2, 0]), Err(Errno::EFAULT));
}

#[test]
fn wake_op_changes_the_second_word_and_wakes_both() {
    let mut k = FakeKernel::new();
    waiting(&mut k, WORD, [1, 2], 1);
    waiting(&mut k, WORD2, [3, 4], 1);
    k.poke(WORD2, &1u32.to_le_bytes());
    // glibc's condition-variable style: *uaddr2 = 0; wake uaddr2 if old > 1.
    let set0_if_gt1 = (FUTEX_OP_SET << 28) | (FUTEX_OP_CMP_GT << 24) | 1;
    let wake_op = |k: &mut FakeKernel, n1: u64, n2: u64, val3: u32| {
        sys(k, nr::FUTEX, &[WORD, op(FUTEX_WAKE_OP), n1, n2, WORD2, val3 as u64])
    };
    assert_eq!(wake_op(&mut k, 1, 1, set0_if_gt1), Ok(1), "old 1 is not > 1: only uaddr");
    assert_eq!(k.peek_u32(WORD2), 0);
    assert_eq!(woken(&mut k), [1]);
    k.poke(WORD2, &2u32.to_le_bytes());
    assert_eq!(wake_op(&mut k, 1, 9, set0_if_gt1), Ok(3));
    assert_eq!(woken(&mut k), [2, 3, 4]);
    let add = (FUTEX_OP_ADD << 28) | (FUTEX_OP_CMP_EQ << 24) | (5 << 12);
    assert_eq!(wake_op(&mut k, 1, 1, add), Ok(0));
    assert_eq!(k.peek_u32(WORD2), 5);
    // uaddr2 must be writable.
    let ro = read_only_page(&mut k);
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAKE_OP), 1, 1, ro, add as u64]), Err(Errno::EFAULT));
}

#[test]
fn private_futexes_are_per_address_space() {
    let mut k = FakeKernel::new();
    k.futexes.enqueue(FutexKey::Private { space: 200, addr: WORD }, 7, u32::MAX);
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAKE), 9]), Ok(0));
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, FUTEX_WAKE as u64, 9]), Ok(0));
    // A private waiter is not found by a shared wake on the same word, nor
    // a shared waiter by a private one (futex's spec sheet).
    waiting(&mut k, WORD, [8], u32::MAX);
    k.futexes.enqueue(FutexKey::Unshared { space: 100, addr: WORD }, 9, u32::MAX);
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, FUTEX_WAKE as u64, 9]), Ok(1));
    assert_eq!(woken(&mut k), [9], "the shared waiter");
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, FUTEX_WAKE as u64, 9]), Ok(0), "the private waiter stays");
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAKE), 9]), Ok(1));
    assert_eq!(woken(&mut k), [8]);
}

#[test]
fn unmapped_words_are_efault_for_every_operation() {
    let mut k = FakeKernel::new();
    let none = 0x7fff_ffff;
    for flags in [0, PRIVATE] {
        let f = |cmd: u32| cmd as u64 | flags;
        assert_eq!(sys(&mut k, nr::FUTEX, &[UNMAPPED, f(FUTEX_WAKE), 1]), Err(Errno::EFAULT));
        assert_eq!(sys(&mut k, nr::FUTEX, &[UNMAPPED, f(FUTEX_WAKE_BITSET), 1, 0, 0, 1]), Err(Errno::EFAULT));
        assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, f(FUTEX_REQUEUE), 1, none, UNMAPPED]), Err(Errno::EFAULT));
        assert_eq!(sys(&mut k, nr::FUTEX, &[UNMAPPED, f(FUTEX_REQUEUE), 1, none, WORD]), Err(Errno::EFAULT));
        assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, f(FUTEX_CMP_REQUEUE), 1, none, UNMAPPED, 0]), Err(Errno::EFAULT));
        assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, f(FUTEX_REQUEUE), 1, none, WORD2]), Ok(0), "both mapped");
    }
}

/// A deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Many simulated waiters on four words, random wakes, bitset wakes,
/// requeues and wake-ops through the system call, against a plain model:
/// a list of (thread, word, bitset) in arrival order (requeued waiters move
/// to the end) and the word values.
#[test]
fn randomized_futex_model() {
    let words = [DATA + 0x300, DATA + 0x304, DATA + 0x308, DATA + 0x30c];
    for seed in 1..=12u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut k = FakeKernel::new();
        let mut model: Vec<(i32, usize, u32)> = Vec::new();
        let mut values = [0u32; 4];
        let mut next_tid = 1000;
        for step in 0..300 {
            let w = rng.below(4) as usize;
            match rng.below(6) {
                0 | 1 => {
                    for _ in 0..rng.below(4) {
                        let bitset = [u32::MAX, 1, 2, 6][rng.below(4) as usize];
                        k.futexes.enqueue(key(words[w]), next_tid, bitset);
                        model.push((next_tid, w, bitset));
                        next_tid += 1;
                    }
                }
                2 => {
                    let n = rng.below(4);
                    let bitset = [u32::MAX, 1, 2, 4][rng.below(4) as usize];
                    let r = sys(&mut k, nr::FUTEX, &[words[w], op(FUTEX_WAKE_BITSET), n, 0, 0, bitset as u64]);
                    let mut expect = Vec::new();
                    model.retain(|&(t, mw, b)| {
                        let take = expect.len() < n as usize && mw == w && b & bitset != 0;
                        if take {
                            expect.push(t);
                        }
                        !take
                    });
                    assert_eq!(r, Ok(expect.len() as u64), "seed {seed} step {step}");
                    let mut got = woken(&mut k);
                    got.sort();
                    expect.sort();
                    assert_eq!(got, expect, "seed {seed} step {step}");
                }
                3 | 4 => {
                    let to = rng.below(4) as usize;
                    let (nw, nr_) = (rng.below(3), rng.below(4));
                    let compare = rng.below(2) == 0;
                    let val3 = if rng.below(4) == 0 { values[w] ^ 1 } else { values[w] };
                    let cmd = if compare { FUTEX_CMP_REQUEUE } else { FUTEX_REQUEUE };
                    let r = sys(&mut k, nr::FUTEX, &[words[w], op(cmd), nw, nr_, words[to], val3 as u64]);
                    if compare && val3 != values[w] {
                        assert_eq!(r, Err(Errno::EAGAIN));
                        assert!(woken(&mut k).is_empty());
                        continue;
                    }
                    let mut expect = Vec::new();
                    model.retain(|&(t, mw, _)| {
                        let take = expect.len() < nw as usize && mw == w;
                        if take {
                            expect.push(t);
                        }
                        !take
                    });
                    let mut moved = Vec::new();
                    // Requeued onto itself, waiters keep their places and
                    // are only counted (futex_requeue's spec sheet).
                    let n_moved = if to == w {
                        (nr_ as usize).min(model.iter().filter(|e| e.1 == w).count())
                    } else {
                        model.retain(|&(t, mw, b)| {
                            let take = moved.len() < nr_ as usize && mw == w;
                            if take {
                                moved.push((t, to, b));
                            }
                            !take
                        });
                        moved.len()
                    };
                    let n = expect.len() + n_moved;
                    model.extend(moved);
                    assert_eq!(r, Ok(n as u64), "seed {seed} step {step}");
                    let mut got = woken(&mut k);
                    got.sort();
                    expect.sort();
                    assert_eq!(got, expect);
                }
                _ => {
                    let w2 = (w + 1 + rng.below(3) as usize) % 4;
                    let (n1, n2) = (rng.below(3), rng.below(3));
                    let (opv, oparg, cmp, cmparg) =
                        (rng.below(5) as u32, rng.below(8) as u32, rng.below(6) as u32, 2u32);
                    let val3 = (opv << 28) | (cmp << 24) | (oparg << 12) | cmparg;
                    let r = sys(&mut k, nr::FUTEX, &[words[w], op(FUTEX_WAKE_OP), n1, n2, words[w2], val3 as u64]);
                    let old = values[w2];
                    values[w2] = match opv {
                        0 => oparg,
                        1 => old.wrapping_add(oparg),
                        2 => old | oparg,
                        3 => old & !oparg,
                        _ => old ^ oparg,
                    };
                    let hit = match cmp {
                        0 => old as i32 == 2,
                        1 => old as i32 != 2,
                        2 => (old as i32) < 2,
                        3 => old as i32 <= 2,
                        4 => old as i32 > 2,
                        _ => old as i32 >= 2,
                    };
                    let mut expect = Vec::new();
                    for (word, n) in [(w, n1), (w2, if hit { n2 } else { 0 })] {
                        let mut taken = 0;
                        model.retain(|&(t, mw, _)| {
                            let take = taken < n && mw == word;
                            if take {
                                expect.push(t);
                                taken += 1;
                            }
                            !take
                        });
                    }
                    assert_eq!(r, Ok(expect.len() as u64), "seed {seed} step {step}");
                    assert_eq!(k.peek_u32(words[w2]), values[w2]);
                    let mut got = woken(&mut k);
                    got.sort();
                    expect.sort();
                    assert_eq!(got, expect);
                }
            }
            for (i, &word) in words.iter().enumerate() {
                let m: Vec<i32> = model.iter().filter(|e| e.1 == i).map(|e| e.0).collect();
                assert_eq!(k.futexes.waiters(key(word)).collect::<Vec<_>>(), m, "seed {seed} step {step}");
            }
        }
    }
}

fn install(k: &mut FakeKernel, sig: u32, flags: u64) {
    let act = SigAction { handler: 0x40_5000, flags: flags | SA_RESTORER, restorer: 0x40_6000, mask: 0 };
    k.poke(DATA + 0x800, &act.to_bytes());
    sys(k, nr::RT_SIGACTION, &[sig as u64, DATA + 0x800, 0, 8]).unwrap();
}

/// The saved registers in the frame of the handler now running.
fn saved_rip_rax(k: &FakeKernel) -> (u64, u64) {
    let uc = k.ctx.rdx + crate::signal::frame::UC_MCONTEXT as u64;
    (k.peek_u64(uc + 128), k.peek_u64(uc + 104))
}

fn interrupt_with(k: &mut FakeKernel, sig: u32) {
    k.add_thread(101);
    k.meanwhile(move |k| {
        k.as_thread(101, |k| sys(k, nr::TGKILL, &[100, 100, sig as u64]).unwrap());
    });
}

#[test]
fn interrupted_waits_restart_with_sa_restart() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, SA_RESTART);
    interrupt_with(&mut k, SIGUSR1);
    let (exit, ctx) = run(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 0, 0]);
    assert_eq!(exit, ExitToUser::Return);
    assert_eq!(ctx.rip, 0x40_5000, "in the handler");
    assert_eq!(saved_rip_rax(&k), (SYSCALL_RIP - 2, nr::FUTEX), "returns to the syscall instruction");
    assert!(k.futexes.is_empty());
}

#[test]
fn interrupted_waits_fail_with_eintr_without_sa_restart() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, 0);
    interrupt_with(&mut k, SIGUSR1);
    run(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 0, 0]);
    assert_eq!(saved_rip_rax(&k), (SYSCALL_RIP, Errno::EINTR.to_rax()));
}

#[test]
fn interrupted_waits_continue_after_a_stop() {
    let mut k = FakeKernel::new();
    interrupt_with(&mut k, SIGTSTP);
    let (_, ctx) = run(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 0, 0]);
    assert_eq!(k.stops, [SIGTSTP]);
    assert_eq!((ctx.rip, ctx.rax), (SYSCALL_RIP - 2, nr::FUTEX), "no handler: called again");
}

#[test]
fn a_timed_wait_continues_through_restart_syscall_after_a_stop() {
    let mut k = FakeKernel::new();
    interrupt_with(&mut k, SIGTSTP);
    let t = timeout(&mut k, 5, 0);
    let (_, ctx) = run(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 0, t]);
    assert_eq!(k.stops, [SIGTSTP]);
    assert_eq!((ctx.rip, ctx.rax), (SYSCALL_RIP - 2, nr::RESTART_SYSCALL));
    let deadline = Deadline { clock: Clock::Monotonic, at: Timespec { sec: 47, nsec: 999_999_999 } };
    let expected = RestartBlock::FutexWait { addr: WORD, private: true, val: 0, bitset: u32::MAX, deadline };
    assert_eq!(k.thread_state().restart_block, Some(expected));
    // The thread executes restart_syscall at the rewound rip; it waits on
    // until the same deadline, and another thread wakes it.
    k.meanwhile(|k| {
        k.as_thread(101, |k| sys(k, nr::FUTEX, &[WORD, op(FUTEX_WAKE), 1]).unwrap());
    });
    assert_eq!(run(&mut k, nr::RESTART_SYSCALL, &[]).1.rax, 0);
    assert_eq!(k.deadlines.last(), Some(&Some(deadline)), "the time left, not a new timeout");
    assert_eq!(k.thread_state().restart_block, None);
    assert_eq!(sys(&mut k, nr::RESTART_SYSCALL, &[]), Err(Errno::EINTR), "nothing to restart");
    // Interrupted again, it saves the same deadline again.
    k.me().thread_mut(100).restart_block = Some(expected);
    k.meanwhile(|k| {
        k.as_thread(101, |k| sys(k, nr::TGKILL, &[100, 100, SIGTSTP as u64]).unwrap());
    });
    let (_, ctx) = run(&mut k, nr::RESTART_SYSCALL, &[]);
    assert_eq!(ctx.rax, nr::RESTART_SYSCALL);
    assert_eq!(k.thread_state().restart_block, Some(expected));
}

#[test]
fn a_timed_wait_fails_with_eintr_after_a_handler() {
    // Even with SA_RESTART: a restart would start the timeout over.
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, SA_RESTART);
    interrupt_with(&mut k, SIGUSR1);
    let t = timeout(&mut k, 5, 0);
    run(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 0, t]);
    assert_eq!(saved_rip_rax(&k), (SYSCALL_RIP, Errno::EINTR.to_rax()));
    // The block stays while the handler runs (restart_syscall(2) there
    // resumes the wait); rt_sigreturn(2) drops it.
    assert!(k.thread_state().restart_block.is_some());
    // An absolute timeout (FUTEX_WAIT_BITSET) is simply restarted.
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, SA_RESTART);
    interrupt_with(&mut k, SIGUSR1);
    let t = timeout(&mut k, 99, 0);
    run(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT_BITSET), 0, t, 0, u32::MAX as u64]);
    assert_eq!(saved_rip_rax(&k), (SYSCALL_RIP - 2, nr::FUTEX));
}

#[test]
fn a_wake_racing_a_signal_wins() {
    let mut k = FakeKernel::new();
    k.add_thread(101);
    install(&mut k, SIGUSR1, 0);
    k.meanwhile(|k| {
        k.as_thread(101, |k| {
            sys(k, nr::FUTEX, &[WORD, op(FUTEX_WAKE), 1]).unwrap();
            sys(k, nr::TGKILL, &[100, 100, SIGUSR1 as u64]).unwrap();
        });
    });
    run(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 0, 0]);
    assert_eq!(saved_rip_rax(&k), (SYSCALL_RIP, 0), "FUTEX_WAIT returned 0, then the handler ran");
}

// ---- robust lists -----------------------------------------------------

const HEAD: u64 = DATA + 0x2000;
const OFFSET: u64 = 16;

fn entry(i: u64) -> u64 {
    DATA + 0x2100 + 0x100 * i
}

/// Build `head -> entries... -> head` with the lock words set to `words`
/// and `pending` as list_op_pending.
fn robust_list(k: &mut FakeKernel, words: &[u32], pending: u64) {
    let mut next = HEAD;
    for (i, &w) in words.iter().enumerate().rev() {
        let e = entry(i as u64);
        k.poke(e, &next.to_le_bytes());
        k.poke(e + OFFSET, &w.to_le_bytes());
        next = e;
    }
    k.poke(HEAD, &next.to_le_bytes());
    k.poke(HEAD + 8, &OFFSET.to_le_bytes());
    k.poke(HEAD + 16, &pending.to_le_bytes());
    assert_eq!(sys(k, nr::SET_ROBUST_LIST, &[HEAD, 24]), Ok(0));
}

#[test]
fn robust_list_walk_marks_owned_locks() {
    let mut k = FakeKernel::new();
    let me = 100u32;
    robust_list(&mut k, &[me | FUTEX_WAITERS, 555, me], entry(5));
    k.poke(entry(5) + OFFSET, &me.to_le_bytes()); // pending, not linked yet
    waiting(&mut k, entry(0) + OFFSET, [7, 8], u32::MAX);
    let walk = crate::futex::thread_exit(&mut k, true);
    assert_eq!((walk.visited, walk.marked, walk.woken), (3, 3, 1));
    assert_eq!(k.peek_u32(entry(0) + OFFSET), FUTEX_WAITERS | FUTEX_OWNER_DIED, "TID cleared, waiters kept");
    assert_eq!(k.peek_u32(entry(1) + OFFSET), 555, "another thread's lock");
    assert_eq!(k.peek_u32(entry(2) + OFFSET), FUTEX_OWNER_DIED);
    assert_eq!(k.peek_u32(entry(5) + OFFSET), FUTEX_OWNER_DIED, "the pending lock too");
    assert_eq!(woken(&mut k), [7], "one waiter woken");
    assert!(k.thread_state().robust_list.is_null(), "done once");
    assert_eq!(crate::futex::thread_exit(&mut k, true), Default::default());
}

#[test]
fn robust_list_walk_is_bounded_and_survives_corruption() {
    // A loop that never returns to the head: at most ROBUST_LIST_LIMIT entries.
    let mut k = FakeKernel::new();
    robust_list(&mut k, &[100], 0);
    k.poke(entry(0), &entry(0).to_le_bytes());
    let walk = crate::futex::thread_exit(&mut k, false);
    assert_eq!(walk.visited, ROBUST_LIST_LIMIT);
    assert_eq!(walk.marked, 1, "the second visit finds it no longer owned");
    // A link into unmapped memory ends the walk; the pending entry still counts.
    let mut k = FakeKernel::new();
    robust_list(&mut k, &[100, 100], entry(5));
    k.poke(entry(5) + OFFSET, &100u32.to_le_bytes());
    k.poke(entry(0), &UNMAPPED.to_le_bytes());
    let walk = crate::futex::thread_exit(&mut k, false);
    assert_eq!((walk.visited, walk.marked), (1, 2));
    assert_eq!(k.peek_u32(entry(1) + OFFSET), 100, "never reached");
    // An unreadable head: nothing.
    let mut k = FakeKernel::new();
    sys(&mut k, nr::SET_ROBUST_LIST, &[UNMAPPED, 24]).unwrap();
    assert_eq!(crate::futex::thread_exit(&mut k, false), Default::default());
    // Tagged (bit 0) links are followed; unaligned lock words are skipped.
    let mut k = FakeKernel::new();
    robust_list(&mut k, &[100, 100], 0);
    k.poke(HEAD, &(entry(0) | 1).to_le_bytes());
    k.poke(HEAD + 8, &(OFFSET + 2).to_le_bytes());
    let walk = crate::futex::thread_exit(&mut k, false);
    assert_eq!((walk.visited, walk.marked), (2, 0));
    // Through the dispatcher's exit path, as the kernel calls it.
    let mut k = FakeKernel::new();
    robust_list(&mut k, &[100], 0);
    Linux::thread_exit(&mut k, true);
    assert_eq!(k.peek_u32(entry(0) + OFFSET), FUTEX_OWNER_DIED);
}

#[test]
fn set_and_get_robust_list() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::SET_ROBUST_LIST, &[HEAD, 16]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SET_ROBUST_LIST, &[HEAD, 32]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SET_ROBUST_LIST, &[HEAD, 24]), Ok(0));
    let (h, l) = (DATA + 0x400, DATA + 0x408);
    assert_eq!(sys(&mut k, nr::GET_ROBUST_LIST, &[0, h, l]), Ok(0));
    assert_eq!((k.peek_u64(h), k.peek_u64(l)), (HEAD, 24));
    // Another thread of the process.
    k.add_thread(101);
    k.as_thread(101, |k| sys(k, nr::SET_ROBUST_LIST, &[HEAD + 0x40, 24]).unwrap());
    assert_eq!(sys(&mut k, nr::GET_ROBUST_LIST, &[101, h, l]), Ok(0));
    assert_eq!(k.peek_u64(h), HEAD + 0x40);
    // Another process: only with matching ids, or as root.
    k.add_process(300, 1000, 300, 300);
    assert_eq!(sys(&mut k, nr::GET_ROBUST_LIST, &[300, h, l]), Err(Errno::EPERM), "uid 1000 but gid differs");
    k.creds_mut().gid = 1000;
    assert_eq!(sys(&mut k, nr::GET_ROBUST_LIST, &[300, h, l]), Ok(0));
    assert_eq!(k.peek_u64(h), 0);
    k.add_process(400, 4000, 400, 400);
    assert_eq!(sys(&mut k, nr::GET_ROBUST_LIST, &[400, h, l]), Err(Errno::EPERM));
    k.creds_mut().caps.effective = crate::cred::CapSet::only(crate::cred::cap::SYS_PTRACE);
    assert_eq!(sys(&mut k, nr::GET_ROBUST_LIST, &[400, h, l]), Ok(0));
    assert_eq!(sys(&mut k, nr::GET_ROBUST_LIST, &[999, h, l]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::GET_ROBUST_LIST, &[u32::MAX as u64, h, l]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::GET_ROBUST_LIST, &[0, UNMAPPED, l]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::GET_ROBUST_LIST, &[0, h, UNMAPPED]), Err(Errno::EFAULT));
}

#[test]
fn clear_child_tid_at_exit() {
    let ctid = DATA + 0x500;
    let mut k = FakeKernel::new();
    k.poke(ctid, &100u32.to_le_bytes());
    sys(&mut k, nr::SET_TID_ADDRESS, &[ctid]).unwrap();
    waiting(&mut k, ctid, [7, 8], u32::MAX);
    Linux::thread_exit(&mut k, true);
    assert_eq!(k.peek_u32(ctid), 0, "cleared");
    assert_eq!(woken(&mut k), [7], "one waiter woken (pthread_join)");
    // Not sharing memory: nothing is written.
    let mut k = FakeKernel::new();
    k.poke(ctid, &100u32.to_le_bytes());
    sys(&mut k, nr::SET_TID_ADDRESS, &[ctid]).unwrap();
    Linux::thread_exit(&mut k, false);
    assert_eq!(k.peek_u32(ctid), 100);
    // A bad address is ignored.
    let mut k = FakeKernel::new();
    sys(&mut k, nr::SET_TID_ADDRESS, &[UNMAPPED]).unwrap();
    Linux::thread_exit(&mut k, true);
    assert!(k.futexes.is_empty());
}

#[test]
fn blocked_signals_do_not_interrupt_a_wait() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, 0);
    k.me().thread_mut(100).mask = SigSet::only(SIGUSR1);
    k.add_thread(101);
    k.me().thread_mut(101).mask = SigSet::only(SIGUSR1);
    k.meanwhile(|k| {
        k.as_thread(101, |k| {
            sys(k, nr::KILL, &[100, SIGUSR1 as u64]).unwrap();
        });
    });
    k.meanwhile(|k| {
        k.as_thread(101, |k| sys(k, nr::FUTEX, &[WORD, op(FUTEX_WAKE), 1]).unwrap());
    });
    assert_eq!(sys(&mut k, nr::FUTEX, &[WORD, op(FUTEX_WAIT), 0, 0]), Ok(0));
    assert!(k.me().pending(100).contains(SIGUSR1));
}

// ---- futex2: futex_wait, futex_wake, futex_waitv, futex_requeue ----------

const F2_PRIVATE: u64 = (FUTEX2_SIZE_U32 | FUTEX2_PRIVATE) as u64;
const F2_SHARED: u64 = FUTEX2_SIZE_U32 as u64;
const ANY: u64 = 0xffff_ffff;
const WAITV: u64 = DATA + 0x400;

/// Write `struct futex_waitv` entries (val, uaddr, flags) at `WAITV`.
fn waitv(k: &mut FakeKernel, entries: &[(u64, u64, u32)]) -> u64 {
    for (i, &(val, uaddr, flags)) in entries.iter().enumerate() {
        let mut b = [0u8; FUTEX_WAITV_SIZE];
        b[0..8].copy_from_slice(&val.to_le_bytes());
        b[8..16].copy_from_slice(&uaddr.to_le_bytes());
        b[16..20].copy_from_slice(&flags.to_le_bytes());
        k.poke(WAITV + (i * FUTEX_WAITV_SIZE) as u64, &b);
    }
    WAITV
}

#[test]
fn futex2_wait_argument_order() {
    let mut k = FakeKernel::new();
    k.poke(WORD, &7u32.to_le_bytes());
    let w = nr::FUTEX_WAIT;
    assert_eq!(sys(&mut k, w, &[WORD, 7, ANY, 0x1, 0, 0]), Err(Errno::EINVAL), "size u16");
    assert_eq!(sys(&mut k, w, &[WORD, 7, ANY, F2_PRIVATE | 0x100, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, w, &[WORD, 8, ANY, F2_PRIVATE | (1 << 40), 0, 0]), Err(Errno::EAGAIN), "high bits ignored");
    let t = timeout(&mut k, 0, 0);
    assert_eq!(sys(&mut k, w, &[WORD, 7, ANY, F2_PRIVATE, t, 7]), Err(Errno::EINVAL), "CLOCK_BOOTTIME");
    assert_eq!(sys(&mut k, w, &[WORD, 8, ANY, F2_PRIVATE, 0, 7]), Err(Errno::EAGAIN), "no timeout: clock ignored");
    assert_eq!(sys(&mut k, w, &[WORD, 1 << 32, ANY, F2_PRIVATE, UNMAPPED, 1]), Err(Errno::EINVAL), "val first");
    assert_eq!(sys(&mut k, w, &[WORD, 7, 0, F2_PRIVATE, UNMAPPED, 1]), Err(Errno::EFAULT), "timeout before mask");
    assert_eq!(sys(&mut k, w, &[WORD, 7, 0, F2_PRIVATE, 0, 0]), Err(Errno::EINVAL), "mask 0");
    assert_eq!(sys(&mut k, w, &[WORD, 7, 1 << 32, F2_PRIVATE, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, w, &[WORD + 2, 7, ANY, F2_PRIVATE, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, w, &[UNMAPPED, 7, ANY, F2_PRIVATE, 0, 0]), Err(Errno::EFAULT));
    let past = timeout(&mut k, 0, 30_000_000);
    assert_eq!(sys(&mut k, w, &[WORD, 8, ANY, F2_PRIVATE, past, 1]), Err(Errno::EAGAIN), "value before deadline");
    assert_eq!(sys(&mut k, w, &[WORD, 7, ANY, F2_PRIVATE, past, 1]), Err(Errno::ETIMEDOUT));
    let abs = Deadline { clock: Clock::Monotonic, at: Timespec { sec: 0, nsec: 30_000_000 } };
    assert_eq!(k.deadlines.last(), Some(&Some(abs)), "absolute");
    assert!(k.futexes.is_empty());
}

#[test]
fn futex2_wake_counts_masks_and_kinds() {
    let mut k = FakeKernel::new();
    waiting(&mut k, WORD, [1, 2, 3], 0b01);
    waiting(&mut k, WORD, [4], 0b10);
    let wake = nr::FUTEX_WAKE;
    assert_eq!(sys(&mut k, wake, &[WORD, 0, 1, F2_PRIVATE]), Err(Errno::EINVAL), "mask 0");
    assert_eq!(sys(&mut k, wake, &[0, ANY, 1, 0x3]), Err(Errno::EINVAL), "flags before address");
    assert_eq!(sys(&mut k, wake, &[WORD, 0b10, 5, F2_PRIVATE]), Ok(1));
    assert_eq!(sys(&mut k, wake, &[WORD, ANY, 0, F2_PRIVATE]), Ok(0));
    assert_eq!(sys(&mut k, wake, &[WORD, ANY, (-1i64) as u64, F2_PRIVATE]), Ok(1), "negative wakes one");
    assert_eq!(woken(&mut k), [1, 4]);
    assert_eq!(sys(&mut k, wake, &[WORD, ANY, 9, F2_SHARED]), Ok(0), "a shared wake misses private waiters");
    assert_eq!(sys(&mut k, wake, &[0, ANY, 1, F2_PRIVATE]), Ok(0), "private: not looked up");
    assert_eq!(sys(&mut k, wake, &[0, ANY, 1, F2_SHARED]), Err(Errno::EFAULT));
}

#[test]
fn futex2_numa_node_word() {
    let mut k = FakeKernel::new();
    let numa = F2_PRIVATE | FUTEX2_NUMA as u64;
    k.poke(WORD + 4, &u32::MAX.to_le_bytes());
    assert_eq!(sys(&mut k, nr::FUTEX_WAKE, &[WORD + 4, ANY, 1, numa]), Err(Errno::EINVAL), "pairs are 8-aligned");
    assert_eq!(sys(&mut k, nr::FUTEX_WAKE, &[WORD, ANY, 1, numa]), Ok(0));
    assert_eq!(k.peek_u32(WORD + 4), 0, "-1 became node 0");
    k.poke(WORD + 4, &4096u32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::FUTEX_WAKE, &[WORD, ANY, 1, numa]), Err(Errno::EINVAL));
}

#[test]
fn futex2_waitv_returns_the_woken_index() {
    let mut k = FakeKernel::new();
    k.add_thread(101);
    k.poke(WORD, &1u32.to_le_bytes());
    k.poke(WORD2, &2u32.to_le_bytes());
    let array = waitv(&mut k, &[(1, WORD, 0x82), (2, WORD2, 0x82)]);
    k.meanwhile(|k| {
        k.as_thread(101, |k| assert_eq!(sys(k, nr::FUTEX, &[WORD2, op(FUTEX_WAKE), 1]), Ok(1)));
    });
    assert_eq!(sys(&mut k, nr::FUTEX_WAITV, &[array, 2, 0, 0, 0]), Ok(1));
    assert!(k.futexes.is_empty(), "off every futex");
    assert_eq!(sys(&mut k, nr::FUTEX_WAITV, &[array, 0, 0, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FUTEX_WAITV, &[array, 129, 0, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FUTEX_WAITV, &[array, 2, 1, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FUTEX_WAITV, &[0, 2, 0, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FUTEX_WAITV, &[UNMAPPED, 2, 0, 0, 0]), Err(Errno::EFAULT));
    // A bad entry later beats a fault earlier; a fault later beats a
    // value mismatch earlier.
    let array = waitv(&mut k, &[(1, UNMAPPED, 0x82), (2, WORD2 + 1, 0x82)]);
    assert_eq!(sys(&mut k, nr::FUTEX_WAITV, &[array, 2, 0, 0, 0]), Err(Errno::EINVAL));
    let array = waitv(&mut k, &[(9, WORD, 0x82), (2, UNMAPPED, 0x82)]);
    assert_eq!(sys(&mut k, nr::FUTEX_WAITV, &[array, 2, 0, 0, 0]), Err(Errno::EFAULT));
    let array = waitv(&mut k, &[(1, WORD, 0x82), (3, WORD2, 0x82)]);
    assert_eq!(sys(&mut k, nr::FUTEX_WAITV, &[array, 2, 0, 0, 0]), Err(Errno::EAGAIN));
}

#[test]
fn futex2_requeue_moves_and_checks() {
    let mut k = FakeKernel::new();
    k.poke(WORD, &5u32.to_le_bytes());
    waiting(&mut k, WORD, [1, 2, 3, 4], u32::MAX);
    let req = nr::FUTEX_REQUEUE;
    let array = waitv(&mut k, &[(5, WORD, 0x82), (0, WORD2, 0x82)]);
    assert_eq!(sys(&mut k, req, &[array, 1, 1, 1]), Err(Errno::EINVAL), "flags");
    assert_eq!(sys(&mut k, req, &[array, 0, (-1i64) as u64, 1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, req, &[0, 0, 1, 1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, req, &[UNMAPPED, 0, 1, 1]), Err(Errno::EFAULT));
    let mixed = waitv(&mut k, &[(5, WORD, 0x82), (0, WORD2, 0x02)]);
    assert_eq!(sys(&mut k, req, &[mixed, 0, 1, 1]), Err(Errno::EINVAL), "kinds differ");
    let differs = waitv(&mut k, &[(6, WORD, 0x82), (0, WORD2, 0x82)]);
    assert_eq!(sys(&mut k, req, &[differs, 0, 1, 1]), Err(Errno::EAGAIN));
    let array = waitv(&mut k, &[(5, WORD, 0x82), (0, WORD2, 0x82)]);
    assert_eq!(sys(&mut k, req, &[array, 0, 1, 2]), Ok(3));
    assert_eq!(woken(&mut k), [1]);
    assert_eq!(k.futexes.waiters(key(WORD)).collect::<Vec<_>>(), [4]);
    assert_eq!(k.futexes.waiters(key(WORD2)).collect::<Vec<_>>(), [2, 3]);
    k.poke(WORD2, &5u32.to_le_bytes());
    let onto_itself = waitv(&mut k, &[(5, WORD2, 0x82), (0, WORD2, 0x82)]);
    assert_eq!(sys(&mut k, req, &[onto_itself, 0, 0, 9]), Ok(2), "counts sleepers");
    assert_eq!(k.futexes.waiters(key(WORD2)).collect::<Vec<_>>(), [2, 3]);
}
