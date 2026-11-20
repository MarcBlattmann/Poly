//! A host model of the kernel under the XNU personality, for replaying the
//! visible spec tests (`specs/macos/**/<entry>.tests/*.c`) on the host.
//!
//! Each simulated process is a host thread with its own task: an address
//! space of regions over shared or private memory objects, Mach state shared
//! through one [`MachState`], blocking on a condition variable, fork that
//! copies the address space by inheritance, signals that end a process.
//! A test body is the C test transliterated against [`P`]; its output lines
//! (`ok`, `not ok`, `value`, `summary`) are compared byte for byte with the
//! arm64 output recorded on macOS (`macos.json`). The model runs as arm64
//! (16 KiB pages, the 24 MHz timebase) to match those recordings.

#![allow(dead_code)]

use poly_mach::ulock;
use poly_mach::{SenderInfo, TaskId, ThreadId};
use poly_personality_xnu::kernel::{
    Arch, Backing, Fault, Inherit, MachState, PageInfo, Region, ThreadStart, Wakeup, XnuKernel,
};
use poly_personality_xnu::{Class, Ret, call};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Once};
use std::time::{Duration, Instant};

pub const PS: u64 = 16384;
pub const PROT_READ: u64 = 1;
pub const PROT_WRITE: u64 = 2;
pub const PROT_EXEC: u64 = 4;

type Obj = Arc<Mutex<Vec<u8>>>;

#[derive(Clone)]
struct FRegion {
    start: u64,
    end: u64,
    prot: u32,
    max: u32,
    backing: Backing,
    inherit: Inherit,
    obj: Obj,
    obj_id: u64,
    /// Offset of `start` in the object.
    off: u64,
    locked: bool,
}

#[derive(Clone, Copy, Default)]
struct Page {
    present: bool,
    referenced: bool,
    modified: bool,
}

#[derive(Default)]
struct Task {
    regions: Vec<FRegion>,
    pages: BTreeMap<u64, Page>,
    next_map: u64,
}

impl Task {
    fn find(&self, addr: u64) -> Option<usize> {
        self.regions.iter().position(|r| r.start <= addr && addr < r.end)
    }

    fn split(&mut self, at: u64) {
        if let Some(i) = self.find(at)
            && self.regions[i].start < at
        {
            let mut hi = self.regions[i].clone();
            hi.off += at - hi.start;
            hi.start = at;
            self.regions[i].end = at;
            self.regions.insert(i + 1, hi);
        }
    }

    /// Split at the range's ends; the indices of the regions inside it.
    fn range(&mut self, start: u64, end: u64) -> Vec<usize> {
        self.split(start);
        self.split(end);
        (0..self.regions.len()).filter(|&i| self.regions[i].start >= start && self.regions[i].end <= end).collect()
    }
}

#[derive(Default)]
struct Slot {
    unblocked: bool,
    signal: Option<u32>,
}

pub struct World {
    mach: Mutex<MachState>,
    tasks: Mutex<BTreeMap<TaskId, Task>>,
    threads: Mutex<BTreeMap<ThreadId, Slot>>,
    cv: Condvar,
    procs: Mutex<BTreeMap<TaskId, Proc>>,
    next_id: AtomicU64,
    start: Instant,
    pub out: Mutex<Checker>,
}

struct Proc {
    thread: ThreadId,
    status: Arc<Mutex<Option<i32>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// A process ended by a signal: unwinds the process's host thread.
struct Killed(u32);

#[derive(Default)]
pub struct Checker {
    text: String,
    passed: u32,
    failed: u32,
}

impl Checker {
    pub fn check(&mut self, name: &str, ok: bool, got: i64) {
        if ok {
            self.passed += 1;
            self.text += &format!("ok {name}\n");
        } else {
            self.failed += 1;
            self.text += &format!("not ok {name} (got {got})\n");
        }
    }

    pub fn value(&mut self, name: &str, v: i64) {
        self.text += &format!("value {name} {v}\n");
    }

    pub fn finish(&mut self) -> String {
        self.text += &format!("summary {} passed, {} failed\n", self.passed, self.failed);
        self.text.clone()
    }
}

/// The kernel as the personality sees it, for one thread.
pub struct K {
    w: Arc<World>,
    task: TaskId,
    thread: ThreadId,
}

impl World {
    fn new() -> Arc<World> {
        Arc::new(World {
            mach: Mutex::new(MachState::new()),
            tasks: Mutex::new(BTreeMap::new()),
            threads: Mutex::new(BTreeMap::new()),
            cv: Condvar::new(),
            procs: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
            start: Instant::now(),
            out: Mutex::new(Checker::default()),
        })
    }

    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn now_ns(&self) -> u64 {
        self.start.elapsed().as_nanos() as u64
    }

    fn new_task(self: &Arc<World>, regions: Vec<FRegion>, next_map: u64) -> K {
        let (task, thread) = (self.id(), self.id());
        self.mach.lock().unwrap().ipc.create_task(task, thread);
        self.tasks.lock().unwrap().insert(task, Task { regions, pages: BTreeMap::new(), next_map });
        self.threads.lock().unwrap().insert(thread, Slot::default());
        K { w: self.clone(), task, thread }
    }

    fn end_task(&self, task: TaskId, thread: ThreadId) {
        let woken = {
            let mut m = self.mach.lock().unwrap();
            m.ulock.cancel(thread);
            m.psynch.cancel(thread);
            let mut w = m.ipc.destroy_thread(thread);
            w.extend(m.ipc.destroy_task(task));
            w
        };
        self.tasks.lock().unwrap().remove(&task);
        let mut th = self.threads.lock().unwrap();
        for t in woken {
            if let Some(s) = th.get_mut(&t) {
                s.unblocked = true;
            }
        }
        self.cv.notify_all();
    }
}

fn region_view(r: &FRegion) -> Region {
    Region { start: r.start, end: r.end, prot: r.prot, max_prot: r.max, backing: r.backing }
}

impl K {
    fn access(
        &self,
        addr: u64,
        buf: &mut [u8],
        write: Option<&[u8]>,
        need: u32,
        fault_in: bool,
    ) -> Result<bool, Fault> {
        let mut tasks = self.w.tasks.lock().unwrap();
        let t = tasks.get_mut(&self.task).ok_or(Fault)?;
        let len = buf.len().max(write.map_or(0, <[u8]>::len)) as u64;
        let mut at = 0u64;
        while at < len {
            let a = addr.checked_add(at).ok_or(Fault)?;
            let i = t.find(a).ok_or(Fault)?;
            let r = &t.regions[i];
            if r.prot & need != need {
                return Err(Fault);
            }
            let page = a & !(PS - 1);
            let n = (r.end.min(page + PS) - a).min(len - at);
            let p = t.pages.entry(page).or_default();
            if !p.present && !fault_in {
                return Ok(false);
            }
            p.present = true;
            p.referenced = true;
            let o = (r.off + a - r.start) as usize;
            let mut obj = r.obj.lock().unwrap();
            if obj.len() < o + n as usize {
                obj.resize(o + n as usize, 0);
            }
            match write {
                Some(src) => {
                    obj[o..o + n as usize].copy_from_slice(&src[at as usize..(at + n) as usize]);
                    t.pages.get_mut(&page).unwrap().modified = true;
                }
                None => buf[at as usize..(at + n) as usize].copy_from_slice(&obj[o..o + n as usize]),
            }
            at += n;
        }
        Ok(true)
    }
}

impl XnuKernel for K {
    fn task(&self) -> TaskId {
        self.task
    }
    fn thread(&self) -> ThreadId {
        self.thread
    }
    fn arch(&self) -> Arch {
        Arch::Arm64
    }
    fn page_size(&self) -> u64 {
        PS
    }
    fn copy_in(&self, addr: u64, buf: &mut [u8]) -> Result<(), Fault> {
        self.access(addr, buf, None, 1, true).map(|_| ())
    }
    fn copy_out(&mut self, addr: u64, buf: &[u8]) -> Result<(), Fault> {
        self.access(addr, &mut [], Some(buf), 2, true).map(|_| ())
    }
    fn load_resident(&self, addr: u64, buf: &mut [u8]) -> Result<bool, Fault> {
        self.access(addr, buf, None, 1, false)
    }
    fn shared_key(&self, addr: u64) -> Result<ulock::Key, Fault> {
        let tasks = self.w.tasks.lock().unwrap();
        let t = tasks.get(&self.task).ok_or(Fault)?;
        let r = &t.regions[t.find(addr).ok_or(Fault)?];
        Ok(match r.backing {
            Backing::Anonymous { shared: true } | Backing::File { shared: true, .. } => {
                ulock::Key::Shared { object: r.obj_id, offset: r.off + addr - r.start }
            }
            _ => ulock::Key::Private { task: self.task, addr },
        })
    }
    fn now_ns(&self) -> u64 {
        self.w.now_ns()
    }
    fn block(&mut self, deadline_ns: Option<u64>) -> Wakeup {
        let mut th = self.w.threads.lock().unwrap();
        loop {
            let s = th.get_mut(&self.thread).expect("thread slot");
            if s.signal.is_some() {
                return Wakeup::Signal;
            }
            if s.unblocked {
                s.unblocked = false;
                return Wakeup::Woken;
            }
            match deadline_ns {
                None => th = self.w.cv.wait(th).unwrap(),
                Some(d) => {
                    let now = self.w.now_ns();
                    if now >= d {
                        return Wakeup::TimedOut;
                    }
                    th = self.w.cv.wait_timeout(th, Duration::from_nanos(d - now)).unwrap().0;
                }
            }
        }
    }
    fn unblock(&mut self, thread: ThreadId) {
        if let Some(s) = self.w.threads.lock().unwrap().get_mut(&thread) {
            s.unblocked = true;
        }
        self.w.cv.notify_all();
    }
    fn with_mach(&mut self, f: &mut dyn FnMut(&mut MachState, &mut dyn XnuKernel)) {
        let w = self.w.clone();
        let mut m = w.mach.lock().unwrap();
        f(&mut m, self);
    }
    fn sender_info(&self) -> SenderInfo {
        SenderInfo::default()
    }
    fn kill_self(&mut self, signal: u32) {
        self.w.threads.lock().unwrap().get_mut(&self.thread).unwrap().signal = Some(signal);
    }
    fn regions(&self, start: u64, end: u64) -> Vec<Region> {
        let tasks = self.w.tasks.lock().unwrap();
        tasks[&self.task].regions.iter().filter(|r| r.start < end && r.end > start).map(region_view).collect()
    }
    fn map_anon(&mut self, len: u64, fixed: Option<u64>, overwrite: bool) -> Result<u64, Fault> {
        if len > 1 << 46 {
            return Err(Fault);
        }
        let id = self.w.id();
        let mut tasks = self.w.tasks.lock().unwrap();
        let t = tasks.get_mut(&self.task).unwrap();
        let start = match fixed {
            Some(a) => {
                let end = a.checked_add(len).ok_or(Fault)?;
                if t.regions.iter().any(|r| r.start < end && r.end > a) {
                    if !overwrite {
                        return Err(Fault);
                    }
                    let gone = t.range(a, end);
                    for i in gone.into_iter().rev() {
                        t.regions.remove(i);
                    }
                }
                a
            }
            None => {
                let a = t.next_map;
                t.next_map += len + PS;
                a
            }
        };
        let r = FRegion {
            start,
            end: start + len,
            prot: 3,
            max: 7,
            backing: Backing::Anonymous { shared: false },
            inherit: Inherit::Copy,
            obj: Arc::new(Mutex::new(vec![0; len as usize])),
            obj_id: id,
            off: 0,
            locked: false,
        };
        let pos = t.regions.iter().position(|x| x.start > start).unwrap_or(t.regions.len());
        t.regions.insert(pos, r);
        Ok(start)
    }
    fn unmap(&mut self, start: u64, end: u64) {
        let mut tasks = self.w.tasks.lock().unwrap();
        let t = tasks.get_mut(&self.task).unwrap();
        for i in t.range(start, end).into_iter().rev() {
            t.regions.remove(i);
        }
        let pages: Vec<u64> = t.pages.range(start..end).map(|(&p, _)| p).collect();
        for p in pages {
            t.pages.remove(&p);
        }
    }
    fn protect(&mut self, start: u64, end: u64, prot: u32) {
        self.each(start, end, |r| r.prot = prot);
    }
    fn set_max_prot(&mut self, start: u64, end: u64, max: u32) {
        self.each(start, end, |r| {
            r.max = max;
            r.prot &= max;
        });
    }
    fn set_inherit(&mut self, start: u64, end: u64, inherit: Inherit) {
        self.each(start, end, |r| r.inherit = inherit);
    }
    fn zero(&mut self, start: u64, end: u64) {
        self.each(start, end, |r| {
            let mut o = r.obj.lock().unwrap();
            let (a, b) = (r.off as usize, (r.off + r.end - r.start) as usize);
            if o.len() < b {
                o.resize(b, 0);
            }
            o[a..b].fill(0);
        });
    }
    fn set_locked(&mut self, start: u64, end: u64, locked: bool) {
        self.each(start, end, |r| r.locked = locked);
    }
    fn sync(&mut self, _start: u64, _end: u64, _wait: bool) {}
    fn page_info(&self, addr: u64) -> PageInfo {
        let tasks = self.w.tasks.lock().unwrap();
        let p = tasks[&self.task].pages.get(&(addr & !(PS - 1))).copied().unwrap_or_default();
        PageInfo { resident: p.present, referenced: p.referenced, modified: p.modified }
    }
    fn thread_entry_registered(&self) -> bool {
        true
    }
    fn create_thread(&mut self, start: ThreadStart) -> Result<u64, u32> {
        Ok(start.pthread)
    }
}

impl K {
    fn each(&mut self, start: u64, end: u64, mut f: impl FnMut(&mut FRegion)) {
        let mut tasks = self.w.tasks.lock().unwrap();
        let t = tasks.get_mut(&self.task).unwrap();
        for i in t.range(start, end) {
            f(&mut t.regions[i]);
        }
    }
}

/// The result of a BSD call, as the test header has it.
#[derive(Clone, Copy, Debug)]
pub struct SysRet {
    pub ret: i64,
    pub err: i64,
}

/// A simulated process, as a test program sees it.
pub struct P {
    pub k: K,
    data_next: u64,
}

pub const DATA: u64 = 0x8000_0000;
const MAP_BASE: u64 = 0x1_0000_0000;

impl P {
    fn killed(&self) -> Option<u32> {
        self.k.w.threads.lock().unwrap().get(&self.k.thread).and_then(|s| s.signal)
    }

    fn after_call(&self) {
        if let Some(sig) = self.killed() {
            std::panic::panic_any(Killed(sig));
        }
    }

    pub fn mach(&mut self, n: u64, args: &[u64]) -> u64 {
        let mut a = [0u64; 8];
        a[..args.len()].copy_from_slice(args);
        let r = call(&mut self.k, Class::Mach, n, &a);
        self.after_call();
        match r {
            Some(Ret::Mach(v)) => v,
            other => panic!("mach trap {n}: {other:?}"),
        }
    }

    /// A Mach trap's 32-bit kern_return_t.
    pub fn kr(&mut self, n: u64, args: &[u64]) -> u32 {
        self.mach(n, args) as u32
    }

    pub fn bsd(&mut self, n: u64, args: &[u64]) -> SysRet {
        let mut a = [0u64; 8];
        a[..args.len()].copy_from_slice(args);
        let r = call(&mut self.k, Class::Bsd, n, &a);
        self.after_call();
        match r {
            Some(Ret::Ok(v)) => SysRet { ret: v as i64, err: 0 },
            Some(Ret::Err(e)) => SysRet { ret: i64::from(e), err: i64::from(e) },
            other => panic!("bsd call {n}: {other:?}"),
        }
    }

    // ----- output -----

    pub fn check(&self, name: &str, ok: bool, got: i64) {
        self.k.w.out.lock().unwrap().check(name, ok, got);
    }
    pub fn check_ok(&self, name: &str, r: SysRet) {
        self.check(name, r.err == 0, r.err);
    }
    pub fn check_err(&self, name: &str, r: SysRet, want: i64) {
        self.check(name, r.err == want, r.err);
    }
    pub fn value(&self, name: &str, v: i64) {
        self.k.w.out.lock().unwrap().value(name, v);
    }

    // ----- memory -----

    /// A variable of the program: `n` bytes in its data area.
    pub fn var(&mut self, n: u64) -> u64 {
        let a = (self.data_next + 7) & !7;
        self.data_next = a + n;
        a
    }

    fn user(&self, addr: u64, buf: &mut [u8], write: Option<&[u8]>) {
        let need = if write.is_some() { 2 } else { 1 };
        if self.k.access(addr, buf, write, need, true).is_err() {
            let mapped = {
                let tasks = self.k.w.tasks.lock().unwrap();
                tasks[&self.k.task].find(addr).is_some()
            };
            std::panic::panic_any(Killed(poly_personality_xnu::kernel::fault_signal(mapped)));
        }
    }

    pub fn peek(&self, addr: u64) -> u8 {
        let mut b = [0u8];
        self.user(addr, &mut b, None);
        b[0]
    }
    pub fn poke(&self, addr: u64, v: u8) {
        self.user(addr, &mut [], Some(&[v]));
    }
    pub fn peek32(&self, addr: u64) -> u32 {
        let mut b = [0u8; 4];
        self.user(addr, &mut b, None);
        u32::from_le_bytes(b)
    }
    pub fn poke32(&self, addr: u64, v: u32) {
        self.user(addr, &mut [], Some(&v.to_le_bytes()));
    }
    pub fn peek64(&self, addr: u64) -> u64 {
        let mut b = [0u8; 8];
        self.user(addr, &mut b, None);
        u64::from_le_bytes(b)
    }
    pub fn poke64(&self, addr: u64, v: u64) {
        self.user(addr, &mut [], Some(&v.to_le_bytes()));
    }

    /// mmap of anonymous memory (the test header's `anon`).
    pub fn anon(&mut self, len: u64, prot: u64) -> u64 {
        let a = self.k.map_anon(len.div_ceil(PS) * PS, None, false).unwrap();
        self.k.protect(a, a + len.div_ceil(PS) * PS, prot as u32);
        a
    }

    /// mmap(MAP_ANON | MAP_SHARED).
    pub fn shared_anon(&mut self, len: u64) -> u64 {
        let a = self.anon(len, PROT_READ | PROT_WRITE);
        self.k.each(a, a + len, |r| {
            r.backing = Backing::Anonymous { shared: true };
            r.inherit = Inherit::Share;
        });
        a
    }

    /// mmap(MAP_SHARED) of a file opened read-write.
    pub fn shared_file(&mut self, len: u64) -> u64 {
        let a = self.anon(len, PROT_READ | PROT_WRITE);
        self.k.each(a, a + len, |r| {
            r.backing = Backing::File { shared: true, writable: true };
            r.inherit = Inherit::Share;
        });
        a
    }

    pub fn munmap(&mut self, addr: u64, len: u64) {
        self.k.unmap(addr, addr + len.div_ceil(PS) * PS);
    }

    // ----- time -----

    /// mach_absolute_time on arm64: the 24 MHz counter.
    pub fn now(&self) -> u64 {
        Arch::Arm64.ns_to_abs(self.k.w.now_ns())
    }

    pub fn nap_ms(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    // ----- processes -----

    /// fork: the child runs `f` and exits with its result (or dies of a
    /// signal); returns the child's pid.
    pub fn fork(&mut self, f: impl FnOnce(&mut P) -> i32 + Send + 'static) -> u64 {
        let w = self.k.w.clone();
        let (regions, next_map) = {
            let tasks = w.tasks.lock().unwrap();
            let t = &tasks[&self.k.task];
            let mut out = Vec::new();
            for r in &t.regions {
                let mut c = r.clone();
                match r.inherit {
                    Inherit::None => continue,
                    Inherit::Share => {}
                    Inherit::Copy => {
                        let data = r.obj.lock().unwrap().clone();
                        c.obj = Arc::new(Mutex::new(data));
                        c.obj_id = w.id();
                        if let Backing::Anonymous { shared: true } = c.backing {
                            c.backing = Backing::Anonymous { shared: false };
                        }
                    }
                }
                out.push(c);
            }
            (out, t.next_map)
        };
        let k = w.new_task(regions, next_map);
        let (task, thread) = (k.task, k.thread);
        let status = Arc::new(Mutex::new(None));
        let st = status.clone();
        let data_next = self.data_next;
        let w2 = w.clone();
        let handle = std::thread::spawn(move || {
            let mut child = P { k, data_next };
            let r = catch_unwind(AssertUnwindSafe(|| f(&mut child)));
            let s = match r {
                Ok(code) => (code & 0xff) << 8,
                Err(e) => match e.downcast::<Killed>() {
                    Ok(k) => k.0 as i32,
                    Err(e) => std::panic::resume_unwind(e),
                },
            };
            *st.lock().unwrap() = Some(s);
            w2.end_task(task, thread);
        });
        w.procs.lock().unwrap().insert(task, Proc { thread, status, handle: Some(handle) });
        task
    }

    /// wait4 for `pid`: its status.
    pub fn wait(&mut self, pid: u64) -> i32 {
        let h = self.k.w.procs.lock().unwrap().get_mut(&pid).and_then(|p| p.handle.take());
        if let Some(h) = h {
            h.join().expect("child thread");
        }
        self.k.w.procs.lock().unwrap()[&pid].status.lock().unwrap().expect("ended")
    }

    /// wait4(WNOHANG): `None` while the child runs.
    pub fn wait_nohang(&self, pid: u64) -> Option<i32> {
        *self.k.w.procs.lock().unwrap()[&pid].status.lock().unwrap()
    }

    pub fn kill(&self, pid: u64, sig: u32) {
        let thread = self.k.w.procs.lock().unwrap()[&pid].thread;
        if let Some(s) = self.k.w.threads.lock().unwrap().get_mut(&thread) {
            s.signal = Some(sig);
        }
        self.k.w.cv.notify_all();
    }
}

pub fn w_exited(s: i32) -> bool {
    s & 0o177 == 0
}
pub fn w_exitstatus(s: i32) -> i32 {
    (s >> 8) & 0xff
}
pub fn w_termsig(s: i32) -> i32 {
    s & 0o177
}

/// The arm64 output recorded on macOS for `test` in `dir`
/// (`specs/macos/<dir>/macos.json`).
pub fn recorded(dir: &str, test: &str) -> String {
    let path = format!("{}/../../specs/macos/{dir}/macos.json", env!("CARGO_MANIFEST_DIR"));
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let at = json.find(&format!("\"name\": \"{test}\"")).unwrap_or_else(|| panic!("{test} not in {path}"));
    let rest = &json[at..];
    let s = rest.find("\"stdout\": \"").expect("stdout") + "\"stdout\": \"".len();
    let mut out = String::new();
    let mut chars = rest[s..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(c) => out.push(c),
                None => break,
            },
            c => out.push(c),
        }
    }
    out
}

static HOOK: Once = Once::new();

/// Run a transliterated test as a fresh process and compare its output with
/// the recording.
pub fn run(dir: &str, test: &str, body: impl FnOnce(&mut P)) {
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if info.payload().downcast_ref::<Killed>().is_none() {
                prev(info);
            }
        }));
    });
    let w = World::new();
    let k = w.new_task(Vec::new(), MAP_BASE);
    let mut p = P { k, data_next: DATA };
    // The program's data area (its variables).
    {
        let mut tasks = w.tasks.lock().unwrap();
        let t = tasks.get_mut(&p.k.task).unwrap();
        t.regions.push(FRegion {
            start: DATA,
            end: DATA + 16 * PS,
            prot: 3,
            max: 7,
            backing: Backing::Anonymous { shared: false },
            inherit: Inherit::Copy,
            obj: Arc::new(Mutex::new(vec![0; (16 * PS) as usize])),
            obj_id: w.id(),
            off: 0,
            locked: false,
        });
    }
    body(&mut p);
    let got = w.out.lock().unwrap().finish();
    let want = recorded(dir, test);
    assert_eq!(got, want, "{dir}/{test}: output differs from the macOS recording");
}
