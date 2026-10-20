//! Loading a program interpreter next to the program (ADR 0049).
//!
//! A dynamically linked executable names its interpreter (the dynamic
//! linker, `ld.so`) in `PT_INTERP`. The gABI ("Program Interpreter") says
//! the system then loads the interpreter as well and hands it control; the
//! interpreter is "a shared object" that the system may load at an address
//! of its choosing (or, rarely, an executable at its fixed addresses), and
//! it finds the program through the auxiliary vector: `AT_PHDR`,
//! `AT_PHENT`, `AT_PHNUM` and `AT_ENTRY` describe the program, `AT_BASE`
//! is the interpreter's own load address (getauxval(3)).
//!
//! The kernel does this in three steps:
//!
//! 1. [`check_interpreter`]: the interpreter must be a loadable image that
//!    does not name an interpreter itself (execve(2): `ELIBBAD`, "an ELF
//!    interpreter was not in a recognized format").
//! 2. [`Footprint::of`]: how much address space an `ET_DYN` interpreter
//!    spans and how its base must be aligned. The kernel reserves
//!    [`Footprint::reservation`] bytes wherever its `mmap` search finds
//!    room and takes the base with [`Footprint::base_in`], so the
//!    interpreter lands in free space whatever its size and alignment.
//! 3. [`LoadPlan::new`](crate::LoadPlan::new) with
//!    [`interpreter_options`] at that base, then
//!    [`AuxInfo::new`](crate::AuxInfo::new) and
//!    [`start_address`](crate::start_address) with both plans.

use crate::header::{ElfType, Error};
use crate::{Elf, LoadOptions, PAGE_SIZE, align_down, align_up};

/// The address space an image needs: its span from the page of its lowest
/// `PT_LOAD` segment to the page-rounded end of its highest, and the
/// alignment its load bias must keep (the largest `p_align`, at least a
/// page; gABI: "p_vaddr should equal p_offset, modulo p_align").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Footprint {
    /// Bytes from the aligned first segment to the end of the last, a
    /// multiple of the page size.
    pub size: u64,
    /// Alignment of the base, a power of two and at least a page.
    pub align: u64,
}

impl Footprint {
    /// The footprint of `elf`'s non-empty `PT_LOAD` segments, measured
    /// from the first one's address rounded down to [`align`](Self::align)
    /// (which is where [`LoadPlan`](crate::LoadPlan) puts the base).
    pub fn of(elf: &Elf<'_>) -> Result<Footprint, Error> {
        let align = elf.load_segments().map(|ph| ph.align).fold(PAGE_SIZE, u64::max);
        let mut loads = elf.load_segments().filter(|ph| ph.memsz != 0);
        let first = loads.next().ok_or(Error::NoLoadableSegments)?;
        // Parsing sorted the segments and checked that their ends round up.
        let last = loads.last().unwrap_or(first);
        let end = align_up(last.mem_end(), PAGE_SIZE).ok_or(Error::AddressOverflow)?;
        let size = end.checked_sub(align_down(first.vaddr, align)).ok_or(Error::AddressOverflow)?;
        Ok(Footprint { size, align })
    }

    /// Bytes to reserve so that a base aligned to [`align`](Self::align)
    /// fits with the whole image inside the reservation, wherever the
    /// reservation (page aligned) starts. `None` on overflow.
    pub const fn reservation(&self) -> Option<u64> {
        match self.size.checked_add(self.align) {
            Some(n) => Some(n - PAGE_SIZE),
            None => None,
        }
    }

    /// The base for an image placed in a reservation starting at `hole`
    /// (page aligned): `hole` rounded up to the alignment. `None` on
    /// overflow.
    pub const fn base_in(&self, hole: u64) -> Option<u64> {
        align_up(hole, self.align)
    }
}

/// Checks that `elf` can serve as a program interpreter: it must not name
/// an interpreter of its own ([`Error::NestedInterpreter`]). Both `ET_DYN`
/// (the usual `ld.so`) and `ET_EXEC` (loaded at its link addresses) are
/// accepted, as the gABI allows either.
pub fn check_interpreter(elf: &Elf<'_>) -> Result<(), Error> {
    if elf.interp().is_some() {
        return Err(Error::NestedInterpreter);
    }
    Ok(())
}

/// Options to plan an interpreter at `base` (from [`Footprint::base_in`]),
/// keeping the other limits of `opts`. An `ET_EXEC` interpreter ignores
/// the base.
pub const fn interpreter_options(opts: &LoadOptions, base: u64) -> LoadOptions {
    LoadOptions { dyn_base: base, min_addr: opts.min_addr, user_end: opts.user_end }
}

/// Whether the image needs a base chosen by the loader (`ET_DYN`), as
/// opposed to its link-time addresses (`ET_EXEC`).
pub const fn is_relocatable(elf: &Elf<'_>) -> bool {
    matches!(elf.elf_type(), ElfType::Dyn)
}
