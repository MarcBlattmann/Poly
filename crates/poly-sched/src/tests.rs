extern crate std;

use super::*;
use std::collections::VecDeque;
use std::vec::Vec;

const BOOT: ThreadId = ThreadId::BOOT;
const IDLE: ThreadId = ThreadId::IDLE;

fn sw(from: ThreadId, to: ThreadId) -> Option<Switch> {
    Some(Switch { from, to })
}

/// Slots on a list, following the links from `head`. Panics on a cycle.
fn list<const N: usize>(s: &Scheduler<N>, head: Link) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = head;
    while let Some(slot) = at {
        assert!(out.len() <= N, "cycle in a scheduler list");
        out.push(slot as usize);
        at = s.slots[slot as usize].next;
    }
    out
}

fn queue<const N: usize>(s: &Scheduler<N>, q: &WaitQueue) -> Vec<usize> {
    let l = list(s, q.head);
    assert_eq!(q.tail.map(usize::from), l.last().copied(), "tail is not the last element");
    l
}

#[test]
fn starts_with_boot_running_and_idle_ready() {
    let s = Scheduler::<4>::new(5);
    assert_eq!(s.current(), BOOT);
    assert_eq!(s.state(BOOT), Some(State::Running));
    assert_eq!(s.state(IDLE), Some(State::Runnable));
    assert_eq!(s.threads(), 2);
    assert!(!s.has_runnable());
    assert_eq!(s.time_slice(), 5);
    assert_eq!(std::format!("{BOOT} {IDLE}"), "0.0 1.0");
}

#[test]
fn spawn_fills_the_table_in_slot_order() {
    let mut s = Scheduler::<5>::new(1);
    let ids: Vec<_> = (0..3).map(|_| s.spawn().unwrap()).collect();
    assert_eq!(ids.iter().map(|t| t.slot()).collect::<Vec<_>>(), [2, 3, 4]);
    assert!(ids.iter().all(|&t| s.state(t) == Some(State::Runnable)));
    assert_eq!(s.spawn(), Err(Error::TableFull));
    assert_eq!(s.threads(), 5);
    assert_eq!(queue(&s, &s.run), [2, 3, 4]);
}

#[test]
fn tiny_table_has_no_free_slot() {
    let mut s = Scheduler::<2>::new(1);
    assert_eq!(s.spawn(), Err(Error::TableFull));
}

#[test]
fn yield_rotates_round_robin() {
    let mut s = Scheduler::<8>::new(3);
    assert_eq!(s.yield_now(), None, "nobody else to run");
    let a = s.spawn().unwrap();
    let b = s.spawn().unwrap();
    assert_eq!(s.yield_now(), sw(BOOT, a));
    assert_eq!(s.state(BOOT), Some(State::Runnable));
    assert_eq!(s.state(a), Some(State::Running));
    assert_eq!(s.yield_now(), sw(a, b));
    assert_eq!(s.yield_now(), sw(b, BOOT));
    assert_eq!(s.yield_now(), sw(BOOT, a));
    assert_eq!(queue(&s, &s.run), [b.slot(), BOOT.slot()]);
}

#[test]
fn tick_preempts_after_the_slice_only_if_someone_waits() {
    let mut s = Scheduler::<8>::new(3);
    // Alone: the slice just renews.
    for _ in 0..10 {
        assert!(!s.tick());
    }
    let a = s.spawn().unwrap();
    // The slice left over from the loop above: 10 ticks = 3 + 3 + 3 + 1.
    assert!(!s.tick());
    assert!(s.tick());
    assert!(s.tick(), "keeps asking until the caller switches");
    assert_eq!(s.yield_now(), sw(BOOT, a));
    // A switched-in thread gets a full slice.
    assert!(!s.tick());
    assert!(!s.tick());
    assert!(s.tick());
    assert_eq!(s.yield_now(), sw(a, BOOT));
}

#[test]
fn idle_runs_when_everyone_blocks_and_is_preempted_at_once() {
    let mut s = Scheduler::<8>::new(100);
    let mut q = WaitQueue::new();
    assert_eq!(s.block_current(&mut q), Ok(Switch { from: BOOT, to: IDLE }));
    assert_eq!(s.current(), IDLE);
    assert_eq!(s.state(BOOT), Some(State::Blocked));
    assert!(!s.tick(), "idle keeps running while nothing is runnable");
    assert_eq!(s.yield_now(), None);
    assert_eq!(s.wake_one(&mut q), Some(BOOT));
    assert!(s.tick(), "idle gives way on the next tick");
    assert_eq!(s.yield_now(), sw(IDLE, BOOT));
    assert_eq!(s.state(IDLE), Some(State::Runnable));
    assert!(!s.has_runnable(), "idle is never queued");
}

#[test]
fn idle_cannot_block_exit_or_be_joined() {
    let mut s = Scheduler::<8>::new(1);
    let mut q = WaitQueue::new();
    assert_eq!(s.join(IDLE), Err(Error::Idle));
    assert_eq!(s.detach(IDLE), Err(Error::Idle));
    assert_eq!(s.reap(IDLE), Err(Error::NotExited));
    s.block_current(&mut q).unwrap();
    assert_eq!(s.current(), IDLE);
    assert_eq!(s.block_current(&mut WaitQueue::new()), Err(Error::Idle));
    assert_eq!(s.exit_current(0), Err(Error::Idle));
    assert_eq!(s.join(BOOT), Err(Error::Idle), "idle cannot wait for a live thread");
}

#[test]
fn wait_queue_is_fifo() {
    let mut s = Scheduler::<8>::new(1);
    let mut q = WaitQueue::new();
    let ids: Vec<_> = (0..3).map(|_| s.spawn().unwrap()).collect();
    // Each of boot, ids[0], ids[1] blocks in turn; ids[2] is left running.
    assert_eq!(s.block_current(&mut q), Ok(Switch { from: BOOT, to: ids[0] }));
    assert_eq!(s.block_current(&mut q), Ok(Switch { from: ids[0], to: ids[1] }));
    assert_eq!(s.block_current(&mut q), Ok(Switch { from: ids[1], to: ids[2] }));
    assert_eq!(queue(&s, &q), [0, 2, 3]);
    assert_eq!(s.wake_one(&mut q), Some(BOOT));
    assert_eq!(s.wake_all(&mut q), 2);
    assert!(q.is_empty());
    assert_eq!(s.wake_one(&mut q), None);
    assert_eq!(s.wake_all(&mut q), 0);
    assert_eq!(queue(&s, &s.run), [0, 2, 3]);
}

#[test]
fn join_waits_for_exit_and_frees_the_slot() {
    let mut s = Scheduler::<4>::new(1);
    let t = s.spawn().unwrap();
    assert_eq!(s.join(t), Ok(Join::Blocked(Switch { from: BOOT, to: t })));
    assert_eq!(s.state(BOOT), Some(State::Blocked));
    assert_eq!(s.unreaped(), None);
    assert_eq!(s.exit_current(42), Ok(Switch { from: t, to: BOOT }), "exit wakes the joiner");
    assert_eq!(s.state(t), Some(State::Exited));
    assert_eq!(s.unreaped(), Some(t));
    assert_eq!(s.reap(t), Ok(()));
    assert_eq!(s.reap(t), Err(Error::NotExited));
    assert_eq!(s.unreaped(), None);
    assert_eq!(s.threads(), 3, "not joined yet");
    assert_eq!(s.join(t), Ok(Join::Exited(42)));
    assert_eq!(s.threads(), 2);
    assert_eq!(s.state(t), None);
    assert_eq!(s.join(t), Err(Error::NoSuchThread));
    // The slot comes back with a new generation; the old id stays dead.
    let u = s.spawn().unwrap();
    assert_eq!(u.slot(), t.slot());
    assert_ne!(u, t);
    assert_eq!(s.join(t), Err(Error::NoSuchThread));
    assert_eq!(s.detach(t), Err(Error::NoSuchThread));
    assert_eq!(s.reap(t), Err(Error::NoSuchThread));
}

#[test]
fn join_before_reap_frees_at_reap() {
    let mut s = Scheduler::<4>::new(1);
    let t = s.spawn().unwrap();
    s.yield_now().unwrap();
    s.exit_current(7).unwrap();
    assert_eq!(s.join(t), Ok(Join::Exited(7)));
    assert_eq!(s.state(t), None, "joined threads are gone");
    assert_eq!(s.threads(), 3, "but the slot waits for reap");
    assert_eq!(s.spawn().map(|u| u.slot()), Ok(3));
    assert_eq!(s.spawn(), Err(Error::TableFull));
    assert_eq!(s.unreaped(), Some(t));
    s.reap(t).unwrap();
    assert_eq!(s.threads(), 3);
    assert_eq!(s.spawn().map(|u| u.slot()), Ok(t.slot()));
}

#[test]
fn live_threads_lists_every_unjoined_thread() {
    let mut s = Scheduler::<6>::new(1);
    let a = s.spawn().unwrap();
    let b = s.spawn().unwrap();
    let c = s.spawn().unwrap();
    let mut q = WaitQueue::new();
    s.block_current(&mut q).unwrap(); // BOOT blocks, a runs
    s.exit_current(1).unwrap(); // a exits, b runs
    s.join(c).unwrap(); // b waits for c, which runs
    let live: Vec<_> = s.live_threads().collect();
    assert_eq!(
        live,
        [(BOOT, State::Blocked), (IDLE, State::Runnable), (a, State::Exited), (b, State::Blocked), (c, State::Running)]
    );
    s.reap(a).unwrap();
    s.detach(a).unwrap();
    assert_eq!(s.live_threads().map(|(t, _)| t).collect::<Vec<_>>(), [BOOT, IDLE, b, c], "a's slot is free");
    s.exit_current(0).unwrap(); // c exits and wakes b, which runs
    assert_eq!(s.current(), b);
    assert_eq!(s.join(c), Ok(Join::Exited(0)));
    assert!(s.live_threads().all(|(t, _)| t != c), "joined, though not reaped yet");
}

#[test]
fn detached_threads_free_themselves() {
    let mut s = Scheduler::<4>::new(1);
    let t = s.spawn().unwrap();
    s.detach(t).unwrap();
    s.detach(t).unwrap();
    assert_eq!(s.join(t), Err(Error::Detached));
    s.yield_now().unwrap();
    s.exit_current(1).unwrap();
    assert_eq!(s.state(t), Some(State::Exited));
    s.reap(t).unwrap();
    assert_eq!(s.state(t), None);
    assert_eq!(s.threads(), 2);

    // Detaching after exit and reap frees at once.
    let u = s.spawn().unwrap();
    s.yield_now().unwrap();
    s.exit_current(1).unwrap();
    s.reap(u).unwrap();
    assert_eq!(s.threads(), 3);
    s.detach(u).unwrap();
    assert_eq!(s.threads(), 2);
}

#[test]
fn joiners_of_a_thread_detached_later_get_an_error() {
    let mut s = Scheduler::<4>::new(1);
    let t = s.spawn().unwrap();
    assert!(matches!(s.join(t), Ok(Join::Blocked(_))));
    s.detach(t).unwrap();
    s.exit_current(0).unwrap();
    assert_eq!(s.current(), BOOT);
    assert_eq!(s.join(t), Err(Error::Detached));
}

#[test]
fn second_joiner_finds_the_thread_gone() {
    let mut s = Scheduler::<8>::new(1);
    let t = s.spawn().unwrap();
    let other = s.spawn().unwrap();
    assert_eq!(s.join(t), Ok(Join::Blocked(Switch { from: BOOT, to: t })));
    assert_eq!(s.yield_now(), sw(t, other));
    assert_eq!(s.join(t), Ok(Join::Blocked(Switch { from: other, to: t })));
    assert_eq!(queue(&s, &s.slots[t.slot()].joiners), [BOOT.slot(), other.slot()]);
    assert_eq!(s.join(t), Err(Error::Current));
    assert_eq!(s.exit_current(9), Ok(Switch { from: t, to: BOOT }));
    assert_eq!(s.join(t), Ok(Join::Exited(9)));
    assert_eq!(s.yield_now(), sw(BOOT, other));
    assert_eq!(s.join(t), Err(Error::NoSuchThread));
}

#[test]
fn boot_thread_may_exit() {
    let mut s = Scheduler::<4>::new(1);
    let t = s.spawn().unwrap();
    assert_eq!(s.exit_current(3), Ok(Switch { from: BOOT, to: t }));
    s.reap(BOOT).unwrap();
    assert_eq!(s.join(BOOT), Ok(Join::Exited(3)));
    assert_eq!(s.state(BOOT), None);
    assert_eq!(s.exit_current(0), Ok(Switch { from: t, to: IDLE }));
}

#[test]
fn bad_ids_are_rejected() {
    let mut s = Scheduler::<4>::new(1);
    let beyond = ThreadId { slot: 9, generation: 0 };
    let never = ThreadId { slot: 3, generation: 0 };
    let stale_boot = ThreadId { slot: 0, generation: 1 };
    for t in [beyond, never, stale_boot] {
        assert_eq!(s.state(t), None);
        assert_eq!(s.join(t), Err(Error::NoSuchThread));
        assert_eq!(s.detach(t), Err(Error::NoSuchThread));
        assert_eq!(s.reap(t), Err(Error::NoSuchThread));
    }
    assert_eq!(s.reap(BOOT), Err(Error::NotExited), "a running thread cannot be reaped");
}

#[test]
fn errors_have_descriptions() {
    for e in [Error::TableFull, Error::NoSuchThread, Error::Idle, Error::Current, Error::Detached, Error::NotExited] {
        assert!(!e.describe().is_empty());
    }
}

// --- Randomized model check -------------------------------------------------

/// xorshift64*: deterministic and dependency-free.
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

#[derive(Clone, Debug)]
struct MThread {
    id: ThreadId,
    state: State,
    detached: bool,
    joined: bool,
    reaped: bool,
    code: usize,
    joiners: VecDeque<usize>,
}

impl MThread {
    fn new(id: ThreadId, state: State) -> MThread {
        MThread { id, state, detached: false, joined: false, reaped: false, code: 0, joiners: VecDeque::new() }
    }
}

/// The scheduler's rules restated with std collections and no intrusive
/// links, slot by slot.
struct Model {
    threads: Vec<Option<MThread>>,
    run: VecDeque<usize>,
    queues: Vec<VecDeque<usize>>,
    current: usize,
    slice: u32,
    left: u32,
}

const IDLE_S: usize = 1;

impl Model {
    fn new(n: usize, slice: u32, queues: usize) -> Model {
        let mut threads = std::vec![None; n];
        threads[0] = Some(MThread::new(BOOT, State::Running));
        threads[1] = Some(MThread::new(IDLE, State::Runnable));
        Model {
            threads,
            run: VecDeque::new(),
            queues: std::vec![VecDeque::new(); queues],
            current: 0,
            slice,
            left: slice,
        }
    }

    fn t(&mut self, slot: usize) -> &mut MThread {
        self.threads[slot].as_mut().unwrap()
    }

    fn id(&self, slot: usize) -> ThreadId {
        self.threads[slot].as_ref().unwrap().id
    }

    fn existing(&self, t: ThreadId) -> Option<usize> {
        let m = self.threads.get(t.slot())?.as_ref()?;
        (m.id == t).then_some(t.slot())
    }

    fn live(&self, t: ThreadId) -> Option<usize> {
        self.existing(t).filter(|&s| !self.threads[s].as_ref().unwrap().joined)
    }

    fn state(&self, t: ThreadId) -> Option<State> {
        self.live(t).map(|s| self.threads[s].as_ref().unwrap().state)
    }

    fn switch_next(&mut self) -> Switch {
        let from = self.id(self.current);
        let next = self.run.pop_front().unwrap_or(IDLE_S);
        self.t(next).state = State::Running;
        self.current = next;
        self.left = self.slice;
        Switch { from, to: self.id(next) }
    }

    fn wake(&mut self, slot: usize) {
        assert_eq!(self.t(slot).state, State::Blocked);
        self.t(slot).state = State::Runnable;
        self.run.push_back(slot);
    }

    fn yield_now(&mut self) -> Option<Switch> {
        if self.run.is_empty() {
            return None;
        }
        let cur = self.current;
        self.t(cur).state = State::Runnable;
        if cur != IDLE_S {
            self.run.push_back(cur);
        }
        Some(self.switch_next())
    }

    fn tick(&mut self) -> bool {
        if self.current == IDLE_S {
            return !self.run.is_empty();
        }
        if self.left > 1 {
            self.left -= 1;
            return false;
        }
        self.left = 0;
        if self.run.is_empty() {
            self.left = self.slice;
            false
        } else {
            true
        }
    }

    fn block(&mut self, q: usize) -> Result<Switch, Error> {
        if self.current == IDLE_S {
            return Err(Error::Idle);
        }
        let cur = self.current;
        self.t(cur).state = State::Blocked;
        self.queues[q].push_back(cur);
        Ok(self.switch_next())
    }

    fn wake_one(&mut self, q: usize) -> Option<ThreadId> {
        let slot = self.queues[q].pop_front()?;
        self.wake(slot);
        Some(self.id(slot))
    }

    fn exit(&mut self, code: usize) -> Result<Switch, Error> {
        if self.current == IDLE_S {
            return Err(Error::Idle);
        }
        let cur = self.current;
        let t = self.t(cur);
        t.state = State::Exited;
        t.code = code;
        let joiners = core::mem::take(&mut t.joiners);
        for j in joiners {
            self.wake(j);
        }
        Ok(self.switch_next())
    }

    fn free_if_done(&mut self, slot: usize) {
        let t = self.threads[slot].as_ref().unwrap();
        if t.state == State::Exited && t.reaped && (t.joined || t.detached) {
            self.threads[slot] = None;
        }
    }

    fn join(&mut self, id: ThreadId) -> Result<Join, Error> {
        let slot = self.live(id).ok_or(Error::NoSuchThread)?;
        if slot == IDLE_S {
            return Err(Error::Idle);
        }
        if slot == self.current {
            return Err(Error::Current);
        }
        if self.t(slot).detached {
            return Err(Error::Detached);
        }
        if self.t(slot).state == State::Exited {
            self.t(slot).joined = true;
            let code = self.t(slot).code;
            self.free_if_done(slot);
            return Ok(Join::Exited(code));
        }
        if self.current == IDLE_S {
            return Err(Error::Idle);
        }
        let cur = self.current;
        self.t(cur).state = State::Blocked;
        self.t(slot).joiners.push_back(cur);
        Ok(Join::Blocked(self.switch_next()))
    }

    fn detach(&mut self, id: ThreadId) -> Result<(), Error> {
        let slot = self.live(id).ok_or(Error::NoSuchThread)?;
        if slot == IDLE_S {
            return Err(Error::Idle);
        }
        self.t(slot).detached = true;
        self.free_if_done(slot);
        Ok(())
    }

    fn reap(&mut self, id: ThreadId) -> Result<(), Error> {
        let slot = self.existing(id).ok_or(Error::NoSuchThread)?;
        let t = self.t(slot);
        if t.state != State::Exited || t.reaped {
            return Err(Error::NotExited);
        }
        t.reaped = true;
        self.free_if_done(slot);
        Ok(())
    }

    fn unreaped(&self) -> Vec<ThreadId> {
        self.threads.iter().flatten().filter(|t| t.state == State::Exited && !t.reaped).map(|t| t.id).collect()
    }
}

const MODEL_SLOTS: usize = 8;
const MODEL_QUEUES: usize = 3;

/// Compare every observable piece of state, and the lists themselves.
fn compare(s: &Scheduler<MODEL_SLOTS>, m: &Model, qs: &[WaitQueue], ids: &[ThreadId]) {
    assert_eq!(s.current(), m.id(m.current));
    for &id in ids {
        assert_eq!(s.state(id), m.state(id), "state of {id}");
    }
    assert_eq!(s.threads(), m.threads.iter().flatten().count());
    assert_eq!(s.has_runnable(), !m.run.is_empty());
    assert_eq!(queue(s, &s.run), Vec::from(m.run.clone()));
    for (q, mq) in qs.iter().zip(&m.queues) {
        assert_eq!(queue(s, q), Vec::from(mq.clone()));
    }
    for (slot, t) in m.threads.iter().enumerate() {
        let joiners = queue(s, &s.slots[slot].joiners);
        assert_eq!(joiners, t.as_ref().map_or(Vec::new(), |t| Vec::from(t.joiners.clone())), "joiners of {slot}");
    }
    let free = list(s, s.free);
    assert_eq!(free.len(), MODEL_SLOTS - s.threads());
    assert!(free.iter().all(|&f| m.threads[f].is_none()), "a used slot is on the free list");
    assert_eq!(s.left, m.left, "ticks left in the slice");
    if let Some(t) = s.unreaped() {
        assert!(m.unreaped().contains(&t));
    } else {
        assert!(m.unreaped().is_empty());
    }
}

fn pick(rng: &mut Rng, ids: &[ThreadId]) -> ThreadId {
    ids[rng.below(ids.len())]
}

fn run_model(seed: u64, steps: usize) {
    let mut rng = Rng(seed);
    let slice = 1 + rng.below(4) as u32;
    let mut s = Scheduler::<MODEL_SLOTS>::new(slice);
    let mut m = Model::new(MODEL_SLOTS, slice, MODEL_QUEUES);
    let mut qs: Vec<WaitQueue> = (0..MODEL_QUEUES).map(|_| WaitQueue::new()).collect();
    // Every id ever handed out, including stale ones, plus never-issued ids.
    let mut ids = std::vec![BOOT, IDLE, ThreadId { slot: 5, generation: 7 }, ThreadId { slot: 200, generation: 0 }];
    let mut last_generation: [Option<u32>; MODEL_SLOTS] = core::array::from_fn(|i| (i < 2).then_some(0));
    let mut code = 0usize;

    for _ in 0..steps {
        match rng.below(100) {
            0..=14 => match s.spawn() {
                Ok(t) => {
                    assert!(m.threads[t.slot()].is_none(), "spawn reused a live slot");
                    assert!(last_generation[t.slot()].is_none_or(|g| t.generation() > g), "generation reused");
                    last_generation[t.slot()] = Some(t.generation());
                    m.threads[t.slot()] = Some(MThread::new(t, State::Runnable));
                    m.run.push_back(t.slot());
                    ids.push(t);
                }
                Err(e) => {
                    assert_eq!(e, Error::TableFull);
                    assert!(m.threads.iter().all(Option::is_some));
                }
            },
            15..=29 => assert_eq!(s.yield_now(), m.yield_now()),
            30..=44 => {
                let preempt = s.tick();
                assert_eq!(preempt, m.tick());
                if preempt && rng.below(4) != 0 {
                    assert_eq!(s.yield_now(), m.yield_now());
                }
            }
            45..=54 => {
                let q = rng.below(MODEL_QUEUES);
                assert_eq!(s.block_current(&mut qs[q]), m.block(q));
            }
            55..=62 => {
                let q = rng.below(MODEL_QUEUES);
                assert_eq!(s.wake_one(&mut qs[q]), m.wake_one(q));
            }
            63..=65 => {
                let q = rng.below(MODEL_QUEUES);
                let expected = m.queues[q].len();
                while m.wake_one(q).is_some() {}
                assert_eq!(s.wake_all(&mut qs[q]), expected);
            }
            66..=73 => {
                code += 1;
                assert_eq!(s.exit_current(code), m.exit(code));
            }
            74..=81 => {
                let t = pick(&mut rng, &ids);
                assert_eq!(s.join(t), m.join(t), "join {t}");
            }
            82..=86 => {
                let t = pick(&mut rng, &ids);
                assert_eq!(s.detach(t), m.detach(t), "detach {t}");
            }
            87..=94 => {
                // What the kernel does after every switch.
                while let Some(t) = s.unreaped() {
                    assert_ne!(t, s.current());
                    assert_eq!(s.reap(t), m.reap(t));
                }
            }
            _ => {
                let t = pick(&mut rng, &ids);
                assert_eq!(s.reap(t), m.reap(t), "reap {t}");
            }
        }
        compare(&s, &m, &qs, &ids);
    }
}

#[test]
fn matches_reference_model_on_random_operations() {
    let mut seeds = Rng(0x5eed_0f5c_4ed0_1e5d);
    for _ in 0..64 {
        run_model(seeds.next() | 1, 4000);
    }
}
