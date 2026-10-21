//! Sections and their views: `NtCreateSection`, `NtOpenSection`,
//! `NtMapViewOfSection`, `NtUnmapViewOfSection`, after Learn
//! ("ZwCreateSection", "ZwOpenSection", "ZwMapViewOfSection",
//! "ZwUnmapViewOfSection", "CreateFileMappingW", "MapViewOfFile").
//!
//! - A pagefile-backed section is `MaximumSize` bytes of zeros shared by
//!   all its views, in every process; a data file section maps a file
//!   object's pages; an image section ([`crate::image`]) maps private
//!   copies of a laid-out PE image with per-page protections.
//! - A view holds a pointer reference on its section, so a section lives
//!   while it is mapped even after its last handle closed.
//! - Views start at 64 KiB boundaries; the section offset is rounded down
//!   to 64 KiB and the view grows by the difference ("the offset is rounded
//!   down to the next allocation-granularity size boundary").

use crate::backend::{Backend, SectionSource, ViewSource};
use crate::consts::*;
use crate::image::ImageSection;
use crate::protect::{Protection, view_allowed};
use crate::system::{NtProcess, NtSystem, Section, SectionKind};
use crate::vm::{ALLOCATION_GRANULARITY, AllocKind, NewView, PAGE_SIZE, PageState, ViewInfo, map_view_pages, page_up};
use alloc::vec;
use core::result::Result;
use poly_ntobj::access::*;
use poly_ntobj::status::*;
use poly_ntobj::{
    AccessMask, Caller, Created, Handle, NewObject, NoParse, NtStatus, ObjectAttributes, ObjectId, ObjectType,
};

/// The largest pagefile-backed section we make: 1 TiB (ours; Windows'
/// limit depends on the commit limit). Larger is `STATUS_SECTION_TOO_BIG`.
pub const MAX_SECTION_SIZE: u64 = 1 << 40;

/// What `NtCreateSection` was asked for.
#[derive(Clone, Copy, Debug)]
pub struct CreateSection<'a> {
    pub attrs: &'a ObjectAttributes<'a>,
    pub desired: AccessMask,
    /// `*MaximumSize`, if the pointer was not null.
    pub maximum_size: Option<u64>,
    pub page_protection: u32,
    pub allocation_attributes: u32,
    /// `FileHandle`, if not null.
    pub file: Option<Handle>,
}

/// `NtCreateSection`.
///
/// - `AllocationAttributes` must hold exactly one of `SEC_COMMIT`,
///   `SEC_RESERVE` and `SEC_IMAGE`, and only known `SEC_*` bits
///   (`STATUS_INVALID_PARAMETER_6`); `SEC_LARGE_PAGES` is
///   `STATUS_NOT_SUPPORTED`. To be confirmed by the spec agent.
/// - `SectionPageProtection` is a `PAGE_*` value without `PAGE_GUARD`
///   (`STATUS_INVALID_PAGE_PROTECTION`).
/// - Without a file: `MaximumSize` is required and not 0
///   (`STATUS_INVALID_PARAMETER_4`), at most [`MAX_SECTION_SIZE`]; an image
///   needs a file (`STATUS_INVALID_FILE_FOR_SECTION`). Statuses to be
///   confirmed.
/// - With a file (a File object, else `STATUS_OBJECT_TYPE_MISMATCH`): an
///   image is parsed from its contents (`STATUS_INVALID_IMAGE_FORMAT`); a
///   data section is the file's size, or `MaximumSize` if that is not
///   larger; a larger one grows a file of NT file I/O opened for writing
///   ([`io_file_section`]) and is `STATUS_SECTION_TOO_BIG` otherwise; an
///   empty file is `STATUS_MAPPED_FILE_SIZE_ZERO`.
/// - Named, and the name is taken: as for every object (`OBJ_OPENIF` opens
///   the existing section; the new one is dropped).
pub fn create_section(
    sys: &mut NtSystem,
    b: &mut dyn Backend,
    caller: &Caller,
    req: &CreateSection<'_>,
) -> Result<Created, NtStatus> {
    const KNOWN: u32 =
        SEC_FILE | SEC_IMAGE | SEC_RESERVE | SEC_COMMIT | SEC_NOCACHE | SEC_WRITECOMBINE | SEC_LARGE_PAGES;
    let a = req.allocation_attributes;
    let kinds = [SEC_COMMIT, SEC_RESERVE, SEC_IMAGE].iter().filter(|&&k| a & k != 0).count();
    if a & !KNOWN != 0 || kinds != 1 {
        return Err(STATUS_INVALID_PARAMETER_6);
    }
    if a & SEC_LARGE_PAGES != 0 {
        return Err(STATUS_NOT_SUPPORTED);
    }
    let protect = Protection::new(req.page_protection, false)?;
    if protect.is_guard() {
        return Err(STATUS_INVALID_PAGE_PROTECTION);
    }
    let image = a & SEC_IMAGE != 0;
    let (size, kind, source) = match req.file {
        None => {
            if image {
                return Err(STATUS_INVALID_FILE_FOR_SECTION);
            }
            let size = req.maximum_size.filter(|&s| s != 0).ok_or(STATUS_INVALID_PARAMETER_4)?;
            if size > MAX_SECTION_SIZE {
                return Err(STATUS_SECTION_TOO_BIG);
            }
            let pages = (page_up(size).expect("bounded") / PAGE_SIZE) as usize;
            let reserve = a & SEC_RESERVE != 0;
            let committed = reserve.then(|| vec![false; pages]);
            (size, SectionKind::Pagefile { committed }, SectionSource::Pagefile { size, committed: !reserve })
        }
        Some(h) => {
            let file = sys.objects.reference_by_handle(caller, h, 0, Some(ObjectType::File))?;
            let key = sys.objects.cookie(file).expect("live");
            let made = if sys.io.file(key).is_some() {
                io_file_section(sys, key, image, req.maximum_size, protect)
            } else {
                file_section(b, key, image, req.maximum_size).map(|(size, img)| (size, img, None))
            };
            match made {
                Ok((size, Some(img), _)) => {
                    let source = SectionSource::Image(img.image.clone());
                    (size, SectionKind::Image { image: alloc::boxed::Box::new(img), file: Some(file) }, source)
                }
                Ok((size, None, Some(open))) => (size, SectionKind::File { file }, SectionSource::Vfs { file: open }),
                Ok((size, None, None)) => (size, SectionKind::File { file }, SectionSource::File { file: key }),
                Err(e) => {
                    let _ = sys.objects.dereference(file);
                    sys.reap(b);
                    return Err(e);
                }
            }
        }
    };
    add_section(
        sys,
        b,
        caller,
        req.attrs,
        req.desired,
        Section { size, protect, attributes: a, backing: 0, kind },
        &source,
    )
}

/// Size (and image) of a section of file `key`.
fn file_section(
    b: &mut dyn Backend,
    key: u64,
    image: bool,
    maximum: Option<u64>,
) -> Result<(u64, Option<ImageSection>), NtStatus> {
    if image {
        let bytes = b.read_file(key)?;
        let img = ImageSection::from_file(&bytes)?;
        return Ok((img.size, Some(img)));
    }
    let len = b.file_size(key).ok_or(STATUS_INVALID_FILE_FOR_SECTION)?;
    if len == 0 {
        return Err(STATUS_MAPPED_FILE_SIZE_ZERO);
    }
    match maximum.filter(|&m| m != 0) {
        Some(m) if m > len => Err(STATUS_SECTION_TOO_BIG),
        Some(m) => Ok((m, None)),
        None => Ok((len, None)),
    }
}

/// Size, image or VFS file of a section of a file object of NT file I/O
/// (ADR 0067, 0079). The file object must have been opened to read its
/// data, and to write it for a section whose protection writes shared
/// pages (Learn: `CreateFileMappingW`'s table of `flProtect` against the
/// file's access; `STATUS_ACCESS_DENIED`). A data section larger than a
/// file opened for writing grows the file to its size, as Learn says
/// ("the file on disk is increased to match"); otherwise a larger size is
/// `STATUS_SECTION_TOO_BIG`. A data section's pages are the file's own
/// (cached) pages: a shared view's writes are the file's contents.
fn io_file_section(
    sys: &mut NtSystem,
    key: u64,
    image: bool,
    maximum: Option<u64>,
    protect: Protection,
) -> Result<SectionFile, NtStatus> {
    use crate::io::consts::{FILE_EXECUTE, FILE_READ_DATA, FILE_WRITE_DATA};
    let access = sys.io.file(key).ok_or(STATUS_INVALID_HANDLE)?.access;
    if image {
        if access & (FILE_READ_DATA | FILE_EXECUTE) == 0 {
            return Err(STATUS_ACCESS_DENIED);
        }
        let img = ImageSection::from_file(&sys.io.read_all(key)?)?;
        return Ok((img.size, Some(img), None));
    }
    let writes = protect.allows_shared_write();
    if access & FILE_READ_DATA == 0 || (writes && access & FILE_WRITE_DATA == 0) {
        return Err(STATUS_ACCESS_DENIED);
    }
    let open = sys.io.open_file(key)?;
    let len = open.stat().map_err(crate::io::status_of)?.size;
    let size = match maximum.filter(|&m| m != 0) {
        Some(m) if m > len => {
            if !writes || m > MAX_SECTION_SIZE {
                return Err(STATUS_SECTION_TOO_BIG);
            }
            open.truncate(m).map_err(crate::io::status_of)?;
            m
        }
        Some(m) => m,
        None if len == 0 => return Err(STATUS_MAPPED_FILE_SIZE_ZERO),
        None => len,
    };
    Ok((size, None, Some(open)))
}

/// A file section's size, its image if it is one, and the VFS file of a
/// data section of NT file I/O.
type SectionFile = (u64, Option<ImageSection>, Option<alloc::sync::Arc<poly_vfs::OpenFile>>);

/// Make the pages, store the section and create its object (named or
/// not). On failure, or when `OBJ_OPENIF` opened an existing section,
/// the new section is dropped again.
pub fn add_section(
    sys: &mut NtSystem,
    b: &mut dyn Backend,
    caller: &Caller,
    attrs: &ObjectAttributes<'_>,
    desired: AccessMask,
    mut section: Section,
    source: &SectionSource,
) -> Result<Created, NtStatus> {
    let backing = match b.create_section(source) {
        Ok(id) => id,
        Err(e) => {
            // Nothing stored yet: only the file reference needs dropping.
            let file = match section.kind {
                SectionKind::File { file } => Some(file),
                SectionKind::Image { file, .. } => file,
                SectionKind::Pagefile { .. } => None,
            };
            if let Some(f) = file {
                let _ = sys.objects.dereference(f);
                sys.reap(b);
            }
            return Err(e);
        }
    };
    section.backing = backing;
    let key = sys.add_section(section);
    let new = NewObject::Opaque { kind: ObjectType::Section, cookie: key, signaled: None, parse: false };
    let created = sys.objects.create_object(caller, attrs, desired, new, &mut NoParse);
    match created {
        Ok(c) if c.status == STATUS_OBJECT_NAME_EXISTS => {
            sys.remove_section(key, b);
            sys.reap(b);
            Ok(c)
        }
        Ok(c) => Ok(c),
        Err(e) => {
            sys.remove_section(key, b);
            sys.reap(b);
            Err(e)
        }
    }
}

/// `NtOpenSection`.
pub fn open_section(
    sys: &mut NtSystem,
    caller: &Caller,
    attrs: &ObjectAttributes<'_>,
    desired: AccessMask,
) -> Result<Handle, NtStatus> {
    sys.objects.open_object(caller, attrs, desired, Some(ObjectType::Section), &mut NoParse)
}

/// What `NtMapViewOfSection` was asked for.
#[derive(Clone, Copy, Debug)]
pub struct MapView {
    pub section: Handle,
    /// `*BaseAddress`: 0 for anywhere.
    pub base: u64,
    pub zero_bits: u64,
    pub commit_size: u64,
    /// `*SectionOffset`, if the pointer was not null.
    pub offset: Option<u64>,
    /// `*ViewSize`: 0 for "to the end of the section".
    pub view_size: u64,
    pub inherit: u32,
    pub allocation_type: u32,
    pub protect: u32,
}

/// A mapped view: what `NtMapViewOfSection` writes back, and its status
/// (`STATUS_SUCCESS`, or `STATUS_IMAGE_NOT_AT_BASE` for an image mapped
/// away from its preferred base without being relocated).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapped {
    pub base: u64,
    pub size: u64,
    pub offset: u64,
    pub status: NtStatus,
}

/// The access a view protection needs on the section handle (Learn,
/// "ZwMapViewOfSection" and "MapViewOfFile": `SECTION_MAP_READ` to read,
/// `SECTION_MAP_WRITE` to write shared, `SECTION_MAP_EXECUTE` to execute;
/// a copy-on-write view only reads the section).
fn map_access(p: Protection) -> AccessMask {
    let mut a = 0;
    if p.allows_read() || p.is_copy_on_write() {
        a |= SECTION_MAP_READ;
    }
    if p.allows_shared_write() {
        a |= SECTION_MAP_WRITE;
    }
    if p.allows_execute() {
        a |= SECTION_MAP_EXECUTE;
    }
    a
}

/// `NtMapViewOfSection` into the calling process.
///
/// - `InheritDisposition` is `ViewShare` or `ViewUnmap`
///   (`STATUS_INVALID_PARAMETER_8`); `AllocationType` may only hold
///   `MEM_TOP_DOWN` and `MEM_RESERVE` (`STATUS_INVALID_PARAMETER_9`); both to
///   be confirmed by the spec agent. Inheritance has no effect until
///   processes can be created by others.
/// - Data sections: the view's protection must suit the section's
///   ([`view_allowed`], `STATUS_SECTION_PROTECTION`) and the handle's
///   access; the offset is rounded down to 64 KiB; a view past the end of
///   the section is `STATUS_INVALID_VIEW_SIZE`. A `SEC_RESERVE` section's
///   pages are reserved in the view unless some view committed them;
///   `CommitSize` bytes from the view's start are committed now.
/// - Images: the offset must be 0 (`STATUS_INVALID_PARAMETER_6`; to be
///   confirmed), the view is the whole image with its page protections
///   (`Win32Protect` only has to be valid). Without a base it goes to the
///   image's preferred base, or, if that is taken and the image can move,
///   anywhere with `STATUS_IMAGE_NOT_AT_BASE`: the caller (our `ntdll`)
///   applies the relocations, as the status's description in `ntstatus.h`
///   says. With `relocate`, the kernel does that itself (an executable
///   started without `ntdll`).
pub fn map_view(
    sys: &mut NtSystem,
    proc: &mut NtProcess,
    b: &mut dyn Backend,
    caller: &Caller,
    req: &MapView,
    relocate: bool,
) -> Result<Mapped, NtStatus> {
    if req.inherit != VIEW_SHARE && req.inherit != VIEW_UNMAP {
        return Err(STATUS_INVALID_PARAMETER_8);
    }
    if req.allocation_type & !(MEM_TOP_DOWN | MEM_RESERVE) != 0 {
        return Err(STATUS_INVALID_PARAMETER_9);
    }
    let protect = Protection::new(req.protect, false)?;
    let object = sys.objects.object_by_handle(caller, req.section, map_access(protect), Some(ObjectType::Section))?;
    map_section(sys, proc, b, object, req, relocate)
}

/// [`map_view`] once the section handle is resolved (and its access
/// checked): map section object `object`. `req.section` is not used.
pub fn map_section(
    sys: &mut NtSystem,
    proc: &mut NtProcess,
    b: &mut dyn Backend,
    object: ObjectId,
    req: &MapView,
    relocate: bool,
) -> Result<Mapped, NtStatus> {
    let protect = Protection::new(req.protect, false)?;
    let key = sys.objects.cookie(object).ok_or(STATUS_INVALID_HANDLE)?;
    let section = sys.section(key).ok_or(STATUS_INVALID_HANDLE)?.clone();
    let top_down = req.allocation_type & MEM_TOP_DOWN != 0;

    let (view, source, status) = if let Some(img) = section.image() {
        if req.offset.is_some_and(|o| o != 0) {
            return Err(STATUS_INVALID_PARAMETER_6);
        }
        let pages: alloc::vec::Vec<PageState> = img.pages.iter().map(|&p| PageState::Committed(p)).collect();
        let info = ViewInfo {
            section: key,
            object,
            backing: section.backing,
            image: true,
            offset: 0,
            shared: false,
            max: None,
            reserve: false,
        };
        let mut view = NewView {
            base: req.base,
            size: img.size,
            zero_bits: req.zero_bits,
            top_down,
            protect: PAGE_EXECUTE_WRITECOPY,
            info,
            pages,
        };
        let mut status = STATUS_SUCCESS;
        if req.base == 0 {
            let preferred = img.image_base();
            let free = preferred % ALLOCATION_GRANULARITY == 0 && proc.vm.place(preferred, img.size, false, 0).is_ok();
            if free {
                view.base = preferred;
            } else if !img.relocatable() {
                return Err(STATUS_CONFLICTING_ADDRESSES);
            } else {
                status = STATUS_IMAGE_NOT_AT_BASE;
            }
        }
        (view, ViewSource::Section(section.backing), status)
    } else {
        if !view_allowed(section.protect, protect) {
            return Err(STATUS_SECTION_PROTECTION);
        }
        let offset = req.offset.unwrap_or(0);
        let start = offset & !(ALLOCATION_GRANULARITY - 1);
        let total = page_up(section.size).expect("bounded");
        if start >= total {
            return Err(STATUS_INVALID_VIEW_SIZE);
        }
        let size = if req.view_size == 0 {
            total - start
        } else {
            req.view_size.checked_add(offset - start).and_then(page_up).ok_or(STATUS_INVALID_VIEW_SIZE)?
        };
        if start + size > total {
            return Err(STATUS_INVALID_VIEW_SIZE);
        }
        let first = (start / PAGE_SIZE) as usize;
        let n = (size / PAGE_SIZE) as usize;
        let commit_pages = (page_up(req.commit_size).unwrap_or(size).min(size) / PAGE_SIZE) as usize;
        let pages =
            match &section.kind {
                SectionKind::Pagefile { committed: Some(c) } => (0..n)
                    .map(|i| {
                        if c[first + i] || i < commit_pages {
                            PageState::Committed(protect)
                        } else {
                            PageState::Reserved
                        }
                    })
                    .collect(),
                _ => vec![PageState::Committed(protect); n],
            };
        let info = ViewInfo {
            section: key,
            object,
            backing: section.backing,
            image: false,
            offset: start,
            shared: !protect.is_copy_on_write(),
            max: Some(section.protect),
            reserve: section.is_reserve(),
        };
        let view =
            NewView { base: req.base, size, zero_bits: req.zero_bits, top_down, protect: req.protect, info, pages };
        if section.is_reserve() && commit_pages > 0 {
            b.commit_section(section.backing, start, commit_pages as u64 * PAGE_SIZE);
            if let Some(SectionKind::Pagefile { committed: Some(c) }) = sys.section_mut(key).map(|s| &mut s.kind) {
                c[first..first + commit_pages.min(n)].fill(true);
            }
        }
        (view, ViewSource::Section(section.backing), STATUS_SUCCESS)
    };

    let offset = view.info.offset;
    let base = proc.vm.insert_view(view)?;
    let mut source = source;
    let mut status = status;
    if status == STATUS_IMAGE_NOT_AT_BASE && relocate {
        let img = section.image().expect("an image");
        match img.relocated(base) {
            Ok(bytes) => {
                source = ViewSource::Bytes(bytes);
                status = STATUS_SUCCESS;
            }
            Err(e) => {
                proc.vm.remove(base);
                return Err(e);
            }
        }
    }
    let a = proc.vm.allocation(base).expect("just inserted").clone();
    if let Err(e) = map_view_pages(b, &a, &source) {
        proc.vm.remove(base);
        return Err(e);
    }
    sys.objects.reference(object).expect("live");
    Ok(Mapped { base, size: a.size, offset, status })
}

/// `NtUnmapViewOfSection` in the calling process: `addr` is anywhere in
/// the view. Its pages go, then its reference on the section.
pub fn unmap_view(sys: &mut NtSystem, proc: &mut NtProcess, b: &mut dyn Backend, addr: u64) -> Result<(), NtStatus> {
    let a = proc.vm.remove_view(addr)?;
    b.release(a.base, a.size);
    if let AllocKind::View(v) = a.kind {
        let _ = sys.objects.dereference(v.object);
    }
    sys.reap(b);
    Ok(())
}

/// After `NtAllocateVirtualMemory` committed `[addr, addr + len)` in a
/// view of a `SEC_RESERVE` section: record the section's pages as
/// committed, for views made later.
pub fn record_section_commit(sys: &mut NtSystem, proc: &NtProcess, addr: u64, len: u64) {
    let Some(a) = proc.vm.allocation(addr) else { return };
    let Some(v) = a.view() else { return };
    if !v.reserve {
        return;
    }
    let first = ((v.offset + (addr - a.base)) / PAGE_SIZE) as usize;
    let n = (len / PAGE_SIZE) as usize;
    if let Some(SectionKind::Pagefile { committed: Some(c) }) = sys.section_mut(v.section).map(|s| &mut s.kind) {
        let end = (first + n).min(c.len());
        c[first.min(end)..end].fill(true);
    }
}
