//! The stack, the program break and page faults.

use super::*;

impl<F: Clone + PartialEq> VmMap<F> {
    /// Create the stack region: the top `size` bytes below
    /// [`Layout::stack_top`], private anonymous memory with protection
    /// `prot` (read-write, plus execute for programs that ask for an
    /// executable stack). Returns its lowest address.
    ///
    /// Errors: `size` zero, misaligned, above [`Layout::stack_max`] or
    /// reaching below [`Layout::min_addr`] ([`Error::Invalid`]); the range is
    /// already mapped ([`Error::Exists`]); too many regions
    /// ([`Error::NoMemory`]).
    pub fn setup_stack(&mut self, size: u64, prot: Prot) -> Result<u64, Error> {
        let top = self.layout.stack_top;
        if size == 0 || !is_page_aligned(size) || size > self.layout.stack_max || size > top - self.layout.min_addr {
            return Err(Error::Invalid);
        }
        let start = top - size;
        if self.regions_in(start, top).next().is_some() {
            return Err(Error::Exists);
        }
        let vma = Vma {
            start,
            end: top,
            prot,
            max_prot: Prot::ALL,
            kind: Kind::Stack,
            backing: Backing::Anonymous,
            wipe_on_fork: false,
            locked: self.lock_future.is_some(),
            sealed: false,
            uffd: 0,
            name: None,
        };
        self.rewrite(start, top, Op::Insert(vma))?;
        Ok(start)
    }

    /// Set the start of the program break (the end of the program's data,
    /// possibly randomised by the kernel). The heap region begins at the
    /// next page boundary. Fails with [`Error::Invalid`] outside the
    /// address space.
    pub fn init_brk(&mut self, start: u64) -> Result<(), Error> {
        if start < self.layout.min_addr || start >= self.layout.max_addr {
            return Err(Error::Invalid);
        }
        self.brk = Some(BrkState { start, current: start });
        Ok(())
    }

    /// The initial program break, if [`VmMap::init_brk`] was called.
    pub fn brk_start(&self) -> Option<u64> {
        self.brk.map(|b| b.start)
    }

    /// Move the program break to `new` (`brk`).
    ///
    /// Growing maps the new pages as a private read-write heap region, but
    /// only if they are free (also of the stack's guard gap), inside the
    /// address space and within [`Layout::heap_max`]. Shrinking unmaps the
    /// whole pages above the new break. A break below the start (including
    /// `brk(0)`, the usual way to ask for the current break) changes nothing.
    /// Without [`VmMap::init_brk`] the break is 0 and cannot move.
    pub fn brk(&mut self, new: u64) -> Brk<F> {
        let Some(state) = self.brk else {
            return Brk { brk: 0, removed: Vec::new() };
        };
        let unchanged = Brk { brk: state.current, removed: Vec::new() };
        if new < state.start || new - state.start > self.layout.heap_max {
            return unchanged;
        }
        let (Some(old_end), Some(new_end)) = (page_up(state.current), page_up(new)) else {
            return unchanged;
        };
        if new_end > self.layout.max_addr {
            return unchanged;
        }
        let removed = if new_end < old_end {
            match self.rewrite(new_end, old_end, Op::Remove) {
                Ok(removed) => removed,
                Err(_) => return unchanged,
            }
        } else {
            if new_end > old_end {
                if !self.is_free(old_end, new_end) {
                    return unchanged;
                }
                let vma = Vma {
                    start: old_end,
                    end: new_end,
                    prot: Prot::RW,
                    max_prot: Prot::ALL,
                    kind: Kind::Heap,
                    backing: Backing::Anonymous,
                    wipe_on_fork: false,
                    locked: self.lock_future.is_some(),
                    sealed: false,
                    uffd: 0,
                    name: None,
                };
                if self.rewrite(old_end, new_end, Op::Insert(vma)).is_err() {
                    return unchanged;
                }
            }
            Vec::new()
        };
        self.brk = Some(BrkState { current: new, ..state });
        Brk { brk: new, removed }
    }

    /// Decide what a user page fault at `addr` means.
    ///
    /// The kernel's page-fault handler, for a fault on a user address (from
    /// user mode, or from the kernel's user-copy routines), with the address
    /// space locked:
    ///
    /// 1. read the faulting address (CR2) and turn the error code into an
    ///    [`Access`];
    /// 2. look at the page's entry: [`PageState::Absent`] if no frame is
    ///    behind it, otherwise [`PageState::Present`] with the frame's
    ///    reference count;
    /// 3. call this and carry out the [`FaultAction`], then return to the
    ///    faulting instruction (or deliver the signal / return `EFAULT`).
    ///
    /// The rules: an address in no region is a `SEGV_MAPERR`, unless it lies
    /// just below a stack region, above [`Layout::stack_floor`] and at least
    /// the guard gap above the region below; then the stack grows down to
    /// the page (this call changes the map) and the fault is handled as one
    /// in the stack. An access the region's protection does not allow (see
    /// [`Prot::allows`]) is a `SEGV_ACCERR`. Otherwise an absent page is
    /// zero-filled or read from the file, a write to a present private page
    /// whose frame is shared is copied, and anything else only needs the
    /// entry's permissions brought up to date.
    pub fn fault(&mut self, addr: u64, access: Access, page: PageState) -> FaultAction<F> {
        let page_addr = page_down(addr);
        if addr < self.layout.min_addr || addr >= self.layout.max_addr {
            return FaultAction::Segv(SegvCode::MapErr);
        }
        let key = match self.find(addr) {
            Some(v) => v.start,
            None => match self.grow_stack(page_addr, access) {
                Ok(key) => key,
                Err(code) => return FaultAction::Segv(code),
            },
        };
        let vma = &self.vmas[&key];
        if !vma.prot.allows(access) {
            return FaultAction::Segv(SegvCode::AccErr);
        }
        if vma.uffd != 0 && page == PageState::Absent {
            return FaultAction::UserFault { page: page_addr, uffd: vma.uffd };
        }
        match page {
            PageState::Absent => match &vma.backing {
                Backing::Anonymous => FaultAction::ZeroFill { page: page_addr, prot: vma.prot },
                Backing::File { file, offset, shared } => {
                    let private_copy = !shared && access == Access::Write;
                    let prot = if *shared || private_copy { vma.prot } else { write_protected(vma.prot) };
                    let offset = offset + (page_addr - vma.start);
                    FaultAction::ReadFile { page: page_addr, file: file.clone(), offset, prot, private_copy }
                }
            },
            PageState::Present { refs } => {
                let prot = vma.page_prot(refs);
                if access == Access::Write && !prot.contains(Prot::WRITE) {
                    FaultAction::CopyOnWrite { page: page_addr, prot: vma.prot }
                } else {
                    FaultAction::SetPermissions { page: page_addr, prot }
                }
            }
        }
    }

    /// Decide what a forced write at `addr` means: a debugger's write
    /// (ptrace(2) `PTRACE_POKEDATA`) into a private region, whatever its
    /// protection. The page becomes the region's own copy, mapped with the
    /// region's protection (a read-only page stays read-only for the
    /// process); the caller then writes it through the kernel. A shared
    /// region that does not allow writing is a `SEGV_ACCERR` (its pages
    /// are the file's or the other users'); an address in no region a
    /// `SEGV_MAPERR`. The stack does not grow.
    pub fn fault_forced(&mut self, addr: u64, page: PageState) -> FaultAction<F> {
        let page_addr = page_down(addr);
        let Some(vma) = self.find(addr) else { return FaultAction::Segv(SegvCode::MapErr) };
        if vma.prot.allows(Access::Write) {
            return self.fault(addr, Access::Write, page);
        }
        if vma.is_shared() {
            return FaultAction::Segv(SegvCode::AccErr);
        }
        match page {
            PageState::Absent => match &vma.backing {
                Backing::Anonymous => FaultAction::ZeroFill { page: page_addr, prot: vma.prot },
                Backing::File { file, offset, .. } => {
                    let offset = offset + (page_addr - vma.start);
                    FaultAction::ReadFile {
                        page: page_addr,
                        file: file.clone(),
                        offset,
                        prot: vma.prot,
                        private_copy: true,
                    }
                }
            },
            PageState::Present { refs } if refs > 1 => FaultAction::CopyOnWrite { page: page_addr, prot: vma.prot },
            PageState::Present { refs } => FaultAction::SetPermissions { page: page_addr, prot: vma.page_prot(refs) },
        }
    }

    /// Grow the stack region above `page` down to it, if the rules allow.
    /// Returns the region's new start (its key).
    pub(super) fn grow_stack(&mut self, page: u64, access: Access) -> Result<u64, SegvCode> {
        let above = self.vmas.range(page..).next().map(|(_, v)| v);
        let Some(stack) = above.filter(|v| v.kind == Kind::Stack) else {
            return Err(SegvCode::MapErr);
        };
        if page < self.layout.stack_floor() {
            return Err(SegvCode::MapErr);
        }
        let below = self.vmas.range(..page).next_back().map(|(_, v)| v);
        if below.is_some_and(|b| b.end.saturating_add(self.layout.stack_guard_gap) > page) {
            return Err(SegvCode::MapErr);
        }
        if !stack.prot.allows(access) {
            return Err(SegvCode::AccErr);
        }
        let old = stack.start;
        let mut grown = self.vmas.remove(&old).expect("the stack region is in the map");
        grown.start = page;
        // With a zero guard gap the stack can reach a compatible region below.
        if let Some(below) = self.vmas.range_mut(..page).next_back().map(|(_, v)| v)
            && below.can_merge(&grown)
        {
            below.end = grown.end;
            return Ok(below.start);
        }
        self.vmas.insert(page, grown);
        Ok(page)
    }
}
