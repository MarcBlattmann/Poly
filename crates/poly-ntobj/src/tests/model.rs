//! Randomized check of the dispatcher against a reference model.
//!
//! The model keeps every blocked wait in one list in the order the waits
//! blocked, and applies the documented rules directly: wait-any takes the
//! smallest ready index, wait-all takes everything at once, a signaled
//! object tries the waits on it oldest first. Many simulated threads wait,
//! signal, release, time out and die at random; after every step the
//! crate's wakes, statuses and object states must match the model's, and no
//! blocked wait may be satisfiable.

use super::*;

const THREADS: usize = 6;

#[derive(Clone, Debug)]
enum MObj {
    Event { sync: bool, signaled: bool },
    Mutant { owner: Option<usize>, count: u32, abandoned: bool },
    Semaphore { count: i32, maximum: i32 },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ready {
    No,
    Yes,
    Abandoned,
}

#[derive(Debug)]
struct MWait {
    id: WaitId,
    thread: usize,
    objects: Vec<usize>,
    all: bool,
}

#[derive(Debug, Default)]
struct Model {
    objects: Vec<MObj>,
    waits: Vec<MWait>,
    wakes: Vec<(WaitId, usize, NtStatus)>,
}

impl Model {
    fn ready(&self, o: usize, t: usize) -> Ready {
        match self.objects[o] {
            MObj::Event { signaled: true, .. } => Ready::Yes,
            MObj::Event { .. } => Ready::No,
            MObj::Semaphore { count, .. } => {
                if count > 0 {
                    Ready::Yes
                } else {
                    Ready::No
                }
            }
            MObj::Mutant { owner: None, abandoned: true, .. } => Ready::Abandoned,
            MObj::Mutant { owner: None, .. } => Ready::Yes,
            MObj::Mutant { owner: Some(x), .. } if x == t => Ready::Yes,
            MObj::Mutant { .. } => Ready::No,
        }
    }

    fn take(&mut self, o: usize, t: usize) {
        match &mut self.objects[o] {
            MObj::Event { sync: true, signaled } => *signaled = false,
            MObj::Event { .. } => {}
            MObj::Semaphore { count, .. } => *count -= 1,
            MObj::Mutant { owner, count, abandoned } => {
                *owner = Some(t);
                *count += 1;
                *abandoned = false;
            }
        }
    }

    /// The status a wait would finish with now, without taking anything.
    fn check(&self, t: usize, objects: &[usize], all: bool) -> Option<NtStatus> {
        if all {
            if objects.iter().any(|&o| self.ready(o, t) == Ready::No) {
                return None;
            }
            let abandoned = objects.iter().position(|&o| self.ready(o, t) == Ready::Abandoned);
            Some(abandoned.map_or(STATUS_WAIT_0, NtStatus::abandoned))
        } else {
            let i = objects.iter().position(|&o| self.ready(o, t) != Ready::No)?;
            Some(if self.ready(objects[i], t) == Ready::Abandoned { NtStatus::abandoned(i) } else { NtStatus::wait(i) })
        }
    }

    fn satisfy(&mut self, t: usize, objects: &[usize], all: bool) -> Option<NtStatus> {
        let status = self.check(t, objects, all)?;
        if all {
            for &o in objects {
                self.take(o, t);
            }
        } else {
            let i = objects.iter().position(|&o| self.ready(o, t) != Ready::No).unwrap();
            self.take(objects[i], t);
        }
        Some(status)
    }

    /// Try the waits on `o`, oldest first.
    fn settle(&mut self, o: usize) {
        let mut i = 0;
        while i < self.waits.len() {
            if self.waits[i].objects.contains(&o) {
                let (t, objects, all) = (self.waits[i].thread, self.waits[i].objects.clone(), self.waits[i].all);
                if let Some(status) = self.satisfy(t, &objects, all) {
                    let w = self.waits.remove(i);
                    self.wakes.push((w.id, w.thread, status));
                    continue;
                }
            }
            i += 1;
        }
    }

    fn blocked(&self, t: usize) -> bool {
        self.waits.iter().any(|w| w.thread == t)
    }
}

/// xorshift64*: deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn thread(t: usize) -> ThreadRef {
    ThreadRef(100 + t as u64)
}

fn compare(om: &mut ObjectManager, model: &mut Model, ids: &[ObjectId], step: usize, seed: u64) {
    let ctx = || std::format!("seed {seed:#x} step {step}");
    let got: Vec<(WaitId, usize, NtStatus)> =
        wakes(om).into_iter().map(|w| (w.wait, (w.thread.0 - 100) as usize, w.status)).collect();
    assert_eq!(got, core::mem::take(&mut model.wakes), "wakes, {}", ctx());
    for (o, m) in model.objects.iter().enumerate() {
        let id = ids[o];
        match *m {
            MObj::Event { signaled, .. } => assert_eq!(om.event_info(id).unwrap().signaled, signaled, "{}", ctx()),
            MObj::Semaphore { count, .. } => assert_eq!(om.semaphore_info(id).unwrap().count, count, "{}", ctx()),
            MObj::Mutant { owner, count, abandoned } => assert_eq!(
                om.mutant_info(id).unwrap(),
                MutantInfo { owner: owner.map(thread), recursion: count, abandoned },
                "object {o}, {}",
                ctx()
            ),
        }
    }
    assert_eq!(om.pending_waits(), model.waits.len(), "{}", ctx());
    for w in &model.waits {
        assert!(om.wait_pending(w.id), "{}", ctx());
        // Nothing blocked could go now: no lost wake-ups.
        assert_eq!(model.check(w.thread, &w.objects, w.all), None, "{}", ctx());
        let objects: Vec<ObjectId> = w.objects.iter().map(|&o| ids[o]).collect();
        let ready = objects.iter().filter(|&&o| om.is_ready_for(o, thread(w.thread))).count();
        if w.all {
            assert!(ready < objects.len(), "{}", ctx());
        } else {
            assert_eq!(ready, 0, "{}", ctx());
        }
    }
}

fn run(seed: u64, steps: usize) {
    let mut rng = Rng(seed);
    let mut om = ObjectManager::new();
    let mut model = Model::default();
    let mut ids = Vec::new();
    let kinds = [
        MObj::Event { sync: false, signaled: false },
        MObj::Event { sync: false, signaled: true },
        MObj::Event { sync: true, signaled: false },
        MObj::Event { sync: true, signaled: true },
        MObj::Mutant { owner: None, count: 0, abandoned: false },
        MObj::Mutant { owner: None, count: 0, abandoned: false },
        MObj::Semaphore { count: 0, maximum: 2 },
        MObj::Semaphore { count: 1, maximum: 3 },
    ];
    for k in kinds {
        let new = match k {
            MObj::Event { sync, signaled } => NewObject::Event {
                kind: if sync { EventType::Synchronization } else { EventType::Notification },
                signaled,
            },
            MObj::Mutant { .. } => NewObject::Mutant { initial_owner: false },
            MObj::Semaphore { count, maximum } => NewObject::Semaphore { initial: count, maximum },
        };
        ids.push(om.new_object(new, thread(0)).unwrap());
        model.objects.push(k);
    }
    let n = ids.len();
    for step in 0..steps {
        match rng.below(100) {
            // A thread that is not blocked waits.
            0..=34 => {
                let t = rng.below(THREADS);
                if model.blocked(t) {
                    continue;
                }
                let all = rng.below(3) == 0;
                let count = 1 + rng.below(4);
                let mut objects = Vec::new();
                while objects.len() < count {
                    let o = rng.below(n);
                    // Wait-any may repeat an object; wait-all may not.
                    if !all && rng.below(8) == 0 || !objects.contains(&o) {
                        objects.push(o);
                    }
                }
                let zero = rng.below(4) == 0;
                let handles: Vec<ObjectId> = objects.iter().map(|&o| ids[o]).collect();
                let wt = if all { WaitType::All } else { WaitType::Any };
                let timeout = if zero { Timeout::Zero } else { Timeout::Infinite };
                let got = om.wait_objects(thread(t), &handles, wt, rng.below(2) == 0, timeout).unwrap();
                match (model.satisfy(t, &objects, all), got) {
                    (Some(s), WaitOutcome::Done(g)) => assert_eq!(s, g, "seed {seed:#x} step {step}"),
                    (None, WaitOutcome::Done(g)) if zero => assert_eq!(g, STATUS_TIMEOUT),
                    (None, WaitOutcome::Blocked(id)) if !zero => {
                        model.waits.push(MWait { id, thread: t, objects, all })
                    }
                    (want, got) => panic!("seed {seed:#x} step {step}: model {want:?}, crate {got:?}"),
                }
            }
            35..=49 => {
                let o = rng.below(4);
                let op = rng.below(3);
                let MObj::Event { signaled, .. } = model.objects[o] else { unreachable!() };
                let previous = match op {
                    0 => om.set_event(ids[o]),
                    1 => om.reset_event(ids[o]),
                    _ => om.pulse_event(ids[o]),
                };
                assert_eq!(previous, Ok(signaled));
                if op != 1 {
                    let MObj::Event { signaled, .. } = &mut model.objects[o] else { unreachable!() };
                    *signaled = true;
                    model.settle(o);
                }
                if op != 0 {
                    let MObj::Event { signaled, .. } = &mut model.objects[o] else { unreachable!() };
                    *signaled = false;
                }
            }
            50..=64 => {
                let o = 6 + rng.below(2);
                let release = 1 + rng.below(3) as i32;
                let MObj::Semaphore { count, maximum } = &mut model.objects[o] else { unreachable!() };
                let previous = *count;
                if previous + release > *maximum {
                    assert_eq!(om.release_semaphore(ids[o], release), Err(STATUS_SEMAPHORE_LIMIT_EXCEEDED));
                } else {
                    *count += release;
                    assert_eq!(om.release_semaphore(ids[o], release), Ok(previous));
                    model.settle(o);
                }
            }
            65..=84 => {
                let o = 4 + rng.below(2);
                let t = rng.below(THREADS);
                if model.blocked(t) {
                    continue;
                }
                let MObj::Mutant { owner, count, .. } = &mut model.objects[o] else { unreachable!() };
                if *owner != Some(t) {
                    assert_eq!(om.release_mutant(ids[o], thread(t)), Err(STATUS_MUTANT_NOT_OWNED));
                } else {
                    assert_eq!(om.release_mutant(ids[o], thread(t)), Ok(*count));
                    *count -= 1;
                    if *count == 0 {
                        *owner = None;
                        model.settle(o);
                    }
                }
            }
            85..=94 => {
                if model.waits.is_empty() {
                    continue;
                }
                let w = model.waits.remove(rng.below(model.waits.len()));
                assert_eq!(om.cancel_wait(w.id, WaitCancel::Timeout), Ok(STATUS_TIMEOUT));
            }
            _ => {
                let t = rng.below(THREADS);
                om.thread_terminated(thread(t));
                model.waits.retain(|w| w.thread != t);
                for o in 0..n {
                    if let MObj::Mutant { owner, count, abandoned } = &mut model.objects[o]
                        && *owner == Some(t)
                    {
                        *owner = None;
                        *count = 0;
                        *abandoned = true;
                        model.settle(o);
                    }
                }
            }
        }
        compare(&mut om, &mut model, &ids, step, seed);
    }
    // Nothing leaks: end every wait, and the objects go with their
    // creation references.
    for w in core::mem::take(&mut model.waits) {
        om.cancel_wait(w.id, WaitCancel::Terminated).unwrap();
    }
    for id in ids {
        om.dereference(id).unwrap();
        assert!(!om.is_alive(id), "seed {seed:#x}");
    }
    assert_eq!(om.object_count(), ObjectManager::new().object_count());
}

#[test]
fn dispatcher_matches_the_model() {
    let mut seeds = Rng(0x5eed_0b1e_c7ab_1e5d);
    for _ in 0..150 {
        run(seeds.next() | 1, 300);
    }
}

#[test]
fn many_waiters_on_one_semaphore() {
    // A long queue: 64 threads block on one semaphore, then units come back
    // a few at a time and go out strictly in arrival order.
    let mut om = ObjectManager::new();
    let s = om.new_object(NewObject::Semaphore { initial: 0, maximum: 64 }, ThreadRef(0)).unwrap();
    let waits: Vec<WaitId> = (0..64).map(|t| blocked(wait1(&mut om, ThreadRef(t), s))).collect();
    let mut next = 0;
    for release in [1, 5, 3, 7, 16, 32] {
        om.release_semaphore(s, release).unwrap();
        let got: Vec<WaitId> = wakes(&mut om).into_iter().map(|w| w.wait).collect();
        assert_eq!(got, waits[next..next + release as usize]);
        next += release as usize;
    }
    assert_eq!(om.pending_waits(), 0);
    assert_eq!(om.semaphore_info(s).unwrap().count, 0);
}
