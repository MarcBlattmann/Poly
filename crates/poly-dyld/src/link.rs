//! Loading a program and its libraries: where each image goes, which
//! library each dependency is, what each symbol resolves to, and the
//! fixups, initializers and thread-local variables of every image.

use crate::error::{Error, Result};
use crate::path::{self, Loader};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ops::Range;
use poly_macho::{
    BIND_SPECIAL_DYLIB_FLAT_LOOKUP, BIND_SPECIAL_DYLIB_MAIN_EXECUTABLE, BIND_SPECIAL_DYLIB_SELF,
    BIND_SPECIAL_DYLIB_WEAK_LOOKUP, BIND_SYMBOL_FLAGS_NON_WEAK_DEFINITION, BIND_TYPE_POINTER, CpuKind, DylibKind,
    ExportKind, ExportTarget, FileType, FixupKind, MachO, REBASE_TYPE_POINTER, S_INIT_FUNC_OFFSETS,
    S_MOD_INIT_FUNC_POINTERS, S_THREAD_LOCAL_REGULAR, S_THREAD_LOCAL_VARIABLES, S_THREAD_LOCAL_ZEROFILL, SG_READ_ONLY,
};

/// The most images one program may load.
pub const MAX_IMAGES: usize = 1024;
/// How deep re-exports may nest when looking a symbol up.
const MAX_REEXPORT_DEPTH: usize = 32;
/// The install name of the system library, whose `_exit` ends a program
/// when its `main` returns.
pub const LIBSYSTEM: &str = "/usr/lib/libSystem.B.dylib";
/// The size of a thread-local variable descriptor: thunk, key, offset
/// (`<mach-o/loader.h>`, `S_THREAD_LOCAL_VARIABLES`).
pub const TLV_DESCRIPTOR_SIZE: u64 = 24;

/// Where the loader gets files from.
pub trait FileSystem {
    /// The whole file at `path`, or `None` if there is none (or it cannot
    /// be read).
    fn read(&mut self, path: &str) -> Option<Vec<u8>>;
}

/// How to place a program.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// The CPU whose slice is loaded from universal files.
    pub cpu: CpuKind,
    /// The page size mappings are made with; segments must be aligned to it.
    pub page_size: u64,
    /// Where a position-independent program's Mach-O header goes; `None`
    /// keeps its preferred address.
    pub program_base: Option<u64>,
    /// The lowest address for libraries; each goes after the previous one,
    /// with an unmapped guard page between them.
    pub library_base: u64,
}

impl Options {
    /// x86-64 with 4 KiB pages, the program at its preferred address and
    /// libraries from 32 GiB on (above the 4 GiB `__PAGEZERO` and the usual
    /// program address).
    pub const X86_64: Options =
        Options { cpu: CpuKind::X86_64, page_size: 4096, program_base: None, library_base: 0x8_0000_0000 };
}

/// Memory protection of a segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prot {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

/// One segment of a placed image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// The segment name (`__TEXT`).
    pub name: String,
    /// Where it starts in memory (page aligned).
    pub addr: u64,
    /// Its size in memory, rounded up to whole pages.
    pub mem_size: u64,
    /// The offset of its bytes in the file (the whole file, a universal
    /// file's slice offset included; page aligned).
    pub file_offset: u64,
    /// How many bytes come from the file; the rest is zero.
    pub file_size: u64,
    /// The protection to run with (`initprot`).
    pub prot: Prot,
    /// `SG_READ_ONLY`: writable while fixups are applied, read-only after.
    pub read_only_after_fixups: bool,
}

/// The thread-local variables of an image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tlv {
    /// Where the template starts (the first thread-local data or zero fill
    /// section).
    pub template: u64,
    /// How many bytes of the template are initial values (the rest of a
    /// block is zero).
    pub init_size: u64,
    /// The size of one thread's block.
    pub size: u64,
    /// The alignment a block needs.
    pub align: u64,
    /// The address of every descriptor (`S_THREAD_LOCAL_VARIABLES`).
    pub descriptors: Vec<u64>,
}

/// A placed image: the program or a library.
#[derive(Clone, Debug)]
pub struct Image {
    /// The path it was loaded from.
    pub path: String,
    /// Its install name (`LC_ID_DYLIB`), for a library.
    pub install_name: Option<String>,
    /// The whole file.
    pub file: Vec<u8>,
    /// The thin slice of `file` loaded.
    pub slice: Range<usize>,
    /// Where its Mach-O header is in memory.
    pub base: u64,
    /// What was added to its link-time addresses.
    pub slide: u64,
    /// Its segments, `__PAGEZERO` and empty ones left out.
    pub segments: Vec<Segment>,
    /// The image of each dependency, by library ordinal - 1 (`None`: a weak
    /// library that is not there).
    pub deps: Vec<Option<usize>>,
    /// The images it re-exports (`LC_REEXPORT_DYLIB`).
    pub reexports: Vec<usize>,
    /// Its run paths, as written.
    pub rpaths: Vec<String>,
    /// Whether it binds with the two-level namespace (`MH_TWOLEVEL`).
    pub two_level: bool,
    /// The image that loaded it first (`None` for the program).
    pub parent: Option<usize>,
    /// Its initializers, in the order to call them.
    pub initializers: Vec<u64>,
    /// Its thread-local variables.
    pub tlv: Option<Tlv>,
}

impl Image {
    /// The parsed thin slice. It was checked when the image was loaded.
    /// Boxed: the view holds a table of segments (a few KiB), and the
    /// loader runs on small kernel stacks, several lookups deep.
    pub fn macho(&self) -> Result<Box<MachO<'_>>> {
        parse_boxed(&self.file[self.slice.clone()]).map_err(|error| Error::Macho { path: self.path.clone(), error })
    }

    /// The thin slice's bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.file[self.slice.clone()]
    }

    /// The memory the image occupies.
    pub fn range(&self) -> Range<u64> {
        let start = self.segments.iter().map(|s| s.addr).min().unwrap_or(self.base);
        let end = self.segments.iter().map(|s| s.addr + s.mem_size).max().unwrap_or(self.base);
        start..end
    }

    fn display_name(&self) -> String {
        self.install_name.clone().unwrap_or_else(|| self.path.clone())
    }
}

/// Where a program starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// `LC_MAIN`: dyld's start code calls `main(argc, argv, envp, apple)`
    /// at `addr` and exits with its result.
    Main { addr: u64, stack_size: u64 },
    /// `LC_UNIXTHREAD`: the thread starts at `pc` with the stack pointer at
    /// `argc` (`sp`: the stack the file asks for, 0 for the default).
    Thread { pc: u64, sp: u64 },
}

/// A linked program: every image placed, every dependency found.
#[derive(Clone, Debug)]
pub struct Program {
    /// The images in load order; the program is the first.
    pub images: Vec<Image>,
    /// Where it starts.
    pub entry: Entry,
    /// Whether it asks for a dynamic linker (`LC_LOAD_DYLINKER`). A program
    /// without one and with `LC_UNIXTHREAD` is static: no dyld runs.
    pub dynamic: bool,
    /// The order to run initializers in: every image after the images it
    /// depends on, the program last.
    pub init_order: Vec<usize>,
}

/// A symbol found in an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Found {
    addr: u64,
    weak: bool,
}

/// Load the program at `path` and every library it needs, place them, and
/// work out their initializers and thread-local variables. Nothing is
/// written: [`Program::fixups`] says what to write once the segments are
/// mapped.
pub fn link(fs: &mut dyn FileSystem, path: &str, opts: &Options) -> Result<Program> {
    let file = fs.read(path).ok_or_else(|| Error::LibraryNotFound { name: path.to_string(), from: "exec".into() })?;
    let mut images = Vec::new();
    let program = open_image(path, file, None, opts)?;
    {
        let m = program.macho()?;
        if m.header().file_type() != FileType::Execute {
            return Err(Error::WrongFileType { path: path.to_string() });
        }
    }
    images.push(program);
    place_program(&mut images[0], opts)?;
    let mut next_lib = page_up(opts.library_base, opts.page_size);

    // Breadth first: each image's dependencies in load command order.
    let mut i = 0;
    while i < images.len() {
        let names: Vec<(String, DylibKind)> = {
            let m = images[i].macho()?;
            m.dylibs().map(|d| (d.name.to_string(), d.kind)).collect()
        };
        let mut deps = Vec::with_capacity(names.len());
        let mut reexports = Vec::new();
        for (name, kind) in names {
            let found = find_library(fs, &mut images, i, &name, opts, &mut next_lib)?;
            match found {
                Some(t) => {
                    if kind == DylibKind::Reexport {
                        reexports.push(t);
                    }
                }
                None if kind == DylibKind::Weak => {}
                None => {
                    return Err(Error::LibraryNotFound { name, from: images[i].path.clone() });
                }
            }
            deps.push(found);
        }
        images[i].deps = deps;
        images[i].reexports = reexports;
        i += 1;
    }
    check_overlaps(&images)?;

    let (entry, dynamic) = {
        let m = images[0].macho()?;
        let e = m
            .entry()
            .map_err(|error| Error::Macho { path: path.to_string(), error })?
            .ok_or_else(|| Error::NoEntry { path: path.to_string() })?;
        let slide = images[0].slide;
        let entry = match e {
            poly_macho::Entry::Main { entryoff, stacksize, .. } => {
                Entry::Main { addr: images[0].base.wrapping_add(entryoff), stack_size: stacksize }
            }
            poly_macho::Entry::UnixThread { pc, sp } => {
                Entry::Thread { pc: pc.wrapping_add(slide), sp: if sp == 0 { 0 } else { sp.wrapping_add(slide) } }
            }
        };
        (entry, m.dylinker().is_some())
    };
    let init_order = init_order(&images);
    let mut program = Program { images, entry, dynamic, init_order };
    for i in 0..program.images.len() {
        program.images[i].tlv = tlv_of(&program.images[i])?;
    }
    for i in 0..program.images.len() {
        program.images[i].initializers = program.initializers_of(i)?;
    }
    Ok(program)
}

/// Read and check one image (its CPU slice), not yet placed.
fn open_image(path: &str, file: Vec<u8>, parent: Option<usize>, opts: &Options) -> Result<Image> {
    let macho_err = |error| Error::Macho { path: path.to_string(), error };
    let thin = poly_macho::select_slice(&file, opts.cpu).map_err(|e| match e {
        poly_macho::Error::NoMatchingArch => Error::WrongCpu { path: path.to_string() },
        e => macho_err(e),
    })?;
    let start = thin.as_ptr() as usize - file.as_ptr() as usize;
    let slice = start..start + thin.len();
    let m = parse_boxed(thin).map_err(macho_err)?;
    let two_level = m.header().is_two_level();
    let install_name = m.dylib_id().map(|d| d.name.to_string());
    let rpaths = m.rpaths().map(ToString::to_string).collect();
    Ok(Image {
        path: path.to_string(),
        install_name,
        file,
        slice,
        base: 0,
        slide: 0,
        segments: Vec::new(),
        deps: Vec::new(),
        reexports: Vec::new(),
        rpaths,
        two_level,
        parent,
        initializers: Vec::new(),
        tlv: None,
    })
}

fn page_up(x: u64, page: u64) -> u64 {
    x.checked_add(page - 1).map_or(!(page - 1), |v| v & !(page - 1))
}

/// Give `image` the slide `slide` and work out its segments.
fn place(image: &mut Image, slide: u64, opts: &Options) -> Result<()> {
    let bad = |why| Error::BadLayout { path: image.path.clone(), why };
    let m = image.macho()?;
    let base = m.header_segment().ok_or_else(|| bad("no segment maps the Mach-O header"))?.vmaddr;
    let mut segments = Vec::new();
    for s in m.segment_table() {
        if s.is_pagezero() || s.vmsize == 0 {
            continue;
        }
        let addr = s.vmaddr.wrapping_add(slide);
        let file_offset = s.fileoff + image.slice.start as u64;
        if !addr.is_multiple_of(opts.page_size) || (s.filesize != 0 && !file_offset.is_multiple_of(opts.page_size)) {
            return Err(bad("a segment is not page aligned"));
        }
        let mem_size = page_up(s.vmsize, opts.page_size);
        if addr.checked_add(mem_size).is_none_or(|end| end > 1 << 47) {
            return Err(bad("a segment does not fit the user address space"));
        }
        let name = core::str::from_utf8(s.name()).unwrap_or("?").to_string();
        segments.push(Segment {
            name,
            addr,
            mem_size,
            file_offset,
            file_size: s.filesize,
            prot: Prot { read: s.initprot.read(), write: s.initprot.write(), execute: s.initprot.execute() },
            read_only_after_fixups: s.flags & SG_READ_ONLY != 0,
        });
    }
    drop(m);
    image.base = base.wrapping_add(slide);
    image.slide = slide;
    image.segments = segments;
    Ok(())
}

/// Place the program: at its preferred address, or at `program_base` if it
/// is position independent and the options ask.
fn place_program(image: &mut Image, opts: &Options) -> Result<()> {
    let (pie, preferred) = {
        let m = image.macho()?;
        let s = m.summary().map_err(|error| Error::Macho { path: image.path.clone(), error })?;
        (s.is_pie, s.base_address)
    };
    let slide = match opts.program_base {
        Some(b) if pie => b.wrapping_sub(preferred),
        _ => 0,
    };
    place(image, slide, opts)
}

/// Place a library at `*next` and move `*next` past it and a guard page.
fn place_library(image: &mut Image, opts: &Options, next: &mut u64) -> Result<()> {
    let low = {
        let m = image.macho()?;
        m.vm_range().ok_or(Error::BadLayout { path: image.path.clone(), why: "no segment has memory" })?
    };
    let slide = next.wrapping_sub(low.start);
    place(image, slide, opts)?;
    let end = image.range().end;
    *next = page_up(end, opts.page_size) + opts.page_size;
    Ok(())
}

/// The image for the library `name` that image `from` loads: one already
/// loaded (same path or install name), or a new one read from `fs`.
/// `Ok(None)`: not found.
fn find_library(
    fs: &mut dyn FileSystem,
    images: &mut Vec<Image>,
    from: usize,
    name: &str,
    opts: &Options,
    next: &mut u64,
) -> Result<Option<usize>> {
    if let Some(i) = images.iter().position(|im| im.install_name.as_deref() == Some(name)) {
        return Ok(Some(i));
    }
    let executable = images[0].path.clone();
    let candidates = {
        let mut chain_ix = Vec::new();
        let mut at = Some(from);
        while let Some(i) = at {
            if chain_ix.contains(&i) {
                break;
            }
            chain_ix.push(i);
            at = images[i].parent;
        }
        let chain: Vec<Loader<'_>> =
            chain_ix.iter().map(|&i| Loader { path: &images[i].path, rpaths: &images[i].rpaths }).collect();
        path::candidates(name, &executable, &chain)
    };
    for cand in candidates {
        if let Some(i) = images.iter().position(|im| im.path == cand) {
            return Ok(Some(i));
        }
        let Some(file) = fs.read(&cand) else { continue };
        if images.len() >= MAX_IMAGES {
            return Err(Error::TooManyImages);
        }
        let mut image = open_image(&cand, file, Some(from), opts)?;
        // The same library under another path (a link) is the same image.
        if let Some(id) = &image.install_name
            && let Some(i) = images.iter().position(|im| im.install_name.as_ref() == Some(id))
        {
            return Ok(Some(i));
        }
        {
            let m = image.macho()?;
            if m.header().file_type() != FileType::Dylib {
                return Err(Error::WrongFileType { path: cand });
            }
        }
        place_library(&mut image, opts, next)?;
        images.push(image);
        return Ok(Some(images.len() - 1));
    }
    Ok(None)
}

fn check_overlaps(images: &[Image]) -> Result<()> {
    let mut ranges: Vec<(Range<u64>, &str)> = Vec::new();
    for im in images {
        for s in &im.segments {
            let r = s.addr..s.addr + s.mem_size;
            if ranges.iter().any(|(o, _)| o.start < r.end && r.start < o.end) {
                return Err(Error::BadLayout { path: im.path.clone(), why: "overlaps another image" });
            }
            ranges.push((r, &im.path));
        }
    }
    Ok(())
}

/// Dependencies before dependents (upward ones aside), the program last.
fn init_order(images: &[Image]) -> Vec<usize> {
    let mut order = Vec::with_capacity(images.len());
    let mut seen = alloc::vec![false; images.len()];
    // Iterative post-order walk: (image, next dependency to visit).
    let mut stack: Vec<(usize, usize)> = alloc::vec![(0, 0)];
    seen[0] = true;
    let upward: Vec<Vec<bool>> = images
        .iter()
        .map(|im| im.macho().map(|m| m.dylibs().map(|d| d.kind == DylibKind::Upward).collect()).unwrap_or_default())
        .collect();
    while let Some(&mut (i, ref mut k)) = stack.last_mut() {
        if let Some(dep) = images[i].deps.get(*k).copied() {
            let up = upward[i].get(*k).copied().unwrap_or(false);
            *k += 1;
            if let Some(d) = dep
                && !up
                && !seen[d]
            {
                seen[d] = true;
                stack.push((d, 0));
            }
        } else {
            order.push(i);
            stack.pop();
        }
    }
    // Images reached only through upward links still get initialized.
    for (i, s) in seen.iter().enumerate() {
        if !s {
            order.insert(order.len().saturating_sub(1), i);
        }
    }
    order
}

/// The thread-local variables of `image`, if it has any.
fn tlv_of(image: &Image) -> Result<Option<Tlv>> {
    let m = image.macho()?;
    let mut descriptors = Vec::new();
    let mut template: Option<Range<u64>> = None;
    let mut init_end = 0;
    let mut align = 1u64;
    for seg in m.segments() {
        for s in seg.sections() {
            let addr = s.addr.wrapping_add(image.slide);
            match s.section_type() {
                S_THREAD_LOCAL_VARIABLES => {
                    let n = s.size / TLV_DESCRIPTOR_SIZE;
                    descriptors.extend((0..n).map(|k| addr + k * TLV_DESCRIPTOR_SIZE));
                }
                t @ (S_THREAD_LOCAL_REGULAR | S_THREAD_LOCAL_ZEROFILL) => {
                    let r = addr..addr + s.size;
                    template = Some(match template {
                        None => r.clone(),
                        Some(t) => t.start.min(r.start)..t.end.max(r.end),
                    });
                    if t == S_THREAD_LOCAL_REGULAR {
                        init_end = init_end.max(r.end);
                    }
                    align = align.max(1u64 << s.align.min(12));
                }
                _ => {}
            }
        }
    }
    if descriptors.is_empty() {
        return Ok(None);
    }
    let t = template.unwrap_or(0..0);
    Ok(Some(Tlv {
        template: t.start,
        init_size: init_end.saturating_sub(t.start),
        size: t.end - t.start,
        align,
        descriptors,
    }))
}

impl Program {
    /// The image whose install name is `name`.
    pub fn image_named(&self, name: &str) -> Option<usize> {
        self.images.iter().position(|im| im.install_name.as_deref() == Some(name))
    }

    /// The address of the exported symbol `name` (with its leading
    /// underscore) of image `image`, re-exports followed.
    pub fn lookup(&self, image: usize, name: &[u8]) -> Result<Option<u64>> {
        Ok(self.find(image, name, 0)?.map(|f| f.addr))
    }

    /// The function `_exit` of libSystem, which ends the program when `main`
    /// returns; `None` if there is no libSystem or it has none.
    pub fn exit_function(&self) -> Result<Option<u64>> {
        match self.image_named(LIBSYSTEM) {
            Some(i) => self.lookup(i, b"_exit"),
            None => Ok(None),
        }
    }

    fn find(&self, image: usize, name: &[u8], depth: usize) -> Result<Option<Found>> {
        let im = &self.images[image];
        if depth > MAX_REEXPORT_DEPTH {
            return Err(Error::Unsupported { path: im.path.clone(), why: "re-exports nest too deep (a loop?)" });
        }
        let m = im.macho()?;
        let merr = |error| Error::Macho { path: im.path.clone(), error };
        if let Some(ex) = m.exports().map_err(merr)?
            && let Some(e) = ex.lookup(name).map_err(merr)?
        {
            let weak = e.is_weak_definition();
            return match e.target {
                ExportTarget::Address(off) => {
                    let addr = if e.kind() == ExportKind::Absolute { off } else { im.base.wrapping_add(off) };
                    Ok(Some(Found { addr, weak }))
                }
                // The stub calls the resolver's choice once it is bound;
                // Poly binds the stub itself (its lazy pointer starts at
                // the resolver's helper).
                ExportTarget::StubAndResolver { stub, .. } => {
                    Ok(Some(Found { addr: im.base.wrapping_add(stub), weak }))
                }
                ExportTarget::Reexport { ordinal, name: other } => {
                    let dep = usize::try_from(ordinal).ok().and_then(|o| o.checked_sub(1));
                    match dep.and_then(|d| im.deps.get(d).copied().flatten()) {
                        Some(t) => self.find(t, if other.is_empty() { name } else { other }, depth + 1),
                        None => Ok(None),
                    }
                }
            };
        }
        for &r in &im.reexports {
            if let Some(f) = self.find(r, name, depth + 1)? {
                return Ok(Some(f));
            }
        }
        Ok(None)
    }

    /// The first definition of `name` in load order, a strong one before a
    /// weak one (weak coalescing and flat lookups).
    fn find_flat(&self, name: &[u8], prefer_strong: bool) -> Result<Option<Found>> {
        let mut weak = None;
        for i in 0..self.images.len() {
            if let Some(f) = self.find(i, name, 0)? {
                if !prefer_strong || !f.weak {
                    return Ok(Some(f));
                }
                weak.get_or_insert(f);
            }
        }
        Ok(weak)
    }

    /// What image `image`'s import of `name` from library `ordinal` binds
    /// to: `Ok(None)` if a weak import is missing.
    fn resolve(&self, image: usize, ordinal: i32, name: &[u8], weak_import: bool) -> Result<Option<u64>> {
        let im = &self.images[image];
        let two_level = im.two_level;
        let expected = || -> String {
            match ordinal {
                BIND_SPECIAL_DYLIB_SELF => im.display_name(),
                BIND_SPECIAL_DYLIB_MAIN_EXECUTABLE => self.images[0].path.clone(),
                o if o > 0 => im
                    .macho()
                    .ok()
                    .and_then(|m| m.dylib_by_ordinal(o as u32).map(|d| d.name.to_string()))
                    .unwrap_or_else(|| "?".into()),
                _ => "flat namespace".into(),
            }
        };
        let found = if !two_level || ordinal == BIND_SPECIAL_DYLIB_FLAT_LOOKUP {
            self.find_flat(name, false)?
        } else {
            match ordinal {
                BIND_SPECIAL_DYLIB_SELF => self.find(image, name, 0)?,
                BIND_SPECIAL_DYLIB_MAIN_EXECUTABLE => self.find(0, name, 0)?,
                BIND_SPECIAL_DYLIB_WEAK_LOOKUP => self.find_flat(name, true)?,
                o if o > 0 => match im.deps.get(o as usize - 1) {
                    Some(Some(t)) => self.find(*t, name, 0)?,
                    Some(None) => None,
                    None => {
                        return Err(Error::Unsupported {
                            path: im.path.clone(),
                            why: "a bind names a library ordinal the image does not have",
                        });
                    }
                },
                _ => {
                    return Err(Error::Unsupported {
                        path: im.path.clone(),
                        why: "an unknown special library ordinal",
                    });
                }
            }
        };
        match found {
            Some(f) => Ok(Some(f.addr)),
            // Never called once binds are eager: only there for the linker.
            None if weak_import || name == b"dyld_stub_binder" => Ok(None),
            None => Err(Error::SymbolNotFound {
                symbol: String::from_utf8_lossy(name).into_owned(),
                from: im.path.clone(),
                expected_in: expected(),
            }),
        }
    }

    /// Every pointer the images need written once their segments are
    /// mapped: `write(address, value)` for each rebase and bind (lazy
    /// binds included: Poly binds everything at load). Thread-local
    /// descriptors' thunks are left to [`tlv_writes`](Self::tlv_writes).
    pub fn fixups(&self, write: &mut dyn FnMut(u64, u64)) -> Result<()> {
        for i in 0..self.images.len() {
            self.fixups_of(i, write)?;
        }
        Ok(())
    }

    /// [`fixups`](Self::fixups) of one image.
    pub fn fixups_of(&self, image: usize, write: &mut dyn FnMut(u64, u64)) -> Result<()> {
        let im = &self.images[image];
        let m = im.macho()?;
        let merr = |error| Error::Macho { path: im.path.clone(), error };
        let skip: BTreeSet<u64> = im.tlv.iter().flat_map(|t| t.descriptors.iter().copied()).collect();
        let mut out = |addr: u64, value: u64| {
            if !skip.contains(&addr) {
                write(addr, value);
            }
        };
        if let Some(cf) = m.chained_fixups().map_err(merr)? {
            let mut imports: Vec<Option<Option<u64>>> = alloc::vec![None; cf.import_count() as usize];
            for f in cf.fixups() {
                let f = f.map_err(merr)?;
                let addr = im.base.wrapping_add(f.vm_offset);
                match f.kind {
                    FixupKind::Rebase(r) => {
                        let target = if r.target_is_vmaddr {
                            r.target.wrapping_add(im.slide)
                        } else {
                            im.base.wrapping_add(r.target)
                        };
                        out(addr, target | (u64::from(r.high8) << 56));
                    }
                    FixupKind::Bind(b) => {
                        let ix = b.ordinal as usize;
                        let imp = cf.import(b.ordinal).map_err(merr)?;
                        let slot = imports.get_mut(ix).ok_or(Error::Unsupported {
                            path: im.path.clone(),
                            why: "a bind names an import out of range",
                        })?;
                        let resolved = match slot {
                            Some(v) => *v,
                            None => {
                                if skip.contains(&addr) {
                                    continue;
                                }
                                let v = self.resolve(image, imp.lib_ordinal, imp.name, imp.weak_import)?;
                                *slot = Some(v);
                                v
                            }
                        };
                        let value = match resolved {
                            Some(a) => a.wrapping_add(imp.addend as u64).wrapping_add(b.addend as u64),
                            None => 0,
                        };
                        out(addr, value);
                    }
                }
            }
            return Ok(());
        }
        let Some(info) = m.dyld_info().map_err(merr)? else { return Ok(()) };
        let table = m.segment_table();
        let unsupported = |why| Error::Unsupported { path: im.path.clone(), why };
        for r in info.rebases() {
            let r = r.map_err(merr)?;
            if r.kind != REBASE_TYPE_POINTER {
                return Err(unsupported("a 32-bit text rebase (only pointer rebases are supported)"));
            }
            let seg = table.get(r.segment as usize).ok_or(unsupported("a rebase in a missing segment"))?;
            let off = seg.fileoff + r.segment_offset;
            let raw = read_u64(im.bytes(), off).ok_or(unsupported("a rebase outside the file"))?;
            out(r.address.wrapping_add(im.slide), raw.wrapping_add(im.slide));
        }
        let lazy = info.lazy_binds().map(|b| (b, true));
        for (b, is_lazy) in info.binds().map(|b| (b, false)).chain(lazy) {
            let b = b.map_err(merr)?;
            // A lazy stream sets no type: its entries are pointers.
            if !is_lazy && b.kind != BIND_TYPE_POINTER {
                return Err(unsupported("a 32-bit text bind (only pointer binds are supported)"));
            }
            let addr = b.address.wrapping_add(im.slide);
            if skip.contains(&addr) {
                continue;
            }
            let value = match self.resolve(image, b.ordinal, b.symbol, b.is_weak_import())? {
                Some(a) => a.wrapping_add(b.addend as u64),
                None => 0,
            };
            out(addr, value);
        }
        // Weak definitions: every use goes to the first definition in load
        // order (a strong one if there is one).
        for b in info.weak_binds() {
            let b = b.map_err(merr)?;
            if b.flags & BIND_SYMBOL_FLAGS_NON_WEAK_DEFINITION != 0 {
                continue;
            }
            if let Some(f) = self.find_flat(b.symbol, true)? {
                out(b.address.wrapping_add(im.slide), f.addr.wrapping_add(b.addend as u64));
            }
        }
        Ok(())
    }

    /// Image `image`'s initializers: the pointers of its
    /// `S_MOD_INIT_FUNC_POINTERS` sections (as its fixups leave them) and
    /// the offsets of its `S_INIT_FUNC_OFFSETS` sections, in section order.
    fn initializers_of(&self, image: usize) -> Result<Vec<u64>> {
        let im = &self.images[image];
        let m = im.macho()?;
        let mut slots = Vec::new();
        let mut out = Vec::new();
        for seg in m.segments() {
            for s in seg.sections() {
                let file = u64::from(s.offset);
                match s.section_type() {
                    S_MOD_INIT_FUNC_POINTERS => {
                        for k in 0..s.size / 8 {
                            let raw = read_u64(im.bytes(), file + k * 8).unwrap_or(0);
                            slots.push((out.len(), s.addr.wrapping_add(im.slide) + k * 8));
                            out.push(raw);
                        }
                    }
                    S_INIT_FUNC_OFFSETS => {
                        for k in 0..s.size / 4 {
                            let off = read_u32(im.bytes(), file + k * 4).unwrap_or(0);
                            out.push(im.base.wrapping_add(u64::from(off)));
                        }
                    }
                    _ => {}
                }
            }
        }
        if !slots.is_empty() {
            let want: BTreeMap<u64, usize> = slots.iter().map(|&(i, a)| (a, i)).collect();
            let mut found = BTreeMap::new();
            self.fixups_of(image, &mut |addr, value| {
                if want.contains_key(&addr) {
                    found.insert(addr, value);
                }
            })?;
            for (addr, i) in want {
                if let Some(v) = found.get(&addr) {
                    out[i] = *v;
                }
            }
        }
        Ok(out)
    }

    /// Every initializer of every image, in the order to call them.
    pub fn initializers(&self) -> Vec<u64> {
        self.init_order.iter().flat_map(|&i| self.images[i].initializers.iter().copied()).collect()
    }

    /// The writes that hand image `image`'s thread-local descriptors to
    /// Poly: each descriptor's thunk becomes `thunk` and its key `block`,
    /// the address of this thread's copy of the template (its offset word
    /// stays). The thunk returns key + offset.
    pub fn tlv_writes(&self, image: usize, block: u64, thunk: u64, write: &mut dyn FnMut(u64, u64)) {
        if let Some(t) = &self.images[image].tlv {
            for &d in &t.descriptors {
                write(d, thunk);
                write(d + 8, block);
            }
        }
    }
}

/// [`MachO::parse`] into a box, the view built in this frame only.
#[inline(never)]
fn parse_boxed(bytes: &[u8]) -> poly_macho::Result<Box<MachO<'_>>> {
    MachO::parse(bytes).map(Box::new)
}

fn read_u64(bytes: &[u8], off: u64) -> Option<u64> {
    let off = usize::try_from(off).ok()?;
    let b = bytes.get(off..off.checked_add(8)?)?;
    Some(u64::from_le_bytes(b.try_into().ok()?))
}

fn read_u32(bytes: &[u8], off: u64) -> Option<u32> {
    let off = usize::try_from(off).ok()?;
    let b = bytes.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes(b.try_into().ok()?))
}
