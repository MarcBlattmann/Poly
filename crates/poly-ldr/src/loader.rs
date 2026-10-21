//! The loader's state and operations (see the crate documentation).
//!
//! # Rules this implements (Learn, unless marked as Poly's choice)
//!
//! - **Mapping.** A DLL goes at its preferred base if that range is free,
//!   else anywhere (64 KiB aligned) and is relocated; an image without
//!   relocations whose base is taken fails with
//!   `STATUS_CONFLICTING_ADDRESSES` (Poly's choice of status). Only x64
//!   PE32+ images load (`STATUS_INVALID_IMAGE_FORMAT`). The headers are
//!   read-only, each section gets the protection its `IMAGE_SCN_MEM_*`
//!   flags say (a page two sections share gets both's rights).
//! - **Imports** are resolved DLL by DLL, symbol by symbol: by name (the
//!   hint tried first), or by ordinal; a forwarder ("DLL.Name",
//!   "DLL.#7") loads its DLL and resolves there, at most
//!   [`MAX_FORWARDS`] deep. A missing name is `STATUS_ENTRYPOINT_NOT_FOUND`,
//!   a missing ordinal `STATUS_ORDINAL_NOT_FOUND`, a missing DLL
//!   `STATUS_DLL_NOT_FOUND`. New-style bound imports (descriptor stamp -1
//!   and a bound import directory entry) whose DLL has the bound time stamp,
//!   sits at its preferred base and had no forwarders are kept as they are;
//!   everything else is rebound from the lookup tables.
//! - **Dependencies first.** A module's entry point and TLS callbacks run
//!   after those of every DLL it imports (a cycle is broken where it is
//!   found); detaching goes the other way. `DLL_PROCESS_ATTACH` gets
//!   `lpReserved` NULL for dynamic loads and non-NULL (Poly: 1) for the
//!   process's static imports; `DLL_PROCESS_DETACH` NULL for an unload and
//!   1 at process exit. A module's TLS callbacks come before its entry
//!   point, for attach and detach (Poly's choice).
//! - **Reference counts.** `LdrLoadDll` and `LdrAddRefDll` add one;
//!   `LdrUnloadDll` drops one. A module stays while it is pinned, has a
//!   reference, or is imported (or forwarded or delay-loaded into) by a
//!   module that stays; when an unload leaves modules that nothing keeps,
//!   they are detached and unmapped together, so cycles of DLLs that import
//!   each other go too. The executable and its static imports are pinned
//!   (never unloaded; Poly's choice). The count a module reports is its
//!   references plus the modules that import it.
//! - **TLS.** A module with a TLS directory gets the lowest free index,
//!   written to its `AddressOfIndex`. Every thread has an array of block
//!   pointers at `TEB.ThreadLocalStoragePointer` (TEB + 0x58), one block
//!   per index: a copy of the template plus the zero fill. A DLL loaded
//!   later gets blocks in every running thread too; threads that existed
//!   before it do not get `DLL_THREAD_ATTACH` for it (`DllMain`).

use crate::ldrdata::{self, EntryData};
use crate::path::{self, Dirs};
use crate::status::*;
use crate::{PAGE_SIZE, Process, page, read_u64, reason, write_u32, write_u64};
use alloc::vec;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_pe::{ExportTarget, ForwardTarget, ImportSymbol, Pe};

/// Most forwarders followed for one symbol.
pub const MAX_FORWARDS: usize = 16;
/// `TEB.ThreadLocalStoragePointer` (see the module documentation).
pub const TEB_TLS_POINTER: u64 = 0x58;
/// `LDR_ADDREF_DLL_PIN`: `LdrAddRefDll`'s flag to pin a module.
pub const ADDREF_PIN: u32 = 1;

/// A module's index in the loader.
pub type ModuleId = usize;

/// What a callout calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalloutKind {
    /// A TLS callback: `callback(DllHandle, Reason, Reserved)`.
    Tls,
    /// The entry point: `DllMain(hinstDLL, fdwReason, lpvReserved)`.
    Entry,
}

/// One call `ntdll` must make, in order, under the loader lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Callout {
    pub module: ModuleId,
    pub kind: CalloutKind,
    pub routine: u64,
    /// The module's base (its `HINSTANCE`).
    pub base: u64,
    pub reason: u32,
    pub reserved: u64,
}

/// The directories the search order uses.
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub dirs: Dirs,
}

/// What a public caller may know about a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleInfo {
    pub id: ModuleId,
    pub base: u64,
    pub size: u64,
    pub entry: u64,
    pub name: Vec<u16>,
    pub path: Vec<u16>,
    pub load_count: u32,
    pub pinned: bool,
    /// Its `LDR_DATA_TABLE_ENTRY`.
    pub record: u64,
    pub tls_index: Option<u32>,
    /// The exception directory (RVA, size).
    pub exception: (u32, u32),
    pub dll: bool,
}

/// The result of a call that may have loaded modules: the callouts to run
/// (attach of every new module, in order), and what [`Loader::fail_load`]
/// needs if one of them fails.
#[derive(Clone, Debug, Default)]
pub struct LoadOp {
    pub callouts: Vec<Callout>,
    new: Vec<ModuleId>,
    edges: Vec<(Option<ModuleId>, ModuleId)>,
}

impl LoadOp {
    /// The modules this call mapped, in initialisation order.
    pub fn new_modules(&self) -> &[ModuleId] {
        &self.new
    }
}

/// Modules to detach and then unmap: run [`Unload::callouts`], then pass
/// it to [`Loader::finish_unload`].
#[derive(Clone, Debug, Default)]
pub struct Unload {
    pub callouts: Vec<Callout>,
    modules: Vec<ModuleId>,
}

impl Unload {
    pub fn modules(&self) -> &[ModuleId] {
        &self.modules
    }
}

/// Why a delay-load import could not be resolved.
#[derive(Clone, Debug)]
pub struct DelayFailure {
    pub status: NtStatus,
    /// The DLL itself could not be loaded (else: the symbol was missing).
    pub dll_failed: bool,
    pub dll: Vec<u8>,
    /// The symbol: a name, or an ordinal.
    pub name: Option<Vec<u8>>,
    pub ordinal: u16,
    /// Modules loaded on the way (their callouts must still run).
    pub op: LoadOp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Mapped; imports being resolved.
    Loading,
    /// Bound; its attach callouts not issued yet.
    Ready,
    /// Its attach callouts were issued.
    Attached,
}

#[derive(Clone, Debug)]
struct TlsInfo {
    start: u64,
    size: u32,
    zero_fill: u32,
    align: u64,
    callbacks: Vec<u64>,
    index: u32,
}

#[derive(Clone, Debug)]
struct Module {
    base: u64,
    size: u64,
    preferred: u64,
    entry: u64,
    name: Vec<u16>,
    path: Vec<u16>,
    dll: bool,
    /// `LdrLoadDll` / `LdrAddRefDll` references.
    refs: u32,
    pinned: bool,
    deps: Vec<ModuleId>,
    state: State,
    tls: Option<TlsInfo>,
    thread_calls: bool,
    record: u64,
    /// The loader allocated its memory (else the kernel mapped it).
    owned: bool,
    exception: (u32, u32),
    time_date_stamp: u32,
    protections: Vec<(u32, u32, u32)>,
}

#[derive(Clone, Debug)]
struct ThreadTls {
    teb: u64,
    array: u64,
    cap: usize,
    /// Per TLS index: (allocation, aligned block), 0 when none.
    blocks: Vec<(u64, u64)>,
}

/// An import of one DLL, read out of an image.
#[derive(Clone, Debug)]
struct DllImports {
    name: Vec<u8>,
    stamp: u32,
    forwarder_chain: u32,
    symbols: Vec<Sym>,
    iat_rva: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Sym {
    Name { hint: u16, name: Vec<u8> },
    Ordinal(u16),
}

impl Sym {
    fn from(s: ImportSymbol<'_>) -> Sym {
        match s {
            ImportSymbol::Name { hint, name } => Sym::Name { hint, name: name.to_vec() },
            ImportSymbol::Ordinal(o) => Sym::Ordinal(o),
        }
    }

    fn missing(&self) -> NtStatus {
        match self {
            Sym::Name { .. } => STATUS_ENTRYPOINT_NOT_FOUND,
            Sym::Ordinal(_) => STATUS_ORDINAL_NOT_FOUND,
        }
    }
}

enum Found {
    Loaded(ModuleId),
    File(Vec<u16>, Vec<u8>),
}

fn bad(_: poly_pe::Error) -> NtStatus {
    STATUS_INVALID_IMAGE_FORMAT
}

fn ascii_wide(b: &[u8]) -> Vec<u16> {
    b.iter().map(|&c| u16::from(c)).collect()
}

/// The protection of each run of pages of an image (RVA, size, `PAGE_*`).
fn image_protections(img: &[u8]) -> Result<Vec<(u32, u32, u32)>, NtStatus> {
    let pe = Pe::parse_mapped(img).map_err(bad)?;
    let plan = pe.load_plan().map_err(bad)?;
    let pages = (plan.size_of_image as u64).div_ceil(PAGE_SIZE) as usize;
    let mut rights = vec![(false, false, false); pages];
    for region in plan.regions() {
        let first = (u64::from(region.rva) / PAGE_SIZE) as usize;
        let last = (u64::from(region.rva) + u64::from(region.size)).div_ceil(PAGE_SIZE).min(pages as u64) as usize;
        for r in rights.iter_mut().take(last).skip(first) {
            r.0 |= region.protection.read;
            r.1 |= region.protection.write;
            r.2 |= region.protection.execute;
        }
    }
    let prot = |(r, w, x): (bool, bool, bool)| match (r || w, w, x) {
        (_, true, true) => page::EXECUTE_READWRITE,
        (true, false, true) => page::EXECUTE_READ,
        (false, false, true) => page::EXECUTE,
        (_, true, false) => page::READWRITE,
        (true, false, false) => page::READONLY,
        (false, false, false) => page::NOACCESS,
    };
    let mut out: Vec<(u32, u32, u32)> = Vec::new();
    for (i, &r) in rights.iter().enumerate() {
        let p = prot(r);
        let rva = (i as u64 * PAGE_SIZE) as u32;
        match out.last_mut() {
            Some(last) if last.2 == p && last.0 + last.1 == rva => last.1 += PAGE_SIZE as u32,
            _ => out.push((rva, PAGE_SIZE as u32, p)),
        }
    }
    Ok(out)
}

/// A bound import directory entry: DLL name, time stamp, forwarder count.
type BoundEntry = (Vec<u8>, u32, usize);

/// Every import an image has.
fn gather_imports(img: &[u8]) -> Result<(Vec<DllImports>, Vec<BoundEntry>), NtStatus> {
    let pe = Pe::parse_mapped(img).map_err(bad)?;
    let mut out = Vec::new();
    if let Some(descs) = pe.imports().map_err(bad)? {
        for d in descs {
            let d = d.map_err(bad)?;
            let mut symbols = Vec::new();
            for e in d.entries().map_err(bad)? {
                symbols.push(Sym::from(e.map_err(bad)?.symbol));
            }
            out.push(DllImports {
                name: d.dll_name().map_err(bad)?.to_vec(),
                stamp: d.time_date_stamp,
                forwarder_chain: d.forwarder_chain,
                symbols,
                iat_rva: d.first_thunk,
            });
        }
    }
    let mut bound = Vec::new();
    if let Some(b) = pe.bound_imports().map_err(bad)? {
        for e in b {
            let e = e.map_err(bad)?;
            bound.push((e.module_name.to_vec(), e.time_date_stamp, e.forwarders().count()));
        }
    }
    Ok((out, bound))
}

/// The loader.
#[derive(Debug, Default)]
pub struct Loader {
    modules: Vec<Option<Module>>,
    load_order: Vec<ModuleId>,
    init_order: Vec<ModuleId>,
    dirs: Dirs,
    ldr: u64,
    tls_slots: Vec<Option<ModuleId>>,
    threads: Vec<ThreadTls>,
}

impl Loader {
    pub fn new(config: Config) -> Loader {
        Loader { dirs: config.dirs, ..Loader::default() }
    }

    /// Make `PEB_LDR_DATA` and point `PEB.Ldr` at it (`peb` 0: only make
    /// it). Call before adding modules.
    pub fn init_ldr_data(&mut self, p: &mut dyn Process, peb: u64) -> Result<u64, NtStatus> {
        self.ldr = ldrdata::create(p, peb)?;
        Ok(self.ldr)
    }

    /// The `PEB_LDR_DATA` address.
    pub fn ldr_data(&self) -> u64 {
        self.ldr
    }

    pub fn dirs(&self) -> &Dirs {
        &self.dirs
    }

    fn m(&self, id: ModuleId) -> &Module {
        self.modules[id].as_ref().expect("a live module")
    }

    fn mm(&mut self, id: ModuleId) -> &mut Module {
        self.modules[id].as_mut().expect("a live module")
    }

    fn info(&self, id: ModuleId) -> ModuleInfo {
        let m = self.m(id);
        ModuleInfo {
            id,
            base: m.base,
            size: m.size,
            entry: m.entry,
            name: m.name.clone(),
            path: m.path.clone(),
            load_count: m.refs + self.modules.iter().flatten().filter(|o| o.deps.contains(&id)).count() as u32,
            pinned: m.pinned,
            record: m.record,
            tls_index: m.tls.as_ref().map(|t| t.index),
            exception: m.exception,
            dll: m.dll,
        }
    }

    /// Every module, in load order.
    pub fn modules(&self) -> Vec<ModuleInfo> {
        self.load_order.iter().map(|&id| self.info(id)).collect()
    }

    /// The module with this id, if it is loaded.
    pub fn module(&self, id: ModuleId) -> Option<ModuleInfo> {
        self.modules.get(id).and_then(Option::as_ref).map(|_| self.info(id))
    }

    /// The module whose image contains `addr` (`LdrFindEntryForAddress`,
    /// `RtlPcToFileHeader`).
    pub fn find(&self, addr: u64) -> Option<ModuleInfo> {
        self.load_order
            .iter()
            .copied()
            .find(|&id| {
                let m = self.m(id);
                addr >= m.base && addr - m.base < m.size
            })
            .map(|id| self.info(id))
    }

    fn by_base(&self, base: u64) -> Option<ModuleId> {
        self.load_order.iter().copied().find(|&id| self.m(id).base == base)
    }

    fn find_loaded(&self, name: &[u16]) -> Option<ModuleId> {
        if path::has_path(name) {
            return self.load_order.iter().copied().find(|&id| path::same_name(&self.m(id).path, name));
        }
        self.load_order.iter().copied().find(|&id| path::same_name(&self.m(id).name, name))
    }

    /// `LdrGetDllHandle`: a loaded module's base by name (the `.dll` rule
    /// and API sets apply), else `STATUS_DLL_NOT_FOUND`.
    pub fn get_handle(&self, name: &[u16]) -> Result<u64, NtStatus> {
        let n = path::with_extension(name);
        let n = match crate::apiset::resolve(path::base_name(&n)) {
            crate::apiset::ApiSet::Host(h) => path::wide(h),
            crate::apiset::ApiSet::Absent => return Err(STATUS_DLL_NOT_FOUND),
            crate::apiset::ApiSet::NotApiSet => n,
        };
        self.find_loaded(&n).map(|id| self.m(id).base).ok_or(STATUS_DLL_NOT_FOUND)
    }

    /// Insert a module record and its `LDR_DATA_TABLE_ENTRY`.
    fn insert(&mut self, p: &mut dyn Process, mut m: Module) -> Result<ModuleId, NtStatus> {
        if self.ldr != 0 {
            let rec = ldrdata::create_entry(
                p,
                &EntryData {
                    base: m.base,
                    entry_point: m.entry,
                    size: m.size,
                    full_name: &m.path,
                    base_name: &m.name,
                    checksum: 0,
                    time_date_stamp: m.time_date_stamp,
                },
            )?;
            ldrdata::insert_tail(p, self.ldr + ldrdata::LDR_IN_LOAD_ORDER, rec + ldrdata::ENTRY_IN_LOAD_ORDER)?;
            ldrdata::insert_tail(p, self.ldr + ldrdata::LDR_IN_MEMORY_ORDER, rec + ldrdata::ENTRY_IN_MEMORY_ORDER)?;
            m.record = rec;
        }
        let id = match self.modules.iter().position(Option::is_none) {
            Some(i) => {
                self.modules[i] = Some(m);
                i
            }
            None => {
                self.modules.push(Some(m));
                self.modules.len() - 1
            }
        };
        self.load_order.push(id);
        Ok(id)
    }

    /// Read what the loader keeps from a mapped image's headers.
    fn describe(img: &[u8], base: u64, path: &[u16]) -> Result<Module, NtStatus> {
        let pe = Pe::parse_mapped(img).map_err(bad)?;
        let info = pe.info();
        let entry = match info.entry_point() {
            0 => 0,
            rva => base + u64::from(rva),
        };
        let exception = info.directory(poly_pe::dir::EXCEPTION).map_or((0, 0), |d| (d.virtual_address, d.size));
        Ok(Module {
            base,
            size: img.len() as u64,
            preferred: info.image_base(),
            entry,
            name: path::base_name(path).to_vec(),
            path: path.to_vec(),
            dll: info.is_dll(),
            refs: 0,
            pinned: false,
            deps: Vec::new(),
            state: State::Ready,
            tls: None,
            thread_calls: true,
            record: 0,
            owned: false,
            exception,
            time_date_stamp: info.file_header.time_date_stamp,
            protections: Vec::new(),
        })
    }

    /// Register an image the kernel mapped: `ntdll` itself (`bound`: its
    /// imports need nothing and it counts as attached) or the executable
    /// (bound later by [`process_init`](Self::process_init)). Both are
    /// pinned.
    pub fn add_mapped(
        &mut self,
        p: &mut dyn Process,
        base: u64,
        path: &[u16],
        bound: bool,
    ) -> Result<ModuleId, NtStatus> {
        let size = {
            let head = p.memory(base, 0x1000)?;
            let pe = Pe::parse_mapped(head).map_err(bad)?;
            u64::from(pe.info().size_of_image()).div_ceil(PAGE_SIZE) * PAGE_SIZE
        };
        let img = p.memory(base, size as usize)?;
        let mut m = Loader::describe(img, base, path)?;
        m.protections = image_protections(img)?;
        m.pinned = true;
        m.refs = 1;
        if bound {
            m.state = State::Attached;
        }
        let id = self.insert(p, m)?;
        if bound {
            self.init_order.push(id);
            self.link_init(p, id)?;
        }
        Ok(id)
    }

    fn link_init(&mut self, p: &mut dyn Process, id: ModuleId) -> Result<(), NtStatus> {
        let rec = self.m(id).record;
        if self.ldr != 0 && rec != 0 && self.m(id).dll {
            ldrdata::insert_tail(p, self.ldr + ldrdata::LDR_IN_INIT_ORDER, rec + ldrdata::ENTRY_IN_INIT_ORDER)?;
        }
        Ok(())
    }

    // ----- finding and mapping -------------------------------------------

    /// Find a module by the search rules: already loaded, or a file.
    fn locate(&self, p: &mut dyn Process, name: &[u16], search_path: Option<&[u16]>) -> Result<Found, NtStatus> {
        if name.is_empty() {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let mut n = path::with_extension(name);
        if !path::has_path(&n) {
            match crate::apiset::resolve(&n) {
                crate::apiset::ApiSet::Host(h) => n = path::wide(h),
                crate::apiset::ApiSet::Absent => return Err(STATUS_DLL_NOT_FOUND),
                crate::apiset::ApiSet::NotApiSet => {}
            }
        }
        if let Some(id) = self.find_loaded(&n) {
            return Ok(Found::Loaded(id));
        }
        let candidates = if path::has_path(&n) { vec![n] } else { path::candidates(&n, &self.dirs, search_path) };
        for c in candidates {
            if let Ok(bytes) = p.read_file(&path::nt_path(&c)) {
                return Ok(Found::File(c, bytes));
            }
        }
        Err(STATUS_DLL_NOT_FOUND)
    }

    /// The module `name` (an import's or forwarder's DLL), loading it if
    /// needed as part of `op`.
    fn dependency(&mut self, p: &mut dyn Process, name: &[u16], op: &mut LoadOp) -> Result<ModuleId, NtStatus> {
        match self.locate(p, name, None)? {
            Found::Loaded(id) => Ok(id),
            Found::File(path, bytes) => self.map_new(p, &path, &bytes, op),
        }
    }

    /// `from` holds a reference on `to` (`None`: a caller of `LdrLoadDll`).
    fn add_edge(&mut self, from: Option<ModuleId>, to: ModuleId, op: &mut LoadOp) {
        if let Some(f) = from {
            if f == to || self.m(f).deps.contains(&to) {
                return;
            }
            self.mm(f).deps.push(to);
        }
        if from.is_none() {
            let t = self.mm(to);
            t.refs = t.refs.saturating_add(1);
        }
        op.edges.push((from, to));
    }

    /// Map a new image file and bind it (and what it imports).
    fn map_new(
        &mut self,
        p: &mut dyn Process,
        path: &[u16],
        file: &[u8],
        op: &mut LoadOp,
    ) -> Result<ModuleId, NtStatus> {
        let pe = Pe::parse(file).map_err(bad)?;
        let info = *pe.info();
        if info.machine() != poly_pe::machine::AMD64 || info.bits() != poly_pe::Bits::Pe32Plus {
            return Err(STATUS_INVALID_IMAGE_FORMAT);
        }
        let plan = pe.load_plan().map_err(bad)?;
        let size = u64::from(plan.size_of_image).div_ceil(PAGE_SIZE) * PAGE_SIZE;
        let mut image = vec![0u8; size as usize];
        plan.map_into(&mut image).map_err(bad)?;
        let base = match p.allocate(Some(plan.image_base), size) {
            Ok(b) => b,
            Err(_) if plan.relocatable => p.allocate(None, size)?,
            Err(_) => return Err(STATUS_CONFLICTING_ADDRESSES),
        };
        let mapped = (|| -> Result<(), NtStatus> {
            if base != plan.image_base {
                poly_pe::relocate(&mut image, &info, base).map_err(bad)?;
            }
            let mem = p.memory(base, size as usize)?;
            mem.copy_from_slice(&image);
            Ok(())
        })();
        if let Err(e) = mapped {
            p.free(base);
            return Err(e);
        }
        drop(image);
        let mut m = match Loader::describe(p.memory(base, size as usize)?, base, path) {
            Ok(m) => m,
            Err(e) => {
                p.free(base);
                return Err(e);
            }
        };
        m.owned = true;
        m.preferred = plan.image_base;
        m.state = State::Loading;
        let id = match self.insert(p, m) {
            Ok(id) => id,
            Err(e) => {
                p.free(base);
                return Err(e);
            }
        };
        op.new.push(id);
        self.bind(p, id, op)?;
        let img = p.memory(base, size as usize)?;
        let prot = image_protections(img)?;
        self.mm(id).protections = prot;
        self.assign_tls(p, id)?;
        self.apply_protections(p, id)?;
        self.mm(id).state = State::Ready;
        // `op.new` is in mapping order; initialisation order puts a module
        // after its dependencies, which finish binding first.
        op.new.retain(|&x| x != id);
        op.new.push(id);
        Ok(id)
    }

    fn apply_protections(&mut self, p: &mut dyn Process, id: ModuleId) -> Result<(), NtStatus> {
        let (base, prot) = (self.m(id).base, self.m(id).protections.clone());
        for (rva, size, pr) in prot {
            p.protect(base + u64::from(rva), u64::from(size), pr)?;
        }
        Ok(())
    }

    /// Resolve and write a module's imports, in place.
    fn bind(&mut self, p: &mut dyn Process, id: ModuleId, op: &mut LoadOp) -> Result<(), NtStatus> {
        let (base, size) = (self.m(id).base, self.m(id).size);
        let (imports, bound) = gather_imports(p.memory(base, size as usize)?)?;
        let mut addrs: Vec<Vec<u64>> = Vec::new();
        for dll in &imports {
            let dep = self.dependency(p, &ascii_wide(&dll.name), op)?;
            self.add_edge(Some(id), dep, op);
            if self.bound_still_valid(dll, &bound, dep) {
                let mut v = Vec::new();
                for j in 0..dll.symbols.len() {
                    v.push(read_u64(p, base + u64::from(dll.iat_rva) + 8 * j as u64)?);
                }
                addrs.push(v);
                continue;
            }
            let mut v = Vec::new();
            for s in &dll.symbols {
                v.push(self.export_address(p, id, dep, s, op, 0)?.ok_or(s.missing())?);
            }
            addrs.push(v);
        }
        if imports.is_empty() {
            return Ok(());
        }
        let img = p.memory(base, size as usize)?;
        let info = *Pe::parse_mapped(img).map_err(bad)?.info();
        poly_pe::bind_imports(img, &info, |req| {
            addrs.get(req.module_index).and_then(|v| v.get(req.index)).copied().ok_or(())
        })
        .map_err(|_| STATUS_INVALID_IMAGE_FORMAT)?;
        Ok(())
    }

    /// Whether an import of `dep` may keep its bound addresses.
    fn bound_still_valid(&self, dll: &DllImports, bound: &[BoundEntry], dep: ModuleId) -> bool {
        let d = self.m(dep);
        if d.base != d.preferred {
            return false;
        }
        match dll.stamp {
            0 => false,
            0xFFFF_FFFF => bound.iter().any(|(name, stamp, forwarders)| {
                path::same_name(&ascii_wide(name), &ascii_wide(&dll.name))
                    && *stamp == d.time_date_stamp
                    && *forwarders == 0
            }),
            stamp => stamp == d.time_date_stamp && dll.forwarder_chain == u32::MAX,
        }
    }

    /// The address `sym` resolves to in module `m`, following forwarders
    /// (each forwarder's DLL becomes a dependency of `importer`). `None`
    /// if the final module does not export it.
    fn export_address(
        &mut self,
        p: &mut dyn Process,
        importer: ModuleId,
        m: ModuleId,
        sym: &Sym,
        op: &mut LoadOp,
        depth: usize,
    ) -> Result<Option<u64>, NtStatus> {
        if depth > MAX_FORWARDS {
            return Err(STATUS_ENTRYPOINT_NOT_FOUND);
        }
        let (base, size) = (self.m(m).base, self.m(m).size);
        let target = {
            let img = p.memory(base, size as usize)?;
            let pe = Pe::parse_mapped(img).map_err(bad)?;
            let Some(ex) = pe.exports().map_err(bad)? else { return Ok(None) };
            let t = match sym {
                Sym::Name { hint, name } => ex.by_name_with_hint(name, *hint),
                Sym::Ordinal(o) => ex.by_ordinal(u32::from(*o)),
            };
            match t.map_err(bad)? {
                None => return Ok(None),
                Some(ExportTarget::Rva(r)) => return Ok(Some(base + u64::from(r))),
                Some(ExportTarget::Forwarder(f)) => {
                    let next = match f.target {
                        ForwardTarget::Name(n) => Sym::Name { hint: 0, name: n.to_vec() },
                        ForwardTarget::Ordinal(o) => Sym::Ordinal(o),
                    };
                    (f.dll.to_vec(), next)
                }
            }
        };
        let (dll, next) = target;
        let dep = self.dependency(p, &ascii_wide(&dll), op)?;
        self.add_edge(Some(importer), dep, op);
        self.export_address(p, importer, dep, &next, op, depth + 1)
    }

    // ----- TLS -------------------------------------------------------------

    fn assign_tls(&mut self, p: &mut dyn Process, id: ModuleId) -> Result<(), NtStatus> {
        let (base, size) = (self.m(id).base, self.m(id).size);
        let parsed = {
            let img = p.memory(base, size as usize)?;
            let pe = Pe::parse_mapped(img).map_err(bad)?;
            match pe.tls().map_err(bad)? {
                None => None,
                Some(t) => {
                    let cbs: Result<Vec<u64>, _> = pe.tls_callbacks(&t).map_err(bad)?.take(1024).collect();
                    let index_rva = pe.tls_index_rva(&t).map_err(bad)?;
                    // The template must lie in the image.
                    let tsize = t.template_size().map_err(bad)?;
                    if tsize > 0 {
                        pe.tls_template(&t).map_err(bad)?;
                    }
                    Some((t, cbs.map_err(bad)?, index_rva, tsize))
                }
            }
        };
        let Some((t, callbacks, index_rva, tsize)) = parsed else { return Ok(()) };
        if u64::from(index_rva) + 4 > size || callbacks.iter().any(|&c| c < base || c - base >= size) {
            return Err(STATUS_INVALID_IMAGE_FORMAT);
        }
        let index = match self.tls_slots.iter().position(Option::is_none) {
            Some(i) => {
                self.tls_slots[i] = Some(id);
                i
            }
            None => {
                self.tls_slots.push(Some(id));
                self.tls_slots.len() - 1
            }
        } as u32;
        write_u32(p, base + u64::from(index_rva), index)?;
        let align = u64::from(t.alignment().unwrap_or(16)).max(16);
        self.mm(id).tls = Some(TlsInfo {
            start: t.start_address_of_raw_data,
            size: tsize,
            zero_fill: t.size_of_zero_fill,
            align,
            callbacks,
            index,
        });
        for i in 0..self.threads.len() {
            self.give_block(p, i, id)?;
        }
        Ok(())
    }

    /// Thread `t` gets a block for module `id`'s TLS index.
    fn give_block(&mut self, p: &mut dyn Process, t: usize, id: ModuleId) -> Result<(), NtStatus> {
        let Some(tls) = self.m(id).tls.clone() else { return Ok(()) };
        let total = (tls.size as usize).checked_add(tls.zero_fill as usize).ok_or(STATUS_NO_MEMORY)?;
        let raw = p.alloc(total.max(1) + (tls.align as usize - 16))?;
        let block = raw.div_ceil(tls.align) * tls.align;
        if tls.size > 0 {
            let template = p.memory(tls.start, tls.size as usize)?.to_vec();
            crate::write_bytes(p, block, &template)?;
        }
        let index = tls.index as usize;
        let th = &mut self.threads[t];
        if index >= th.cap {
            let cap = (index + 1).max(th.cap * 2).max(4);
            let array = p.alloc(cap * 8)?;
            for (i, &(_, b)) in self.threads[t].blocks.iter().enumerate() {
                write_u64(p, array + 8 * i as u64, b)?;
            }
            let th = &mut self.threads[t];
            let old = th.array;
            th.array = array;
            th.cap = cap;
            let teb = th.teb;
            write_u64(p, teb + TEB_TLS_POINTER, array)?;
            if old != 0 {
                p.free_alloc(old);
            }
        }
        let th = &mut self.threads[t];
        if th.blocks.len() <= index {
            th.blocks.resize(index + 1, (0, 0));
        }
        th.blocks[index] = (raw, block);
        let array = th.array;
        write_u64(p, array + 8 * index as u64, block)
    }

    /// Free every thread's block for module `id` and its index.
    fn drop_tls(&mut self, p: &mut dyn Process, id: ModuleId) {
        let Some(index) = self.m(id).tls.as_ref().map(|t| t.index as usize) else { return };
        for th in &mut self.threads {
            if let Some(&(raw, _)) = th.blocks.get(index)
                && raw != 0
            {
                p.free_alloc(raw);
                th.blocks[index] = (0, 0);
                let _ = write_u64(p, th.array + 8 * index as u64, 0);
            }
        }
        if let Some(s) = self.tls_slots.get_mut(index) {
            *s = None;
        }
    }

    // ----- callouts --------------------------------------------------------

    fn callouts_for(&self, id: ModuleId, reason: u32, reserved: u64, out: &mut Vec<Callout>) {
        let m = self.m(id);
        let thread = matches!(reason, reason::THREAD_ATTACH | reason::THREAD_DETACH);
        if thread && !m.thread_calls {
            return;
        }
        if let Some(t) = &m.tls {
            for &cb in &t.callbacks {
                out.push(Callout { module: id, kind: CalloutKind::Tls, routine: cb, base: m.base, reason, reserved });
            }
        }
        if m.dll && m.entry != 0 {
            out.push(Callout {
                module: id,
                kind: CalloutKind::Entry,
                routine: m.entry,
                base: m.base,
                reason,
                reserved,
            });
        }
    }

    /// Issue the attach callouts of every module `op` made ready.
    fn attach(&mut self, p: &mut dyn Process, op: &mut LoadOp, reserved: u64) -> Result<(), NtStatus> {
        for &id in &op.new.clone() {
            if self.m(id).state != State::Ready {
                continue;
            }
            self.mm(id).state = State::Attached;
            self.init_order.push(id);
            self.link_init(p, id)?;
            self.callouts_for(id, reason::PROCESS_ATTACH, reserved, &mut op.callouts);
        }
        Ok(())
    }

    /// Undo `op`: its references on modules that stay, and every module it
    /// mapped (which are returned, for [`finish_unload`](Self::finish_unload)).
    fn undo(&mut self, op: &LoadOp) -> Vec<ModuleId> {
        for &(from, to) in op.edges.iter().rev() {
            if op.new.contains(&to) {
                continue;
            }
            if let Some(f) = from
                && !op.new.contains(&f)
            {
                self.mm(f).deps.retain(|&d| d != to);
            }
            if from.is_none() {
                let t = self.mm(to);
                t.refs = t.refs.saturating_sub(1);
            }
        }
        self.init_order.retain(|id| !op.new.contains(id));
        op.new.clone()
    }

    fn rollback(&mut self, p: &mut dyn Process, op: &LoadOp) {
        let modules = self.undo(op);
        self.finish_unload(p, Unload { callouts: Vec::new(), modules });
    }

    // ----- public operations ------------------------------------------------

    /// Process start: register the first thread (`teb`, for TLS), bind the
    /// executable's imports (loading its static DLLs, pinned), give it its
    /// TLS index, and return the `DLL_PROCESS_ATTACH` callouts (the DLLs in
    /// dependency order, then the executable's TLS callbacks).
    pub fn process_init(&mut self, p: &mut dyn Process, exe: ModuleId, teb: u64) -> Result<Vec<Callout>, NtStatus> {
        self.thread_register(p, teb)?;
        let mut op = LoadOp::default();
        let (base, size) = (self.m(exe).base, self.m(exe).size);
        p.protect(base, size, page::READWRITE)?;
        self.bind(p, exe, &mut op)?;
        self.assign_tls(p, exe)?;
        self.apply_protections(p, exe)?;
        for &id in &op.new {
            self.mm(id).pinned = true;
        }
        self.attach(p, &mut op, 1)?;
        self.mm(exe).state = State::Attached;
        self.init_order.push(exe);
        self.callouts_for(exe, reason::PROCESS_ATTACH, 1, &mut op.callouts);
        Ok(op.callouts)
    }

    /// `LdrLoadDll`: load `name` (or take another reference on it) and
    /// return its id, base and the attach callouts of every module it
    /// brought in. On failure nothing it mapped stays.
    pub fn load(
        &mut self,
        p: &mut dyn Process,
        name: &[u16],
        search_path: Option<&[u16]>,
    ) -> Result<(ModuleId, u64, LoadOp), NtStatus> {
        let mut op = LoadOp::default();
        let id = match self.locate(p, name, search_path)? {
            Found::Loaded(id) => id,
            Found::File(path, bytes) => match self.map_new(p, &path, &bytes, &mut op) {
                Ok(id) => id,
                Err(e) => {
                    self.rollback(p, &op);
                    return Err(e);
                }
            },
        };
        self.add_edge(None, id, &mut op);
        self.attach(p, &mut op, 0)?;
        Ok((id, self.m(id).base, op))
    }

    /// A `DLL_PROCESS_ATTACH` callout of `op` (for module `failed`)
    /// returned `FALSE`. Returns the clean-up: `failed` gets
    /// `DLL_PROCESS_DETACH` at once (`DllMain`), the modules of `op` whose
    /// attach had already run are detached in reverse, and every module of
    /// `op` is unmapped by [`finish_unload`](Self::finish_unload). The
    /// caller then fails with `STATUS_DLL_INIT_FAILED`.
    pub fn fail_load(&mut self, op: &LoadOp, failed: ModuleId) -> Unload {
        let mut callouts = Vec::new();
        if let Some(m) = self.modules.get(failed).and_then(Option::as_ref)
            && m.dll
            && m.entry != 0
        {
            callouts.push(Callout {
                module: failed,
                kind: CalloutKind::Entry,
                routine: m.entry,
                base: m.base,
                reason: reason::PROCESS_DETACH,
                reserved: 0,
            });
        }
        let done: Vec<ModuleId> = op.new.iter().copied().take_while(|&id| id != failed).collect();
        for &id in done.iter().rev() {
            self.callouts_for(id, reason::PROCESS_DETACH, 0, &mut callouts);
        }
        let modules = self.undo(op);
        Unload { callouts, modules }
    }

    /// `LdrUnloadDll`: drop a reference. When it reaches zero the module,
    /// and the dependencies that reach zero with it, are to be detached
    /// (the returned callouts, dependents first) and then unmapped
    /// ([`finish_unload`](Self::finish_unload)).
    pub fn unload(&mut self, base: u64) -> Result<Unload, NtStatus> {
        let id = self.by_base(base).ok_or(STATUS_DLL_NOT_FOUND)?;
        let m = self.mm(id);
        if m.pinned || m.refs == 0 {
            return Ok(Unload::default());
        }
        m.refs -= 1;
        if m.refs > 0 {
            return Ok(Unload::default());
        }
        // What stays: pinned or referenced modules and what they import.
        let mut alive: Vec<ModuleId> =
            self.load_order.iter().copied().filter(|&x| self.m(x).pinned || self.m(x).refs > 0).collect();
        let mut i = 0;
        while i < alive.len() {
            for d in self.m(alive[i]).deps.clone() {
                if !alive.contains(&d) {
                    alive.push(d);
                }
            }
            i += 1;
        }
        let gone: Vec<ModuleId> = self.load_order.iter().copied().filter(|x| !alive.contains(x)).collect();
        // Detach in reverse initialisation order.
        let mut ordered: Vec<ModuleId> = self.init_order.iter().rev().copied().filter(|x| gone.contains(x)).collect();
        for &x in &gone {
            if !ordered.contains(&x) {
                ordered.push(x);
            }
        }
        let mut callouts = Vec::new();
        for &x in &ordered {
            if self.m(x).state == State::Attached {
                self.callouts_for(x, reason::PROCESS_DETACH, 0, &mut callouts);
            }
        }
        self.init_order.retain(|x| !gone.contains(x));
        Ok(Unload { callouts, modules: ordered })
    }

    /// Unmap what an [`Unload`] names: TLS, loader entries, memory.
    pub fn finish_unload(&mut self, p: &mut dyn Process, u: Unload) {
        for &id in &u.modules {
            if self.modules.get(id).and_then(Option::as_ref).is_none() {
                continue;
            }
            self.drop_tls(p, id);
            let m = self.modules[id].take().expect("checked");
            if m.record != 0 {
                let _ = ldrdata::destroy_entry(p, m.record);
            }
            if m.owned {
                p.free(m.base);
            }
            self.load_order.retain(|&x| x != id);
            self.init_order.retain(|&x| x != id);
            for other in self.modules.iter_mut().flatten() {
                other.deps.retain(|&d| d != id);
            }
        }
    }

    /// `LdrAddRefDll`: one more reference, or pin it (`ADDREF_PIN`).
    pub fn add_ref(&mut self, base: u64, flags: u32) -> Result<(), NtStatus> {
        let id = self.by_base(base).ok_or(STATUS_DLL_NOT_FOUND)?;
        let m = self.mm(id);
        if flags & ADDREF_PIN != 0 {
            m.pinned = true;
        } else {
            m.refs = m.refs.saturating_add(1);
        }
        Ok(())
    }

    /// `LdrDisableThreadCalloutsForDll`.
    pub fn disable_thread_calls(&mut self, base: u64) -> Result<(), NtStatus> {
        let id = self.by_base(base).ok_or(STATUS_DLL_NOT_FOUND)?;
        self.mm(id).thread_calls = false;
        Ok(())
    }

    /// `LdrGetProcedureAddress`: by name, or by ordinal when `name` is
    /// `None`. A forwarder may load its DLL (callouts in the op).
    pub fn get_procedure(
        &mut self,
        p: &mut dyn Process,
        base: u64,
        name: Option<&[u8]>,
        ordinal: u16,
    ) -> Result<(u64, LoadOp), NtStatus> {
        let id = self.by_base(base).ok_or(STATUS_DLL_NOT_FOUND)?;
        let sym = match name {
            Some(n) => Sym::Name { hint: 0, name: n.to_vec() },
            None => Sym::Ordinal(ordinal),
        };
        let mut op = LoadOp::default();
        match self.export_address(p, id, id, &sym, &mut op, 0) {
            Ok(Some(a)) => {
                self.attach(p, &mut op, 0)?;
                Ok((a, op))
            }
            r => {
                self.rollback(p, &op);
                let missing = if name.is_some() { STATUS_PROCEDURE_NOT_FOUND } else { STATUS_ORDINAL_NOT_FOUND };
                Err(r.err().unwrap_or(missing))
            }
        }
    }

    /// `LdrResolveDelayLoadedAPI`: resolve the delay-load import whose
    /// address table slot is `thunk` (a VA in module `parent`), described
    /// by the descriptor at `descriptor` (spec "Delay-Load Directory Table";
    /// its fields are RVAs). Loads the DLL on first use (it stays loaded
    /// while the parent is), stores its handle and the address, and
    /// returns the address with the attach callouts of anything loaded.
    pub fn resolve_delay(
        &mut self,
        p: &mut dyn Process,
        parent: u64,
        descriptor: u64,
        thunk: u64,
    ) -> Result<(u64, LoadOp), alloc::boxed::Box<DelayFailure>> {
        let fail = |status, dll: Vec<u8>, sym: Option<&Sym>, op: LoadOp, dll_failed| {
            alloc::boxed::Box::new(DelayFailure {
                status,
                dll_failed,
                dll,
                name: match sym {
                    Some(Sym::Name { name, .. }) => Some(name.clone()),
                    _ => None,
                },
                ordinal: match sym {
                    Some(Sym::Ordinal(o)) => *o,
                    _ => 0,
                },
                op,
            })
        };
        let Some(pid) = self.by_base(parent) else {
            return Err(fail(STATUS_INVALID_PARAMETER, Vec::new(), None, LoadOp::default(), true));
        };
        let (base, size) = (self.m(pid).base, self.m(pid).size);
        let parsed = (|| -> Result<(Vec<u8>, u64, u64, Sym), NtStatus> {
            let img = p.memory(base, size as usize)?;
            let at = descriptor.checked_sub(base).filter(|&o| o + 32 <= size).ok_or(STATUS_INVALID_PARAMETER)? as usize;
            let d = &img[at..at + 32];
            let field = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap_or([0; 4]));
            let (name_rva, handle_rva, iat_rva, int_rva) = (field(4), field(8), field(12), field(16));
            let index = thunk
                .checked_sub(base + u64::from(iat_rva))
                .filter(|o| o % 8 == 0 && thunk < base + size)
                .ok_or(STATUS_INVALID_PARAMETER)?
                / 8;
            let cstr = |rva: u64| -> Result<Vec<u8>, NtStatus> {
                let s = img.get(rva as usize..).ok_or(STATUS_INVALID_IMAGE_FORMAT)?;
                let n =
                    s.iter().take(poly_pe::MAX_NAME_LEN).position(|&c| c == 0).ok_or(STATUS_INVALID_IMAGE_FORMAT)?;
                Ok(s[..n].to_vec())
            };
            let dll = cstr(u64::from(name_rva))?;
            let entry_at = (u64::from(int_rva) + 8 * index) as usize;
            let raw = img
                .get(entry_at..entry_at + 8)
                .map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
                .ok_or(STATUS_INVALID_IMAGE_FORMAT)?;
            let sym = if raw & (1 << 63) != 0 {
                Sym::Ordinal(raw as u16)
            } else {
                let hn = raw & 0x7FFF_FFFF;
                let hint = img.get(hn as usize..hn as usize + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
                Sym::Name { hint, name: cstr(hn + 2)? }
            };
            Ok((dll, base + u64::from(handle_rva), base + u64::from(iat_rva) + 8 * index, sym))
        })();
        let (dll, handle_slot, slot, sym) = match parsed {
            Ok(x) => x,
            Err(e) => return Err(fail(e, Vec::new(), None, LoadOp::default(), true)),
        };
        let mut op = LoadOp::default();
        let handle = match read_u64(p, handle_slot) {
            Ok(h) => h,
            Err(e) => return Err(fail(e, dll, Some(&sym), op, true)),
        };
        let dep = match self.by_base(handle).filter(|_| handle != 0) {
            Some(d) => d,
            None => match self.dependency(p, &ascii_wide(&dll), &mut op) {
                Ok(d) => {
                    self.add_edge(Some(pid), d, &mut op);
                    let b = self.m(d).base;
                    if let Err(e) = self.write_slot(p, pid, handle_slot, b) {
                        self.rollback(p, &op);
                        return Err(fail(e, dll, Some(&sym), LoadOp::default(), true));
                    }
                    d
                }
                Err(e) => {
                    self.rollback(p, &op);
                    return Err(fail(e, dll, Some(&sym), LoadOp::default(), true));
                }
            },
        };
        let resolved = self.export_address(p, pid, dep, &sym, &mut op, 0);
        if self.attach(p, &mut op, 0).is_err() {
            return Err(fail(STATUS_NO_MEMORY, dll, Some(&sym), op, false));
        }
        match resolved {
            Ok(Some(a)) => match self.write_slot(p, pid, slot, a) {
                Ok(()) => Ok((a, op)),
                Err(e) => Err(fail(e, dll, Some(&sym), op, false)),
            },
            Ok(None) => {
                let status =
                    if matches!(sym, Sym::Name { .. }) { STATUS_PROCEDURE_NOT_FOUND } else { STATUS_ORDINAL_NOT_FOUND };
                Err(fail(status, dll, Some(&sym), op, false))
            }
            Err(e) => Err(fail(e, dll, Some(&sym), op, false)),
        }
    }

    /// Write a pointer into module `id`'s image, making its page writable
    /// for the write if its section is not.
    fn write_slot(&mut self, p: &mut dyn Process, id: ModuleId, at: u64, v: u64) -> Result<(), NtStatus> {
        let m = self.m(id);
        let rva = (at - m.base) as u32;
        let prot = m.protections.iter().find(|&&(r, s, _)| rva >= r && rva - r < s).map_or(page::READONLY, |x| x.2);
        let writable = matches!(prot, page::READWRITE | page::EXECUTE_READWRITE);
        let pg = at & !(PAGE_SIZE - 1);
        if !writable {
            p.protect(pg, PAGE_SIZE, page::READWRITE)?;
        }
        let r = write_u64(p, at, v);
        if !writable {
            p.protect(pg, PAGE_SIZE, prot)?;
        }
        r
    }

    // ----- threads and process exit ---------------------------------------

    fn thread_register(&mut self, p: &mut dyn Process, teb: u64) -> Result<usize, NtStatus> {
        if let Some(i) = self.threads.iter().position(|t| t.teb == teb) {
            return Ok(i);
        }
        self.threads.push(ThreadTls { teb, array: 0, cap: 0, blocks: Vec::new() });
        let t = self.threads.len() - 1;
        let with_tls: Vec<ModuleId> = self.tls_slots.iter().flatten().copied().collect();
        for id in with_tls {
            self.give_block(p, t, id)?;
        }
        Ok(t)
    }

    /// A new thread (`teb`) starts: its TLS blocks, then the
    /// `DLL_THREAD_ATTACH` callouts in initialisation order.
    pub fn thread_attach(&mut self, p: &mut dyn Process, teb: u64) -> Result<Vec<Callout>, NtStatus> {
        self.thread_register(p, teb)?;
        let mut out = Vec::new();
        for &id in &self.init_order {
            self.callouts_for(id, reason::THREAD_ATTACH, 0, &mut out);
        }
        Ok(out)
    }

    /// A thread is ending: its `DLL_THREAD_DETACH` callouts, in reverse
    /// initialisation order. Run them, then [`thread_exit`](Self::thread_exit).
    pub fn thread_detach(&self) -> Vec<Callout> {
        let mut out = Vec::new();
        for &id in self.init_order.iter().rev() {
            self.callouts_for(id, reason::THREAD_DETACH, 0, &mut out);
        }
        out
    }

    /// Free a thread's TLS.
    pub fn thread_exit(&mut self, p: &mut dyn Process, teb: u64) {
        let Some(i) = self.threads.iter().position(|t| t.teb == teb) else { return };
        let t = self.threads.swap_remove(i);
        for (raw, _) in t.blocks {
            if raw != 0 {
                p.free_alloc(raw);
            }
        }
        if t.array != 0 {
            let _ = write_u64(p, teb + TEB_TLS_POINTER, 0);
            p.free_alloc(t.array);
        }
    }

    /// `LdrShutdownProcess`: the `DLL_PROCESS_DETACH` callouts at exit, in
    /// reverse initialisation order, `lpReserved` 1.
    pub fn process_detach(&mut self) -> Vec<Callout> {
        let mut out = Vec::new();
        for &id in self.init_order.iter().rev() {
            self.callouts_for(id, reason::PROCESS_DETACH, 1, &mut out);
        }
        self.init_order.clear();
        out
    }

    /// A TLS index's block for a thread (tests and diagnostics).
    pub fn tls_block(&self, teb: u64, index: u32) -> Option<u64> {
        let t = self.threads.iter().find(|t| t.teb == teb)?;
        t.blocks.get(index as usize).map(|b| b.1).filter(|&b| b != 0)
    }

    /// The modules in initialisation order (tests and diagnostics).
    pub fn init_order(&self) -> Vec<ModuleId> {
        self.init_order.clone()
    }
}
