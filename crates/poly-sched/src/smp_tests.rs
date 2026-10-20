//! Several CPUs: placement, stealing, kicks, affinity, the switch window
//! (`saving`), and a randomized check of the invariants across CPUs.

extern crate std;

use super::*;
use std::vec::Vec;

const BOOT: ThreadId = ThreadId::BOOT;

/// Slots on `cpu`'s run queue, in order.
fn run_queue<const N: usize, const C: usize>(s: &Scheduler<N, C>, cpu: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = s.cpus[cpu].run.head;
    while let Some(slot) = at {
        assert!(out.len() <= N, "cycle in a run queue");
        out.push(slot as usize);
        at = s.slots[slot as usize].next;
    }
    out
}

/// A scheduler with `cpus` CPUs online, each running its idle thread
/// except CPU 0 (the boot thread).
fn machine<const N: usize, const C: usize>(cpus: usize) -> Scheduler<N, C> {
    let mut s = Scheduler::<N, C>::new(3);
    for cpu in 1..cpus {
        let idle = s.add_cpu(cpu).unwrap();
        assert_eq!(s.current(cpu), idle);
        assert!(s.is_idle(cpu));
        assert_eq!(s.state(idle), Some(State::Running));
        s.set_online(cpu, true).unwrap();
    }
    s
}

#[test]
fn cpus_are_added_once_and_go_online() {
    let mut s = Scheduler::<16, 4>::new(3);
    assert_eq!(s.online(), CpuSet::one(0));
    assert_eq!(s.present(), CpuSet::one(0));
    assert_eq!(s.add_cpu(0), Err(Error::NoSuchCpu), "CPU 0 exists from the start");
    assert_eq!(s.add_cpu(4), Err(Error::NoSuchCpu), "beyond C");
    assert_eq!(s.set_online(2, true), Err(Error::NoSuchCpu), "not added yet");
    let idle = s.add_cpu(2).unwrap();
    assert_eq!(s.add_cpu(2), Err(Error::NoSuchCpu));
    assert_eq!(s.idle_thread(2), Some(idle));
    assert_eq!(s.idle_thread(1), None);
    assert!(s.is_idle_thread(idle) && s.is_idle_thread(ThreadId::IDLE) && !s.is_idle_thread(BOOT));
    assert_eq!(s.present(), CpuSet::from_bits(0b101));
    assert_eq!(s.online(), CpuSet::one(0), "added CPUs take no threads until online");
    // An offline CPU gets nothing.
    let t = s.spawn(0, CpuSet::ALL).unwrap();
    assert_eq!(s.cpu_of(t), Some(0));
    s.set_online(2, true).unwrap();
    assert_eq!(s.online(), CpuSet::from_bits(0b101));
    // Idle threads cannot be moved, blocked, joined or detached.
    assert_eq!(s.set_affinity(idle, CpuSet::ALL), Err(Error::Idle));
    assert_eq!(s.join(0, idle), Err(Error::Idle));
    assert_eq!(s.detach(idle), Err(Error::Idle));
    assert_eq!(s.block_current(2, &mut WaitQueue::new()), Err(Error::Idle));
    assert_eq!(s.exit_current(2, 0), Err(Error::Idle));
}

#[test]
fn new_threads_go_to_idle_cpus_first_and_kick_them() {
    let mut s = machine::<16, 4>(4);
    let _ = s.take_kicks();
    let ids: Vec<_> = (0..5).map(|_| s.spawn(0, CpuSet::ALL).unwrap()).collect();
    // One each on the idle CPUs 1-3, then the least loaded (all equal:
    // the spawner's CPU, 0, then the first of the others).
    let cpus: Vec<_> = ids.iter().map(|&t| s.cpu_of(t).unwrap()).collect();
    assert_eq!(cpus, [1, 2, 3, 0, 1]);
    assert_eq!(s.take_kicks(), CpuSet::from_bits(0b1110), "each idle CPU that got a thread");
    assert_eq!(s.take_kicks(), CpuSet::EMPTY);
    // Each idle CPU picks its thread up.
    for cpu in 1..4 {
        assert!(s.needs_switch(cpu));
        let sw = s.yield_now(cpu).unwrap();
        assert_eq!(sw.from, s.idle_thread(cpu).unwrap());
        assert_eq!(sw.to, ids[cpu - 1]);
        assert_eq!(s.state(sw.to), Some(State::Running));
        assert_eq!(s.state(sw.from), Some(State::Runnable), "an idle thread is runnable while another runs");
    }
    assert_eq!(run_queue(&s, 1), [ids[4].slot()]);
}

#[test]
fn a_thread_is_not_resumed_elsewhere_before_its_switch_completes() {
    let mut s = machine::<16, 2>(2);
    let a = s.spawn(0, CpuSet::one(0)).unwrap();
    // BOOT yields to a on CPU 0; BOOT is queued on CPU 0 but its registers
    // are not saved yet.
    assert_eq!(s.yield_now(0), Some(Switch { from: BOOT, to: a }));
    assert_eq!(run_queue(&s, 0), [BOOT.slot()]);
    let _ = s.take_kicks();
    assert!(!s.has_runnable(1), "CPU 1 must not take a thread that is still being switched away from");
    assert_eq!(s.yield_now(1), None);
    s.saved(0);
    assert_eq!(s.take_kicks(), CpuSet::one(1), "the idle CPU is told once it may");
    assert!(s.needs_switch(1));
    let idle1 = s.idle_thread(1).unwrap();
    assert_eq!(s.yield_now(1), Some(Switch { from: idle1, to: BOOT }), "stolen by the idle CPU");
    assert_eq!(s.cpu_of(BOOT), Some(1));
    assert!(run_queue(&s, 0).is_empty());
}

#[test]
fn an_exited_thread_is_reaped_only_once_switched_away_from() {
    let mut s = machine::<16, 2>(2);
    let t = s.spawn(1, CpuSet::one(1)).unwrap();
    s.yield_now(1).unwrap();
    let sw = s.exit_current(1, 5).unwrap();
    assert_eq!(sw.from, t);
    assert_eq!(s.unreaped(), None);
    assert_eq!(s.reap(t), Err(Error::NotExited));
    // Another CPU's operations do not complete CPU 1's switch.
    let _ = s.yield_now(0);
    assert_eq!(s.unreaped(), None);
    s.saved(1);
    assert_eq!(s.unreaped(), Some(t));
    s.reap(t).unwrap();
    assert_eq!(s.join(0, t), Ok(Join::Exited(5)));
}

#[test]
fn an_idle_cpu_steals_from_a_busy_one() {
    let mut s = machine::<16, 2>(2);
    // CPU 1 is offline while threads pile up on CPU 0.
    s.set_online(1, false).unwrap();
    let ids: Vec<_> = (0..3).map(|_| s.spawn(0, CpuSet::ALL).unwrap()).collect();
    assert_eq!(run_queue(&s, 0), ids.iter().map(|t| t.slot()).collect::<Vec<_>>());
    s.set_online(1, true).unwrap();
    // Its tick sees work to take.
    assert!(s.tick(1));
    let sw = s.yield_now(1).unwrap();
    assert_eq!(sw.to, ids[0], "the oldest waiting thread");
    assert_eq!(run_queue(&s, 0), [ids[1].slot(), ids[2].slot()]);
    // A pinned thread is never taken.
    let pinned = s.spawn(0, CpuSet::one(0)).unwrap();
    s.set_online(1, false).unwrap();
    let _ = s.exit_current(1, 0).unwrap();
    s.saved(1);
    s.set_online(1, true).unwrap();
    let mut taken = Vec::new();
    while let Some(sw) = s.yield_now(1) {
        taken.push(sw.to);
        if s.is_idle(1) {
            break;
        }
        s.exit_current(1, 0).unwrap();
        s.saved(1);
    }
    assert!(!taken.contains(&pinned), "a thread pinned to CPU 0 ran on CPU 1");
    assert_eq!(run_queue(&s, 0), [pinned.slot()]);
}

#[test]
fn a_busy_cpu_balances_at_the_end_of_its_slice() {
    let mut s = machine::<16, 2>(2);
    let busy = s.spawn(1, CpuSet::ALL).unwrap();
    s.yield_now(1).unwrap();
    s.set_online(1, false).unwrap();
    let ids: Vec<_> = (0..4).map(|_| s.spawn(0, CpuSet::ALL).unwrap()).collect();
    s.set_online(1, true).unwrap();
    assert_eq!(s.queued(0), 4);
    // CPU 1's own queue is empty: at the end of its slice it takes one.
    assert!(!s.tick(1));
    assert!(!s.tick(1));
    assert!(s.tick(1));
    assert_eq!(s.yield_now(1), Some(Switch { from: busy, to: ids[0] }));
    assert_eq!(run_queue(&s, 1), [busy.slot()]);
    assert_eq!(s.queued(0), 3);
    // Now 1 waits on CPU 1 and 3 on CPU 0: it takes another (3 >= 1 + 2).
    s.saved(1);
    for _ in 0..2 {
        assert!(!s.tick(1));
    }
    assert!(s.tick(1));
    assert_eq!(s.yield_now(1), Some(Switch { from: ids[0], to: ids[1] }));
    assert_eq!((s.queued(0), s.queued(1)), (2, 2));
    // Balanced: its own queue comes first again.
    s.saved(1);
    for _ in 0..2 {
        assert!(!s.tick(1));
    }
    assert!(s.tick(1));
    assert_eq!(s.yield_now(1), Some(Switch { from: ids[1], to: busy }));
}

#[test]
fn wakeups_go_back_to_their_cpu_when_it_is_idle() {
    let mut s = machine::<16, 3>(3);
    let t = s.spawn(0, CpuSet::ALL).unwrap();
    assert_eq!(s.cpu_of(t), Some(1));
    s.yield_now(1).unwrap();
    let mut q = WaitQueue::new();
    s.block_current(1, &mut q).unwrap();
    s.saved(1);
    let _ = s.take_kicks();
    assert_eq!(s.wake_one(&mut q), Some(t));
    assert_eq!(s.cpu_of(t), Some(1), "back where it ran, which is idle");
    assert_eq!(s.take_kicks(), CpuSet::one(1));
    // Busy elsewhere: an idle CPU gets it instead.
    s.yield_now(1).unwrap();
    s.block_current(1, &mut q).unwrap();
    s.saved(1);
    let u = s.spawn(1, CpuSet::one(1)).unwrap();
    s.yield_now(1).unwrap();
    assert_eq!(s.current(1), u);
    s.wake_one(&mut q).unwrap();
    assert_eq!(s.cpu_of(t), Some(2));
}

#[test]
fn affinity_moves_queued_threads_and_running_ones_at_the_next_tick() {
    let mut s = machine::<16, 3>(3);
    s.set_online(1, false).unwrap();
    s.set_online(2, false).unwrap();
    let t = s.spawn(0, CpuSet::ALL).unwrap();
    assert_eq!(s.cpu_of(t), Some(0));
    s.set_online(1, true).unwrap();
    s.set_online(2, true).unwrap();
    let _ = s.take_kicks();
    assert_eq!(s.set_affinity(t, CpuSet::EMPTY), Err(Error::NoSuchCpu));
    assert_eq!(s.set_affinity(t, CpuSet::one(9)), Err(Error::NoSuchCpu), "beyond C");
    let mut small = machine::<8, 4>(2);
    assert_eq!(small.spawn(0, CpuSet::one(3)), Err(Error::NoSuchCpu), "not added");
    s.set_affinity(t, CpuSet::one(2)).unwrap();
    assert_eq!(s.affinity(t), Some(CpuSet::one(2)));
    assert_eq!(s.cpu_of(t), Some(2), "moved off CPU 0's queue");
    assert!(run_queue(&s, 0).is_empty());
    assert_eq!(s.take_kicks(), CpuSet::one(2));
    assert!(!s.has_runnable(1), "CPU 1 may not run it");
    s.yield_now(2).unwrap();
    // Running on CPU 2, then pinned to CPU 1: CPU 2 is kicked and moves it
    // at its next tick, even with nothing else to run.
    s.set_affinity(t, CpuSet::one(1)).unwrap();
    assert_eq!(s.take_kicks(), CpuSet::one(2));
    assert!(s.needs_switch(2));
    assert!(s.tick(2));
    let idle2 = s.idle_thread(2).unwrap();
    assert_eq!(s.yield_now(2), Some(Switch { from: t, to: idle2 }));
    assert_eq!(s.cpu_of(t), Some(1));
    s.saved(2);
    assert_eq!(s.yield_now(1).map(|sw| sw.to), Some(t));
    // Spawning with an affinity no CPU here has fails.
    assert_eq!(s.spawn(0, CpuSet::one(5)), Err(Error::NoSuchCpu));
    // A thread pinned to an offline CPU waits on that CPU's queue.
    s.set_online(2, false).unwrap();
    let w = s.spawn(0, CpuSet::one(2)).unwrap();
    assert_eq!(s.cpu_of(w), Some(2));
    assert!(!s.has_runnable(0) || run_queue(&s, 0).iter().all(|&x| x != w.slot()));
}

#[test]
fn cpu_sets_print_as_ranges() {
    let set = |bits: u64| std::format!("{}", CpuSet::from_bits(bits));
    assert_eq!(set(0), "");
    assert_eq!(set(1), "0");
    assert_eq!(set(0b1111), "0-3");
    assert_eq!(set(0b1101), "0,2-3");
    assert_eq!(set(0b1010_0001), "0,5,7");
    assert_eq!(CpuSet::first_n(64), CpuSet::ALL);
    assert_eq!(CpuSet::first_n(3).iter().collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(CpuSet::one(64), CpuSet::EMPTY);
    let mut s = CpuSet::EMPTY;
    s.insert(3);
    s.insert(63);
    assert_eq!((s.len(), s.first(), s.contains(63), s.contains(64)), (2, Some(3), true, false));
    s.remove(3);
    assert_eq!(s, CpuSet::one(63));
    assert_eq!(CpuSet::first_n(4).without(CpuSet::one(1)), CpuSet::from_bits(0b1101));
    let id = ThreadId { slot: 7, generation: 0xdead_beef };
    assert_eq!(ThreadId::from_bits(id.to_bits()), id);
}

// --- Randomized invariant check across CPUs ---------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const SLOTS: usize = 24;
const CPUS: usize = 4;
const QUEUES: usize = 3;

/// Every invariant the kernel relies on, checked on the whole state.
fn check(s: &Scheduler<SLOTS, CPUS>, qs: &[WaitQueue]) {
    // Where each slot is listed: run queue of a CPU, a wait queue, a
    // joiners list, the free list.
    let mut listed = [0usize; SLOTS];
    for cpu in 0..CPUS {
        let q = run_queue(s, cpu);
        assert_eq!(q.len(), s.queued(cpu), "queued count of CPU {cpu}");
        for &slot in &q {
            listed[slot] += 1;
            let t = &s.slots[slot];
            assert_eq!(t.state, SlotState::Runnable, "slot {slot} queued on CPU {cpu}");
            assert!(!t.idle, "an idle thread is queued");
            assert_eq!(usize::from(t.cpu), cpu, "slot {slot} queued on CPU {cpu} says otherwise");
        }
    }
    let mut follow = |head: Link| {
        let mut at = head;
        while let Some(slot) = at {
            listed[slot as usize] += 1;
            at = s.slots[slot as usize].next;
        }
    };
    for q in qs {
        follow(q.head);
    }
    for slot in &s.slots {
        follow(slot.joiners.head);
    }
    follow(s.free);
    let mut running = Vec::new();
    for (slot, t) in s.slots.iter().enumerate() {
        match t.state {
            SlotState::Free | SlotState::Blocked => assert_eq!(listed[slot], 1, "slot {slot} ({:?}) listed", t.state),
            SlotState::Runnable if t.idle => assert_eq!(listed[slot], 0),
            SlotState::Runnable => assert_eq!(listed[slot], 1, "runnable slot {slot} on one run queue"),
            SlotState::Running | SlotState::Exited => assert_eq!(listed[slot], 0, "slot {slot} listed"),
        }
        if t.state == SlotState::Running {
            running.push(slot);
        }
        if t.idle {
            let cpu = usize::from(t.cpu);
            assert_eq!(s.cpus[cpu].idle as usize, slot, "idle thread of CPU {cpu}");
        }
        if t.saving {
            let owners = (0..CPUS).filter(|&c| s.cpus[c].prev == Some(slot as u16)).count();
            assert_eq!(owners, 1, "saving slot {slot} is the previous thread of one CPU");
        }
    }
    // The running threads are exactly the CPUs' current threads, each on
    // one CPU, and allowed there unless it must move.
    let mut currents: Vec<usize> =
        (0..CPUS).filter(|&c| s.cpus[c].exists()).map(|c| s.cpus[c].current as usize).collect();
    currents.sort_unstable();
    let before = currents.len();
    currents.dedup();
    assert_eq!(currents.len(), before, "a thread is current on two CPUs");
    assert_eq!(currents, running);
    for cpu in 0..CPUS {
        if s.cpus[cpu].exists() {
            let cur = &s.slots[s.cpus[cpu].current as usize];
            assert_eq!(usize::from(cur.cpu), cpu);
            assert!(!cur.saving, "a running thread is being saved");
        }
    }
    if let Some(t) = s.unreaped() {
        assert!(!s.slots[t.slot()].saving);
    }
}

fn run_random(seed: u64, steps: usize) {
    let mut rng = Rng(seed);
    let mut s = Scheduler::<SLOTS, CPUS>::new(1 + rng.below(3) as u32);
    let mut qs: Vec<WaitQueue> = (0..QUEUES).map(|_| WaitQueue::new()).collect();
    let mut ids = std::vec![BOOT];
    let mut added = 1;
    for _ in 0..steps {
        let cpu = rng.below(added);
        let affinity = match rng.below(4) {
            0 => CpuSet::one(rng.below(CPUS)),
            1 => CpuSet::from_bits(rng.next() & 0xf).union(CpuSet::one(0)),
            _ => CpuSet::ALL,
        };
        match rng.below(100) {
            0..=1 if added < CPUS => match s.add_cpu(added) {
                Ok(idle) => {
                    ids.push(idle);
                    s.set_online(added, true).unwrap();
                    added += 1;
                }
                Err(e) => assert_eq!(e, Error::TableFull),
            },
            2..=13 => {
                if let Ok(t) = s.spawn(cpu, affinity) {
                    ids.push(t);
                    assert!(affinity.contains(usize::from(s.slots[t.slot()].cpu)), "placed against its affinity");
                }
            }
            14..=27 => {
                if let Some(sw) = s.yield_now(cpu) {
                    assert_ne!(sw.from, sw.to);
                    let to = &s.slots[sw.to.slot()];
                    assert!(to.affinity.contains(cpu), "{} runs on CPU {cpu} against its affinity", sw.to);
                }
            }
            28..=41 => {
                if s.tick(cpu) && rng.below(3) != 0 {
                    s.yield_now(cpu);
                }
            }
            42..=51 => {
                let _ = s.block_current(cpu, &mut qs[rng.below(QUEUES)]);
            }
            52..=61 => {
                let _ = s.wake_one(&mut qs[rng.below(QUEUES)]);
            }
            62..=64 => {
                let _ = s.wake_all(&mut qs[rng.below(QUEUES)]);
            }
            65..=71 => {
                let _ = s.exit_current(cpu, 0);
            }
            72..=77 => {
                let t = ids[rng.below(ids.len())];
                let _ = s.join(cpu, t);
            }
            78..=80 => {
                let t = ids[rng.below(ids.len())];
                let _ = s.detach(t);
            }
            81..=85 => {
                let t = ids[rng.below(ids.len())];
                let _ = s.set_affinity(t, affinity);
            }
            86..=95 => {
                // Some CPU completes its switch and reaps, as the kernel
                // does right after it; others may lag behind.
                s.saved(cpu);
                while let Some(t) = s.unreaped() {
                    s.reap(t).unwrap();
                }
            }
            96..=97 if added > 1 => {
                let c = 1 + rng.below(added - 1);
                let online = s.online().contains(c);
                s.set_online(c, !online).unwrap();
            }
            _ => {
                let _ = s.take_kicks();
            }
        }
        check(&s, &qs);
    }
}

#[test]
fn invariants_hold_on_random_operations_across_cpus() {
    let mut seeds = Rng(0x0dd5_eed5_0f5c_4ed0);
    for _ in 0..64 {
        run_random(seeds.next() | 1, 4000);
    }
}
