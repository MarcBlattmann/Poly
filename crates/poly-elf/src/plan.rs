//! Turning `PT_LOAD` segments into page mappings.
//!
//! The gABI ("Program Loading") describes a segment as `p_filesz` bytes of
//! the file at `p_offset`, placed at `p_vaddr`, followed by zeros up to
//! `p_memsz`. Mapped page by page, that becomes:
//!
//! ```text
//!   page_floor(p_vaddr)        p_vaddr + p_filesz     p_vaddr + p_memsz
//!   |                          |                      |
//!   [ file-backed pages  ......|0000 zero tail]       |
//!                                              [anonymous zero pages ]
//!   ^ starts with the bytes before p_vaddr      ^ page_ceil(p_vaddr + p_filesz)
//!     in the same file page
//! ```
//!
//! A file-backed [`Mapping`] holds file bytes from the page-aligned file
//! offset up to `p_offset + p_filesz` and zeros after that, to the end of the
//! page. If `p_memsz` reaches further, an anonymous mapping follows. The
//! zero tail is applied even when `p_memsz == p_filesz`: the ABI leaves those
//! bytes unspecified, and zeros leak nothing from the file past the segment.

use crate::header::{ElfType, Error, PF_R, PF_W, PF_X, PHDR_SIZE, ProgramHeader, Tls};
use crate::{Elf, MAX_LOAD_SEGMENTS, PAGE_SIZE, align_down, align_up};

/// Default address of the first page of an `ET_DYN` image: two thirds of the
/// 47-bit user address space, rounded down to a page. Linux places
/// position-independent executables here when address space randomisation is
/// off (`setarch -R`), as `/proc/self/maps` shows; matching it keeps traces
/// from both systems comparable. It is raised to the image's largest
/// `p_align`. An interpreter needs a different base; see
/// [`LoadOptions::dyn_base`].
pub const DEFAULT_DYN_BASE: u64 = 0x5555_5555_4000;

/// Lowest address a segment may occupy by default: 64 KiB, the usual
/// default of Linux's `vm.mmap_min_addr` (see mmap(2) and proc(5)), so a
/// NULL pointer dereference with a small offset always faults.
pub const DEFAULT_MIN_ADDR: u64 = 0x1_0000;

/// End (exclusive) of the user address space by default: the canonical
/// lower half of a 48-bit address space minus its last page. Keeping that
/// page unmapped means code cannot run off the end of the lower half, so the
/// address after a `syscall` instruction is always canonical, which `sysret`
/// requires (Intel SDM Vol. 2B, SYSRET).
pub const USER_END: u64 = 0x0000_8000_0000_0000 - PAGE_SIZE;

/// Most mappings a plan holds: a file-backed and an anonymous one per segment.
pub const MAX_MAPPINGS: usize = 2 * MAX_LOAD_SEGMENTS;

/// Page protection of a mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Prot(u8);

impl Prot {
    pub const NONE: Prot = Prot(0);
    pub const READ: Prot = Prot(1);
    pub const WRITE: Prot = Prot(2);
    pub const EXEC: Prot = Prot(4);

    /// Protection from a program header's `p_flags` (`PF_R`, `PF_W`, `PF_X`).
    pub const fn from_flags(flags: u32) -> Prot {
        let mut bits = 0;
        if flags & PF_R != 0 {
            bits |= Prot::READ.0;
        }
        if flags & PF_W != 0 {
            bits |= Prot::WRITE.0;
        }
        if flags & PF_X != 0 {
            bits |= Prot::EXEC.0;
        }
        Prot(bits)
    }

    pub const fn union(self, other: Prot) -> Prot {
        Prot(self.0 | other.0)
    }

    pub const fn contains(self, other: Prot) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn readable(self) -> bool {
        self.contains(Prot::READ)
    }

    pub const fn writable(self) -> bool {
        self.contains(Prot::WRITE)
    }

    pub const fn executable(self) -> bool {
        self.contains(Prot::EXEC)
    }
}

/// Things a plan allows but a hardened system would not: W^X violations
/// and similar. The loader reports them; the kernel decides what to log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Warnings(u8);

impl Warnings {
    pub const NONE: Warnings = Warnings(0);
    /// A `PT_LOAD` segment is both writable and executable.
    pub const WRITABLE_AND_EXECUTABLE: Warnings = Warnings(1);
    /// `PT_GNU_STACK` asks for an executable stack.
    pub const EXECUTABLE_STACK: Warnings = Warnings(2);
    /// No `PT_GNU_STACK`; the stack is made non-executable anyway.
    pub const NO_GNU_STACK: Warnings = Warnings(4);
    /// The entry point is in a segment without `PF_X`.
    pub const ENTRY_NOT_EXECUTABLE: Warnings = Warnings(8);

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Warnings) -> bool {
        self.0 & other.0 == other.0
    }

    const fn with(self, other: Warnings) -> Warnings {
        Warnings(self.0 | other.0)
    }
}

/// File bytes a mapping starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileRange {
    /// Page-aligned offset in the file of the mapping's first byte.
    pub offset: u64,
    /// Number of file bytes; everything after them in the mapping is zero.
    pub len: u64,
}

/// A page-aligned range of the new address space and what it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    /// First address, page aligned.
    pub start: u64,
    /// End address (exclusive), page aligned, greater than `start`.
    pub end: u64,
    pub prot: Prot,
    /// `Some`: the first `len` bytes come from the file at `offset`, the rest
    /// of the last page is zero. `None`: anonymous zero pages.
    pub file: Option<FileRange>,
}

impl Mapping {
    const EMPTY: Mapping = Mapping { start: 0, end: 0, prot: Prot::NONE, file: None };

    /// Size in bytes, a multiple of the page size.
    pub const fn size(&self) -> u64 {
        self.end - self.start
    }

    /// First address that must read as zero (`end` if the mapping is all file).
    pub const fn zero_start(&self) -> u64 {
        match self.file {
            Some(f) => self.start + f.len,
            None => self.start,
        }
    }

    /// The file bytes that go at `start`, for a loader that copies the
    /// image instead of mapping it page by page (empty for anonymous memory).
    pub fn file_bytes<'a>(&self, image: &'a [u8]) -> &'a [u8] {
        let Some(f) = self.file else { return &[] };
        let range = usize::try_from(f.offset)
            .ok()
            .and_then(|start| Some(start..start.checked_add(usize::try_from(f.len).ok()?)?));
        range.and_then(|r| image.get(r)).unwrap_or(&[])
    }

    /// Fills `out` (one page) with the contents of the page at `page`, for
    /// demand paging. Returns `false`, leaving `out` untouched, if `page` is
    /// not a page of this mapping, `out` is not one page long or `image` is
    /// not the file the plan was made from.
    pub fn fill_page(&self, image: &[u8], page: u64, out: &mut [u8]) -> bool {
        if !page.is_multiple_of(PAGE_SIZE) || page < self.start || page >= self.end || out.len() as u64 != PAGE_SIZE {
            return false;
        }
        let rel = page - self.start;
        let mut src: &[u8] = &[];
        if let Some(f) = self.file
            && rel < f.len
        {
            let n = (f.len - rel).min(PAGE_SIZE);
            let bytes = f.offset.checked_add(rel).and_then(|from| Some(from..from.checked_add(n)?)).and_then(|r| {
                let r = usize::try_from(r.start).ok()?..usize::try_from(r.end).ok()?;
                image.get(r)
            });
            match bytes {
                Some(b) => src = b,
                None => return false,
            }
        }
        let (head, tail) = out.split_at_mut(src.len());
        head.copy_from_slice(src);
        tail.fill(0);
        true
    }
}

/// Where an image may go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadOptions {
    /// Where the first page of an `ET_DYN` image goes (raised to its largest
    /// `p_align`). Ignored for `ET_EXEC`, which is loaded at its link-time
    /// addresses. The main executable uses [`DEFAULT_DYN_BASE`]; a program
    /// interpreter is planned a second time with a base the kernel takes
    /// from free space in the new address space (Linux puts it in the mmap
    /// area below the stack), so the two images cannot collide.
    pub dyn_base: u64,
    /// No mapping may start below this address.
    pub min_addr: u64,
    /// No mapping may end above this address.
    pub user_end: u64,
}

impl LoadOptions {
    pub const DEFAULT: LoadOptions =
        LoadOptions { dyn_base: DEFAULT_DYN_BASE, min_addr: DEFAULT_MIN_ADDR, user_end: USER_END };
}

impl Default for LoadOptions {
    fn default() -> Self {
        LoadOptions::DEFAULT
    }
}

/// Everything the kernel needs to set up the address space of one image.
#[derive(Clone, Copy, Debug)]
pub struct LoadPlan {
    mappings: [Mapping; MAX_MAPPINGS],
    count: usize,
    elf_type: ElfType,
    bias: u64,
    entry: u64,
    phdr: Option<u64>,
    phnum: u16,
    start: u64,
    end: u64,
    stack_executable: bool,
    warnings: Warnings,
    tls: Option<Tls>,
}

impl LoadPlan {
    /// Plans the mappings of `elf` within the limits of `opts`.
    pub fn new(elf: &Elf<'_>, opts: &LoadOptions) -> Result<LoadPlan, Error> {
        // Link-time page the image starts at and the address it is moved to.
        let mut loads = elf.load_segments().filter(|ph| ph.memsz != 0);
        let first = loads.next().ok_or(Error::NoLoadableSegments)?;
        let (link_base, load_base) = match elf.elf_type() {
            ElfType::Exec => (0, 0),
            ElfType::Dyn => {
                // The bias must keep every segment's p_vaddr ≡ p_offset
                // (mod p_align), so it is a multiple of the largest one.
                let align = elf.load_segments().map(|ph| ph.align).fold(PAGE_SIZE, u64::max);
                let base = align_up(opts.dyn_base, align).ok_or(Error::AddressOverflow)?;
                (align_down(first.vaddr, align), base)
            }
        };
        let reloc = |vaddr: u64| {
            vaddr.checked_sub(link_base).and_then(|o| o.checked_add(load_base)).ok_or(Error::AddressOverflow)
        };

        let mut plan = LoadPlan {
            mappings: [Mapping::EMPTY; MAX_MAPPINGS],
            count: 0,
            elf_type: elf.elf_type(),
            bias: load_base.wrapping_sub(link_base),
            entry: reloc(elf.entry())?,
            phdr: None,
            phnum: elf.phnum(),
            start: 0,
            end: 0,
            stack_executable: false,
            warnings: Warnings::NONE,
            tls: None,
        };

        for ph in elf.load_segments().filter(|ph| ph.memsz != 0) {
            plan.add_segment(&ph, &reloc)?;
            let prot = Prot::from_flags(ph.flags);
            if prot.writable() && prot.executable() {
                plan.warnings = plan.warnings.with(Warnings::WRITABLE_AND_EXECUTABLE);
            }
            if ph.contains(elf.entry(), 1) && !prot.executable() {
                plan.warnings = plan.warnings.with(Warnings::ENTRY_NOT_EXECUTABLE);
            }
        }
        let maps = plan.mappings();
        let (Some(&lowest), Some(&highest)) = (maps.first(), maps.last()) else {
            return Err(Error::NoLoadableSegments);
        };
        plan.start = lowest.start;
        plan.end = highest.end;
        if plan.start < opts.min_addr || plan.end > opts.user_end {
            return Err(Error::OutsideUserSpace);
        }

        plan.phdr = match elf.phdr_vaddr() {
            Some(v) => Some(reloc(v).map_err(|_| Error::BadPhdr)?),
            None => None,
        };
        if let Some(tls) = elf.tls() {
            plan.tls = Some(Tls { vaddr: reloc(tls.vaddr).map_err(|_| Error::BadTls)?, ..tls });
        }
        match elf.gnu_stack() {
            Some(flags) if flags & PF_X != 0 => {
                plan.stack_executable = true;
                plan.warnings = plan.warnings.with(Warnings::EXECUTABLE_STACK);
            }
            Some(_) => {}
            None => plan.warnings = plan.warnings.with(Warnings::NO_GNU_STACK),
        }
        Ok(plan)
    }

    fn push(&mut self, m: Mapping) -> Result<(), Error> {
        let slot = self.mappings.get_mut(self.count).ok_or(Error::TooManyLoadSegments)?;
        *slot = m;
        self.count += 1;
        Ok(())
    }

    /// Appends the mappings of one non-empty `PT_LOAD` segment.
    fn add_segment(&mut self, ph: &ProgramHeader, reloc: &impl Fn(u64) -> Result<u64, Error>) -> Result<(), Error> {
        let prot = Prot::from_flags(ph.flags);
        let page = align_down(ph.vaddr, PAGE_SIZE);
        let lead = ph.vaddr - page;
        // Parsing checked that both ends round up to a page without overflow.
        let file_end = ph.vaddr + ph.filesz;
        let mem_end = align_up(ph.vaddr + ph.memsz, PAGE_SIZE).ok_or(Error::AddressOverflow)?;
        let mut anon_start = page;
        if ph.filesz != 0 {
            let end = align_up(file_end, PAGE_SIZE).ok_or(Error::AddressOverflow)?;
            // Congruence was checked, so the offset has at least `lead` bytes before it.
            let file = FileRange { offset: ph.offset - lead, len: lead + ph.filesz };
            self.push(Mapping { start: reloc(page)?, end: reloc(end)?, prot, file: Some(file) })?;
            anon_start = end;
        }
        if mem_end > anon_start {
            self.push(Mapping { start: reloc(anon_start)?, end: reloc(mem_end)?, prot, file: None })?;
        }
        Ok(())
    }

    /// The mappings in ascending address order; they do not overlap.
    pub fn mappings(&self) -> &[Mapping] {
        self.mappings.get(..self.count).unwrap_or(&[])
    }

    /// The mapping containing `addr`, for the page-fault handler.
    pub fn mapping_at(&self, addr: u64) -> Option<&Mapping> {
        self.mappings().iter().find(|m| addr >= m.start && addr < m.end)
    }

    pub const fn elf_type(&self) -> ElfType {
        self.elf_type
    }

    /// Load bias: what was added to every link-time address (0 for
    /// `ET_EXEC`). For an interpreter this is the value of `AT_BASE`.
    pub const fn bias(&self) -> u64 {
        self.bias
    }

    /// Entry point after relocation (`AT_ENTRY` for the main executable).
    pub const fn entry(&self) -> u64 {
        self.entry
    }

    /// Address of the program header table in the new address space
    /// (`AT_PHDR`), or `None` if no segment maps it. In that case the
    /// table must be placed in memory separately; [`StackParams::phdr_table`](crate::StackParams::phdr_table)
    /// copies it onto the stack.
    pub const fn phdr(&self) -> Option<u64> {
        self.phdr
    }

    /// Number of program headers (`AT_PHNUM`).
    pub const fn phnum(&self) -> u16 {
        self.phnum
    }

    /// Size of one program header (`AT_PHENT`).
    pub const fn phent(&self) -> u16 {
        PHDR_SIZE as u16
    }

    /// First mapped address.
    pub const fn start(&self) -> u64 {
        self.start
    }

    /// End of the highest mapping, page aligned.
    pub const fn end(&self) -> u64 {
        self.end
    }

    /// Initial program break for `brk(2)`: the page-aligned end of the
    /// highest segment. Only meaningful for the main executable.
    pub const fn brk(&self) -> u64 {
        self.end
    }

    /// Whether the stack should be executable (`PT_GNU_STACK` with `PF_X`).
    pub const fn stack_executable(&self) -> bool {
        self.stack_executable
    }

    pub const fn warnings(&self) -> Warnings {
        self.warnings
    }

    /// The TLS template at its relocated address.
    pub const fn tls(&self) -> Option<Tls> {
        self.tls
    }
}
