//! Image sections (`SEC_IMAGE`): a PE executable or DLL laid out as it is
//! mapped, with the protection of every page, using `poly-pe`'s load plan
//! (ADR 0029).
//!
//! - Only x64 PE32+ images are accepted; anything else is
//!   `STATUS_INVALID_IMAGE_FORMAT` (to be confirmed by the spec agent,
//!   which may find `STATUS_INVALID_IMAGE_WIN_32` or a machine mismatch
//!   status for some cases).
//! - Page protections follow the sections' `IMAGE_SCN_MEM_*` flags (PE
//!   Format, "Section Flags"): the headers are `PAGE_READONLY`; readable
//!   sections `PAGE_READONLY`, writable ones `PAGE_WRITECOPY` (views of an
//!   image are copy on write, Learn: "Memory Protection Constants"),
//!   executable ones `PAGE_EXECUTE_READ` / `PAGE_EXECUTE_WRITECOPY`, a
//!   section without any access `PAGE_NOACCESS`. A page shared by two
//!   sections (alignments below the page size) gets the union of their
//!   rights. Gaps between sections are `PAGE_NOACCESS`. The exact values
//!   Windows reports are to be confirmed by the spec agent.
//!
//! # Held and lazy images (ADR 0155)
//!
//! A small image is laid out in memory when its section is made
//! ([`Held`]). A big one cannot be: the kernel's heap hands out at most
//! 4 MiB at a time, and Chrome's `chrome.dll` is 310 MB. Above
//! [`EAGER_MAX`] an image is [`Lazy`]: only the headers are read when the
//! section is made, and every page is made when a view first touches it,
//! from the file (through the page cache on a disk file) and, in a view
//! that is not at the preferred base, with the base relocations of that
//! page applied ([`Lazy::with_relocation`]). Both give the same bytes
//! (a host test compares them page by page).

use crate::consts::*;
use crate::protect::Protection;
use crate::vm::{PAGE_SIZE, page_up};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::STATUS_INVALID_IMAGE_FORMAT;
use poly_pe::{Bits, ImageInfo, Pe, reloc_kind};

/// Images up to this many bytes are laid out in memory when their section
/// is made.
pub const EAGER_MAX: u64 = 1 << 20;

/// How much of a file is read to parse the headers of a lazy image.
const HEADER_READ: usize = 64 * 1024;

/// The bytes of an image at their RVAs.
pub trait ImageBytes: Send + Sync + core::fmt::Debug {
    /// Bytes of address space: `SizeOfImage` rounded up to a page.
    fn len(&self) -> u64;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Copy bytes from RVA `offset` on into the start of `buf`; returns
    /// how many (0 only at or past the end).
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> usize;

    /// All of them, if they are held in memory.
    fn held(&self) -> Option<&[u8]> {
        None
    }
}

/// An image laid out in memory.
#[derive(Clone, Debug)]
pub struct Held(pub Arc<[u8]>);

impl ImageBytes for Held {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> usize {
        let start = usize::try_from(offset).map_or(self.0.len(), |o| o.min(self.0.len()));
        let n = buf.len().min(self.0.len() - start);
        buf[..n].copy_from_slice(&self.0[start..start + n]);
        n
    }

    fn held(&self) -> Option<&[u8]> {
        Some(&self.0)
    }
}

/// A file an image is read from.
pub trait ImageFile: Send + Sync + core::fmt::Debug {
    fn len(&self) -> u64;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Copy bytes from `offset` on into the start of `buf`; returns how
    /// many (0 only at or past the end). Reads until `buf` is full or the
    /// file ends.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> usize;
}

/// A file held in memory.
#[derive(Clone, Debug)]
pub struct FileBytes(pub Arc<[u8]>);

impl ImageFile for FileBytes {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> usize {
        let start = usize::try_from(offset).map_or(self.0.len(), |o| o.min(self.0.len()));
        let n = buf.len().min(self.0.len() - start);
        buf[..n].copy_from_slice(&self.0[start..start + n]);
        n
    }
}

/// A VFS file (an NT file object of NT file I/O, ADR 0067).
#[derive(Clone, Debug)]
pub struct VfsFile(pub Arc<poly_vfs::OpenFile>);

impl ImageFile for VfsFile {
    fn len(&self) -> u64 {
        self.0.stat().map_or(0, |s| s.size)
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> usize {
        let mut done = 0;
        while done < buf.len() {
            match self.0.read_at(offset + done as u64, &mut buf[done..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => done += n,
            }
        }
        done
    }
}

/// An image laid out in memory, ready to be mapped.
#[derive(Clone, Debug)]
pub struct ImageSection {
    /// The image's bytes at their RVAs, `size` long.
    pub image: Arc<dyn ImageBytes>,
    pub info: ImageInfo,
    /// Bytes of address space: `SizeOfImage` rounded up to a page.
    pub size: u64,
    /// The protection of each page.
    pub pages: Vec<Protection>,
    /// The same image as [`Lazy`], when it is not held.
    lazy: Option<Arc<Lazy>>,
}

#[derive(Clone, Copy, Default)]
struct Rights {
    read: bool,
    write: bool,
    execute: bool,
}

impl Rights {
    fn protection(self) -> u32 {
        match (self.read || self.write, self.write, self.execute) {
            (_, true, true) => PAGE_EXECUTE_WRITECOPY,
            (true, false, true) => PAGE_EXECUTE_READ,
            (false, false, true) => PAGE_EXECUTE,
            (_, true, false) => PAGE_WRITECOPY,
            (true, false, false) => PAGE_READONLY,
            (false, false, false) => PAGE_NOACCESS,
        }
    }
}

/// One range of the image and where its bytes are in the file.
#[derive(Clone, Copy, Debug)]
struct Part {
    rva: u32,
    file_offset: u64,
    /// Bytes the file holds for it; the rest of `extent` is zero.
    file_len: u32,
}

/// A base relocation block: the entries of one page.
#[derive(Clone, Copy, Debug)]
struct Block {
    page_rva: u32,
    /// Where the entries are, as an RVA of the relocation table.
    table_rva: u64,
    count: u32,
}

/// A relocated view's fix-ups.
#[derive(Debug)]
struct Fixups {
    /// Added to every relocated value: the new base minus the preferred one.
    delta: u64,
    /// Sorted by `page_rva`.
    blocks: Vec<Block>,
    /// Where the header keeps `ImageBase`, and the base stored there.
    base_field: u64,
    new_base: u64,
}

/// A big image, read from its file a page at a time.
#[derive(Clone, Debug)]
pub struct Lazy {
    file: Arc<dyn ImageFile>,
    parts: Vec<Part>,
    size: u64,
    fixups: Option<Arc<Fixups>>,
    /// The relocation directory (RVA, size), if any.
    reloc_dir: Option<(u32, u32)>,
}

impl Lazy {
    /// The image's bytes before any relocation, from RVA `at`.
    fn read_raw(&self, at: u64, buf: &mut [u8]) {
        buf.fill(0);
        let end = at + buf.len() as u64;
        for p in &self.parts {
            let (from, to) = (u64::from(p.rva).max(at), (u64::from(p.rva) + u64::from(p.file_len)).min(end));
            if from < to {
                let into = &mut buf[(from - at) as usize..(to - at) as usize];
                self.file.read_at(p.file_offset + (from - u64::from(p.rva)), into);
            }
        }
    }

    /// One page of the image, with the view's relocations applied.
    fn fill_page(&self, page: u64, out: &mut [u8; PAGE_SIZE as usize]) {
        self.read_raw(page, out);
        let Some(f) = &self.fixups else { return };
        // The header's `ImageBase` says where the image is now.
        for (i, byte) in f.new_base.to_le_bytes().iter().enumerate() {
            let at = f.base_field + i as u64;
            if (page..page + PAGE_SIZE).contains(&at) {
                out[(at - page) as usize] = *byte;
            }
        }
        // The blocks of this page and of the one before (a value may cross
        // the page boundary).
        let first = page.saturating_sub(PAGE_SIZE);
        let from = f.blocks.partition_point(|b| u64::from(b.page_rva) < first);
        for b in f.blocks[from..].iter().take_while(|b| u64::from(b.page_rva) <= page) {
            let mut table = vec![0u8; b.count as usize * 2];
            self.read_raw(b.table_rva, &mut table);
            for e in table.chunks_exact(2) {
                let entry = u16::from_le_bytes([e[0], e[1]]);
                let width = match (entry >> 12) as u8 {
                    reloc_kind::DIR64 => 8usize,
                    reloc_kind::HIGHLOW => 4,
                    _ => continue,
                };
                let target = u64::from(b.page_rva) + u64::from(entry & 0xfff);
                if target + width as u64 <= page || target >= page + PAGE_SIZE {
                    continue;
                }
                // The value as the image holds it: in the page when it lies
                // inside (entries do not overlap, so it is still the
                // file's), else read from the file (it crosses the boundary).
                let mut value = [0u8; 8];
                let inside = target >= page && target + width as u64 <= page + PAGE_SIZE;
                if inside {
                    let at = (target - page) as usize;
                    value[..width].copy_from_slice(&out[at..at + width]);
                } else {
                    self.read_raw(target, &mut value[..width]);
                }
                let fixed = u64::from_le_bytes(value).wrapping_add(f.delta).to_le_bytes();
                for (i, byte) in fixed[..width].iter().enumerate() {
                    let at = target + i as u64;
                    if (page..page + PAGE_SIZE).contains(&at) {
                        out[(at - page) as usize] = *byte;
                    }
                }
            }
        }
    }

    /// This image as a view at `base` sees it: with `info`'s relocations
    /// applied lazily. `STATUS_INVALID_IMAGE_FORMAT` if the table is
    /// malformed (it is checked whole, as `poly_pe::relocate` does).
    pub fn with_relocation(&self, info: &ImageInfo, base: u64) -> Result<Lazy, NtStatus> {
        let delta = base.wrapping_sub(info.image_base());
        if info.relocs_stripped() && delta != 0 {
            return Err(STATUS_INVALID_IMAGE_FORMAT);
        }
        // `ImageBase` of the PE32+ optional header, after the signature,
        // the file header (20 bytes) and 24 bytes of fields.
        let base_field = u64::from(info.nt_offset) + 4 + 20 + 24;
        let mut blocks = Vec::new();
        if let Some((rva, size)) = self.reloc_dir {
            let mut pos = 0u64;
            let end = u64::from(size);
            let mut head = [0u8; 8];
            while end - pos >= 8 {
                self.read_raw(u64::from(rva) + pos, &mut head);
                let page_rva = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
                let block = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
                if block < 8 || u64::from(block) > end - pos || page_rva > u32::MAX - 0xfff {
                    return Err(STATUS_INVALID_IMAGE_FORMAT);
                }
                let count = (block - 8) / 2;
                // Every entry must be a kind this machine uses, inside the image.
                let mut table = vec![0u8; count as usize * 2];
                self.read_raw(u64::from(rva) + pos + 8, &mut table);
                for e in table.chunks_exact(2) {
                    let entry = u16::from_le_bytes([e[0], e[1]]);
                    let width = match (entry >> 12) as u8 {
                        reloc_kind::ABSOLUTE => continue,
                        reloc_kind::DIR64 => 8u64,
                        reloc_kind::HIGHLOW => 4,
                        _ => return Err(STATUS_INVALID_IMAGE_FORMAT),
                    };
                    if u64::from(page_rva) + u64::from(entry & 0xfff) + width > self.size {
                        return Err(STATUS_INVALID_IMAGE_FORMAT);
                    }
                }
                blocks.push(Block { page_rva, table_rva: u64::from(rva) + pos + 8, count });
                pos += u64::from(block);
            }
        }
        blocks.sort_by_key(|b| b.page_rva);
        Ok(Lazy {
            file: Arc::clone(&self.file),
            parts: self.parts.clone(),
            size: self.size,
            fixups: Some(Arc::new(Fixups { delta, blocks, base_field, new_base: base })),
            reloc_dir: self.reloc_dir,
        })
    }
}

impl ImageBytes for Lazy {
    fn len(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> usize {
        if offset >= self.size {
            return 0;
        }
        let n = buf.len().min((self.size - offset) as usize);
        let mut done = 0;
        let mut page = [0u8; PAGE_SIZE as usize];
        while done < n {
            let at = offset + done as u64;
            let base = at & !(PAGE_SIZE - 1);
            self.fill_page(base, &mut page);
            let from = (at - base) as usize;
            let take = (PAGE_SIZE as usize - from).min(n - done);
            buf[done..done + take].copy_from_slice(&page[from..from + take]);
            done += take;
        }
        n
    }
}

impl ImageSection {
    /// Parse and lay out the image file `file`.
    pub fn from_file(file: &[u8]) -> Result<ImageSection, NtStatus> {
        let bad = |_| STATUS_INVALID_IMAGE_FORMAT;
        let pe = Pe::parse(file).map_err(bad)?;
        let plan = Self::checked_plan(&pe, file.len() as u64)?;
        let size = page_up(u64::from(plan.size_of_image)).ok_or(STATUS_INVALID_IMAGE_FORMAT)?;
        if size > EAGER_MAX {
            return Self::lazy(Arc::new(FileBytes(Arc::from(file))));
        }
        let mut image = vec![0u8; size as usize];
        plan.map_into(&mut image).map_err(bad)?;
        let pages = Self::page_protections(&plan, size);
        Ok(ImageSection { image: Arc::new(Held(image.into())), info: *pe.info(), size, pages, lazy: None })
    }

    /// Parse and lay out the image in `file`, reading only its headers when
    /// it is big (see the module documentation).
    pub fn from_reader(file: Arc<dyn ImageFile>) -> Result<ImageSection, NtStatus> {
        let len = file.len();
        if len <= EAGER_MAX {
            let mut bytes = vec![0u8; len as usize];
            let n = file.read_at(0, &mut bytes);
            bytes.truncate(n);
            return Self::from_file(&bytes);
        }
        Self::lazy(file)
    }

    pub(crate) fn lazy(file: Arc<dyn ImageFile>) -> Result<ImageSection, NtStatus> {
        let bad = |_| STATUS_INVALID_IMAGE_FORMAT;
        let mut head = vec![0u8; HEADER_READ.min(file.len() as usize)];
        let n = file.read_at(0, &mut head);
        head.truncate(n);
        let pe = Pe::parse(&head).map_err(bad)?;
        let plan = Self::checked_plan(&pe, file.len())?;
        let size = page_up(u64::from(plan.size_of_image)).ok_or(STATUS_INVALID_IMAGE_FORMAT)?;
        let pages = Self::page_protections(&plan, size);
        let parts = plan
            .regions()
            .map(|r| Part { rva: r.rva, file_offset: r.data_offset as u64, file_len: r.data_size })
            .collect();
        let reloc_dir =
            pe.info().directory(poly_pe::dir::BASERELOC).filter(|d| d.size != 0).map(|d| (d.virtual_address, d.size));
        let lazy = Arc::new(Lazy { file, parts, size, fixups: None, reloc_dir });
        Ok(ImageSection { image: lazy.clone(), info: *pe.info(), size, pages, lazy: Some(lazy) })
    }

    fn checked_plan<'a>(pe: &Pe<'a>, file_len: u64) -> Result<poly_pe::LoadPlan<'a>, NtStatus> {
        let bad = |_| STATUS_INVALID_IMAGE_FORMAT;
        let info = *pe.info();
        if info.machine() != poly_pe::machine::AMD64 || info.bits() != Bits::Pe32Plus {
            return Err(STATUS_INVALID_IMAGE_FORMAT);
        }
        pe.load_plan_in(file_len).map_err(bad)
    }

    fn page_protections(plan: &poly_pe::LoadPlan<'_>, size: u64) -> Vec<Protection> {
        let mut rights = vec![Rights::default(); (size / PAGE_SIZE) as usize];
        for region in plan.regions() {
            let start = u64::from(region.rva) / PAGE_SIZE;
            let end = page_up(u64::from(region.rva) + u64::from(region.size)).unwrap_or(size).min(size) / PAGE_SIZE;
            for r in &mut rights[start as usize..end as usize] {
                r.read |= region.protection.read;
                r.write |= region.protection.write;
                r.execute |= region.protection.execute;
            }
        }
        rights.iter().map(|r| Protection::new(r.protection(), false).expect("a base protection")).collect()
    }

    pub fn image_base(&self) -> u64 {
        self.info.image_base()
    }

    pub fn relocatable(&self) -> bool {
        self.info.relocatable()
    }

    /// The image relocated to load at `base` (ADR 0029: `poly_pe::relocate`
    /// validates the whole table before writing; a lazy image checks the
    /// table whole and fixes pages as they are made).
    pub fn relocated(&self, base: u64) -> Result<Arc<dyn ImageBytes>, NtStatus> {
        if let Some(lazy) = &self.lazy {
            return Ok(Arc::new(lazy.with_relocation(&self.info, base)?));
        }
        let held = self.image.held().ok_or(STATUS_INVALID_IMAGE_FORMAT)?;
        let mut copy = held.to_vec();
        poly_pe::relocate(&mut copy, &self.info, base).map_err(|_| STATUS_INVALID_IMAGE_FORMAT)?;
        Ok(Arc::new(Held(copy.into())))
    }
}
