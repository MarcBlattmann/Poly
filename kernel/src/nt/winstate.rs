//! The window-state page (ADR 0249): one page every Windows process that asks maps read-only,
//! whose first eight bytes are a generation the kernel raises before every change of the window
//! manager (`poly_personality_nt::wmgen`). `user32` keeps what `Window` answered for as long as the
//! generation stays the same, as Windows' `user32` reads its shared window data instead of asking
//! the kernel.
//!
//! The page is shared the way the vDSO's data page is (`crate::vdso`): a one-page file of a private
//! tmpfs, through the page cache, mapped as a shared read-only file mapping whose protection
//! ceiling is read-only. The kernel writes it through the direct map.

use crate::fs::cache::{self, CachedPage};
use crate::sync::{Mutex, SetOnce};
use crate::user::memory::{FileRef, IoError, MapRequest, Memory, PAGE_SIZE, PageSource, Placement, Prot};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};
use poly_vfs::{DeviceId, FileSystem, FileType, NewNode, Tmpfs};

/// The page as a mappable source: the page cache's page, shared.
struct PageSourceOfState {
    page: CachedPage,
}

impl PageSource for PageSourceOfState {
    fn len(&self) -> u64 {
        PAGE_SIZE
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError> {
        let start = offset.min(PAGE_SIZE) as usize;
        let n = buf.len().min(PAGE_SIZE as usize - start);
        self.page.read(start, &mut buf[..n]);
        Ok(n)
    }

    fn cached_page(&self, offset: u64) -> Option<Result<CachedPage, IoError>> {
        (offset == 0).then(|| Ok(self.page.clone()))
    }

    fn shareable(&self) -> bool {
        true
    }

    fn label(&self) -> Option<&str> {
        Some("[window state]")
    }
}

struct State {
    source: Arc<PageSourceOfState>,
    /// The generation, in the page, through the direct map.
    generation: &'static AtomicU64,
}

static STATE: SetOnce<State> = SetOnce::new();
static SETUP: Mutex<bool> = Mutex::new(false);

/// Make the page once. It may sleep (the page cache's locks): called before an NT system call
/// takes the NT state's lock.
pub(super) fn ensure() {
    if STATE.get().is_some() {
        return;
    }
    let mut tried = SETUP.lock();
    if *tried || STATE.get().is_some() {
        return;
    }
    *tried = true;
    let page = || -> Result<CachedPage, poly_vfs::Errno> {
        let fs = Arc::new(Tmpfs::new(crate::fs::clock()));
        let node = NewNode { kind: FileType::Regular, perm: 0o444, uid: 0, gid: 0, rdev: DeviceId::default() };
        let ino = fs.create(fs.root(), b"window-state", &node)?;
        fs.write(ino, 0, &[0; PAGE_SIZE as usize])?;
        cache::cache().register(fs).page(ino, 0)
    };
    let page = match page() {
        Ok(p) => p,
        Err(e) => {
            crate::kprintln!("nt: no window-state page ({}): user32 asks the kernel every time", e.name());
            return;
        }
    };
    let Some(virt) = crate::arch::phys_to_virt(page.frame().phys()) else { return };
    // SAFETY: the frame is a whole page of RAM that the page cache keeps for as long as this
    // module holds the page (forever); its first eight bytes are only ever used as this atomic,
    // and user mappings of it are read-only.
    let generation = unsafe { &*(virt as *const AtomicU64) };
    generation.store(1, Ordering::SeqCst);
    let _ = STATE.set(State { source: Arc::new(PageSourceOfState { page }), generation });
}

/// The window manager is about to change: raise the generation (the hook of
/// `poly_personality_nt::wmgen::Tracked`).
pub(super) fn changed() {
    if let Some(s) = STATE.get() {
        s.generation.fetch_add(1, Ordering::SeqCst);
    }
}

/// Map the page at `addr` of `memory`, replacing the page committed there; read-only for good.
pub(super) fn map_into(memory: &Memory, addr: u64) -> Result<(), ()> {
    let s = STATE.get().ok_or(())?;
    let source: Arc<dyn PageSource> = s.source.clone();
    let req = MapRequest::file(PAGE_SIZE, Prot::READ, FileRef::new(source), 0, true)
        .with_max_prot(Prot::READ)
        .at(Placement::Fixed(addr));
    memory.map(req).map(drop).map_err(drop)
}
