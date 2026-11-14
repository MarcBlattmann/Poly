//! The object services: arguments copied in, `poly-ntobj` asked, results
//! copied out. Behaviour of the object manager itself is ADR 0030's; what
//! is added here is the user-memory side: the structures of Learn's
//! "NtQueryObject", "NtQueryDirectoryObject" and
//! "ZwQuerySymbolicLinkObject" pages and the order of the copies.
//!
//! Output pointers are written after the work is done; a failing write is
//! `STATUS_ACCESS_VIOLATION`, and a handle that could not be returned is
//! closed again. Windows probes output buffers first, so it fails before
//! doing anything; the order is to be confirmed by the spec agent.

use crate::abi::{self, Attributes, UnicodeString};
use crate::consts::*;
use crate::kernel::NtKernel;
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::status::*;
use poly_ntobj::{EventType, Handle, NtStatus, ObjectType, ProcessId};

pub type Result<T> = core::result::Result<T, NtStatus>;

/// Write a new handle to `ptr`; if that fails, close it.
pub fn return_handle<K: NtKernel>(k: &mut K, ptr: u64, h: Handle) -> Result<()> {
    if abi::write_u64(k, ptr, h.0).is_err() {
        let caller = k.caller();
        k.with_state(|sys, _, b| {
            let _ = sys.objects.close(&caller, h);
            sys.reap(b);
        });
        return Err(STATUS_ACCESS_VIOLATION);
    }
    Ok(())
}

/// `(handle*, access, attributes*)`: the three arguments every create and
/// open service starts with.
#[inline(never)]
fn open_args<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<(u64, u32, Attributes)> {
    let [out, access, attrs] = abi::args::<K, 3>(k, regs)?;
    let attrs = abi::read_attributes(k, attrs)?;
    Ok((out, access as u32, attrs))
}

pub fn close<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let h = Handle(abi::arg(k, regs, 0)?);
    let caller = k.caller();
    k.with_state(|sys, _, b| {
        // A completion port whose last handle goes wakes its waiters with
        // `STATUS_ABANDONED_WAIT_0` (ADR 0169; Learn: "GetQueuedCompletionStatus").
        let port = sys
            .objects
            .object_by_handle(&caller, h, 0, Some(poly_ntobj::ObjectType::IoCompletion))
            .ok()
            .and_then(|o| sys.objects.cookie(o).map(|c| (o, c)));
        let r = sys.objects.nt_close(&caller, h);
        if let Some((o, cookie)) = port
            && r.is_ok()
            && sys.objects.handle_count(o) == Some(0)
        {
            sys.io.port_abandon(&mut sys.objects, cookie);
        }
        sys.reap(b);
        r
    })?;
    Ok(STATUS_SUCCESS)
}

pub fn make_temporary<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let h = Handle(abi::arg(k, regs, 0)?);
    let caller = k.caller();
    k.with_state(|sys, _, b| {
        let r = sys.objects.nt_make_temporary_object(&caller, h);
        sys.reap(b);
        r
    })?;
    Ok(STATUS_SUCCESS)
}

/// A process handle as the object services see it: the process it names
/// (`None` for a null handle), checked for `access`. Only NT processes
/// have process objects; their cookie is the process id.
pub fn process_of(
    sys: &mut crate::system::NtSystem,
    caller: &poly_ntobj::Caller,
    h: Handle,
    access: u32,
) -> Result<Option<ProcessId>> {
    if h == Handle::NULL {
        return Ok(None);
    }
    if h == Handle::CURRENT_PROCESS {
        return Ok(Some(caller.process));
    }
    let id = sys.objects.object_by_handle(caller, h, access, Some(poly_ntobj::ObjectType::Process))?;
    Ok(Some(ProcessId(sys.objects.cookie(id).expect("live") as u32)))
}

pub fn duplicate_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [src_process, src, dst_process, out, access, attributes, options] = abi::args::<K, 7>(k, regs)?;
    let caller = k.caller();
    let made = k.with_state(|sys, _, b| {
        let r = (|| {
            let from =
                process_of(sys, &caller, Handle(src_process), PROCESS_DUP_HANDLE)?.ok_or(STATUS_INVALID_HANDLE)?;
            let to = process_of(sys, &caller, Handle(dst_process), PROCESS_DUP_HANDLE)?;
            // A process that has ended cannot be given a handle (nor have one taken).
            if sys.process_exits.contains_key(&from) || to.is_some_and(|t| sys.process_exits.contains_key(&t)) {
                return Err(STATUS_PROCESS_IS_TERMINATING);
            }
            // Without a `TargetHandle` pointer nothing is made: only the source closes.
            let to = if out == 0 { None } else { to };
            sys.objects.nt_duplicate_object(
                &caller,
                from,
                Handle(src),
                to,
                access as u32,
                attributes as u32,
                options as u32,
            )
        })();
        sys.reap(b);
        r
    });
    // A refused or failed duplication leaves NULL in `*TargetHandle` (the sheet: "handle NULL").
    let made = match made {
        Ok(m) => m,
        Err(e) => {
            if out != 0 {
                let _ = return_handle(k, out, Handle::NULL);
            }
            return Err(e);
        }
    };
    if let Some(h) = made
        && out != 0
    {
        return_handle(k, out, h)?;
    }
    Ok(STATUS_SUCCESS)
}

/// Write `info` (or as much as `length` allows: nothing) and the return
/// length. A buffer smaller than `info` is `STATUS_INFO_LENGTH_MISMATCH`
/// with the size needed in `ReturnLength` (Learn, "NtQueryObject"; to be
/// confirmed by the spec agent for every class).
fn return_info<K: NtKernel>(k: &mut K, buf: u64, length: u64, ret: u64, info: &[u8]) -> Result<NtStatus> {
    abi::write_opt_u32(k, ret, info.len() as u32)?;
    if (length as usize) < info.len() {
        return Ok(STATUS_INFO_LENGTH_MISMATCH);
    }
    abi::write(k, buf, info)?;
    Ok(STATUS_SUCCESS)
}

/// What `NtQueryObject(ObjectNameInformation)` answers for a file object.
#[derive(Debug, PartialEq, Eq)]
pub struct NameAnswer {
    pub status: NtStatus,
    /// What `*ReturnLength` gets.
    pub return_length: u32,
    /// What is written at the start of the caller's buffer.
    pub bytes: Vec<u8>,
}

/// The answer of `NtQueryObject(ObjectNameInformation)` for a file on
/// `device` (its object name, e.g. `\Device\NamedPipe`) with the path
/// `path` there, for a caller's buffer of `length` bytes at `buf`
/// (specs/windows/ntdll.dll/NtQueryObject.yaml, observed with pipes):
///
/// * the whole name fits: a `UNICODE_STRING` (its buffer right behind it),
///   the characters and a NUL; the return length is that size;
/// * below the `UNICODE_STRING`'s size: `STATUS_INFO_LENGTH_MISMATCH`,
///   nothing written, the full size returned;
/// * too small for the device's name: `STATUS_BUFFER_OVERFLOW`, nothing
///   written, the full size returned;
/// * the device's name fits but not one character more:
///   `STATUS_OBJECT_PATH_INVALID`, a complete string for the device's name
///   alone, return length 0;
/// * more than that: `STATUS_BUFFER_OVERFLOW` with the buffer filled to
///   its last byte with the start of the name.
///
/// A file with no path on its device (`path` `None`: an anonymous pipe)
/// has only the device's name to give, and that is
/// `STATUS_OBJECT_PATH_INVALID` once it fits.
pub fn file_name_answer(device: &[u16], path: Option<&[u16]>, length: usize, buf: u64) -> NameAnswer {
    let header = UNICODE_STRING_SIZE;
    let string = |chars: usize, text: &[u8]| {
        let s = UnicodeString { length: chars as u16, maximum_length: chars as u16 + 2, buffer: buf + header as u64 };
        let mut out = s.to_bytes().to_vec();
        out.extend_from_slice(text);
        out
    };
    let device_bytes = abi::utf16_bytes(device);
    let device_size = header + device_bytes.len() + 2;
    let mut device_only = device_bytes.clone();
    device_only.extend_from_slice(&[0, 0]);
    let mut full = device_bytes.clone();
    full.extend_from_slice(&abi::utf16_bytes(path.unwrap_or_default()));
    let full_chars = full.len();
    full.extend_from_slice(&[0, 0]);
    let full_size = if path.is_some() { header + full.len() } else { device_size };
    let nothing = |status| NameAnswer { status, return_length: full_size as u32, bytes: Vec::new() };
    if length < header {
        return nothing(STATUS_INFO_LENGTH_MISMATCH);
    }
    if length < device_size {
        return nothing(STATUS_BUFFER_OVERFLOW);
    }
    if path.is_some() && length >= full_size {
        return NameAnswer {
            status: STATUS_SUCCESS,
            return_length: full_size as u32,
            bytes: string(full_chars, &full),
        };
    }
    if path.is_none() || length < device_size + 2 {
        return NameAnswer {
            status: STATUS_OBJECT_PATH_INVALID,
            return_length: 0,
            bytes: string(device_bytes.len(), &device_only),
        };
    }
    NameAnswer {
        status: STATUS_BUFFER_OVERFLOW,
        return_length: full_size as u32,
        bytes: string(length - header - 2, &full[..length - header]),
    }
}

/// A `UNICODE_STRING` followed by `pad` bytes and then its characters and
/// a NUL, all at `buf`: what `ObjectNameInformation` and
/// `ObjectTypeInformation` return.
fn string_info(buf: u64, header: usize, text: &[u16]) -> Vec<u8> {
    let mut out = alloc::vec![0u8; header];
    let bytes = abi::utf16_bytes(text);
    let s = if text.is_empty() && header == UNICODE_STRING_SIZE {
        // An unnamed object: an empty name with no buffer.
        UnicodeString { length: 0, maximum_length: 0, buffer: 0 }
    } else {
        UnicodeString {
            length: bytes.len() as u16,
            maximum_length: bytes.len() as u16 + 2,
            buffer: buf + header as u64,
        }
    };
    out[..UNICODE_STRING_SIZE].copy_from_slice(&s.to_bytes());
    if s.buffer != 0 {
        out.extend_from_slice(&bytes);
        out.extend_from_slice(&[0, 0]);
    }
    out
}

/// `NtQueryObject`: `ObjectBasicInformation`
/// (`PUBLIC_OBJECT_BASIC_INFORMATION`), `ObjectNameInformation`
/// (`OBJECT_NAME_INFORMATION`: the full path, empty for an unnamed object)
/// and `ObjectTypeInformation` (`PUBLIC_OBJECT_TYPE_INFORMATION`: the type
/// name). Other classes are `STATUS_INVALID_INFO_CLASS` (to be confirmed).
pub fn query_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    let (h, class) = (Handle(h), class as u32);
    let caller = k.caller();
    if class == OBJECT_NAME_INFORMATION {
        // A file object's name is its device's name and its path there.
        let parts = k.with_state(|sys, _, _| {
            let id = sys.objects.object_by_handle(&caller, h, 0, Some(ObjectType::File)).ok()?;
            sys.io.object_name_parts(sys.objects.cookie(id)?)
        });
        if let Some((device, path)) = parts {
            let a = file_name_answer(&device, path.as_deref(), length as usize, buf);
            abi::write_opt_u32(k, ret, a.return_length)?;
            abi::write(k, buf, &a.bytes)?;
            return Ok(a.status);
        }
    }
    let info: Vec<u8> = k.with_state(|sys, _, _| -> Result<Vec<u8>> {
        let om = &sys.objects;
        Ok(match class {
            OBJECT_BASIC_INFORMATION => {
                let i = om.nt_query_object_basic(&caller, h)?;
                let mut b = alloc::vec![0u8; OBJECT_BASIC_INFORMATION_SIZE];
                for (n, v) in [i.attributes, i.granted_access, i.handle_count, i.pointer_count].iter().enumerate() {
                    b[n * 4..n * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
                b
            }
            OBJECT_NAME_INFORMATION => string_info(buf, UNICODE_STRING_SIZE, &om.nt_query_object_name(&caller, h)?),
            OBJECT_TYPE_INFORMATION => {
                let name: Vec<u16> = om.nt_query_object_type(&caller, h)?.encode_utf16().collect();
                string_info(buf, OBJECT_TYPE_INFORMATION_SIZE, &name)
            }
            OBJECT_HANDLE_FLAG_INFORMATION => {
                let f = om.handle_flags(&caller, h)?;
                alloc::vec![u8::from(f.inherit), u8::from(f.protect_from_close)]
            }
            _ => return Err(STATUS_INVALID_INFO_CLASS),
        })
    })?;
    return_info(k, buf, length, ret, &info)
}

/// `NtSetInformationObject`: only Poly's `ObjectHandleFlagInformation`
/// ([`OBJECT_HANDLE_FLAG_INFORMATION`], ADR 0089), the handle's inherit
/// and protect-from-close flags (`SetHandleInformation`). A length other
/// than the structure's is `STATUS_INFO_LENGTH_MISMATCH`, a pseudo handle
/// `STATUS_INVALID_HANDLE`, another class `STATUS_INVALID_INFO_CLASS` (to
/// be confirmed by the spec agent).
pub fn set_information_object<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, length] = abi::args::<K, 4>(k, regs)?;
    if class as u32 != OBJECT_HANDLE_FLAG_INFORMATION {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    if length as usize != OBJECT_HANDLE_FLAG_INFORMATION_SIZE {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    let mut info = [0u8; OBJECT_HANDLE_FLAG_INFORMATION_SIZE];
    abi::read(k, buf, &mut info)?;
    let flags = poly_ntobj::HandleFlags { inherit: info[0] != 0, protect_from_close: info[1] != 0 };
    let caller = k.caller();
    k.with_state(|sys, _, _| sys.objects.set_handle_flags(&caller, Handle(h), flags))?;
    Ok(STATUS_SUCCESS)
}

pub fn create_directory<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let (out, access, attrs) = open_args(k, regs)?;
    let caller = k.caller();
    let c = k.with_state(|sys, _, _| {
        sys.objects.nt_create_directory_object(&caller, &attrs.as_object_attributes(), access)
    })?;
    return_handle(k, out, c.handle)?;
    Ok(c.status)
}

pub fn open_directory<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let (out, access, attrs) = open_args(k, regs)?;
    let caller = k.caller();
    let h =
        k.with_state(|sys, _, _| sys.objects.nt_open_directory_object(&caller, &attrs.as_object_attributes(), access))?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

/// `NtQueryDirectoryObject` (Learn: "NtQueryDirectoryObject"). The buffer
/// gets an array of `OBJECT_DIRECTORY_INFORMATION` (name and type name)
/// ended by an all-zero entry, then the strings, each with a NUL. `Context`
/// is the index of the next entry (entries in the object manager's order,
/// ADR 0030); `RestartScan` starts at 0; `ReturnSingleEntry` returns at most
/// one. No entry left: `STATUS_NO_MORE_ENTRIES`; not even one fits:
/// `STATUS_BUFFER_TOO_SMALL` with the size needed in `ReturnLength`; some
/// but not all fit: `STATUS_MORE_ENTRIES`. The layout of the strings and
/// these statuses are to be confirmed by the spec agent.
pub fn query_directory<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, buf, length, single, restart, context, ret] = abi::args::<K, 7>(k, regs)?;
    let start = if restart as u8 != 0 { 0 } else { abi::read_u32(k, context)? as usize };
    let caller = k.caller();
    let entries = k.with_state(|sys, _, _| sys.objects.nt_query_directory_object(&caller, Handle(h)))?;
    if start >= entries.len() {
        abi::write_opt_u32(k, ret, 0)?;
        return Ok(STATUS_NO_MORE_ENTRIES);
    }
    let rest = &entries[start..];
    let max = if single as u8 != 0 { 1 } else { rest.len() };
    let size_of = |n: usize| -> usize {
        let strings: usize = rest[..n].iter().map(|(name, ty)| name.len() * 2 + 2 + ty.len() * 2 + 2).sum();
        (n + 1) * OBJECT_DIRECTORY_INFORMATION_SIZE + strings
    };
    let mut n = 0;
    while n < max && size_of(n + 1) <= length as usize {
        n += 1;
    }
    if n == 0 {
        abi::write_opt_u32(k, ret, size_of(1) as u32)?;
        return Ok(STATUS_BUFFER_TOO_SMALL);
    }
    let mut out = alloc::vec![0u8; (n + 1) * OBJECT_DIRECTORY_INFORMATION_SIZE];
    for (i, (name, ty)) in rest[..n].iter().enumerate() {
        let ty: Vec<u16> = ty.encode_utf16().collect();
        for (j, text) in [name.as_slice(), ty.as_slice()].into_iter().enumerate() {
            let at = buf + out.len() as u64;
            let s = UnicodeString {
                length: (text.len() * 2) as u16,
                maximum_length: (text.len() * 2 + 2) as u16,
                buffer: at,
            };
            let slot = i * OBJECT_DIRECTORY_INFORMATION_SIZE + j * UNICODE_STRING_SIZE;
            out[slot..slot + UNICODE_STRING_SIZE].copy_from_slice(&s.to_bytes());
            out.extend_from_slice(&abi::utf16_bytes(text));
            out.extend_from_slice(&[0, 0]);
        }
    }
    abi::write(k, buf, &out)?;
    abi::write_u32(k, context, (start + n) as u32)?;
    abi::write_opt_u32(k, ret, out.len() as u32)?;
    Ok(if start + n < entries.len() && single as u8 == 0 { STATUS_MORE_ENTRIES } else { STATUS_SUCCESS })
}

pub fn create_symbolic_link<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let (out, access, attrs) = open_args(k, regs)?;
    let target_ptr = abi::arg(k, regs, 3)?;
    let target = abi::read_unicode(k, target_ptr)?;
    let caller = k.caller();
    let c = k.with_state(|sys, _, _| {
        sys.objects.nt_create_symbolic_link_object(&caller, &attrs.as_object_attributes(), access, &target)
    })?;
    return_handle(k, out, c.handle)?;
    Ok(c.status)
}

pub fn open_symbolic_link<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let (out, access, attrs) = open_args(k, regs)?;
    let caller = k.caller();
    let h = k.with_state(|sys, _, _| {
        sys.objects.nt_open_symbolic_link_object(&caller, &attrs.as_object_attributes(), access)
    })?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

/// `NtQuerySymbolicLinkObject` (Learn: "ZwQuerySymbolicLinkObject"): the
/// caller's `UNICODE_STRING` gives the buffer and its `MaximumLength`. The
/// target is copied with a NUL if there is room for one; `Length` excludes
/// it and `ReturnedLength` counts the bytes written. A buffer too small for
/// the target is `STATUS_BUFFER_TOO_SMALL` with the size needed in
/// `ReturnedLength`. NUL and lengths to be confirmed by the spec agent.
pub fn query_symbolic_link<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, target, ret] = abi::args::<K, 3>(k, regs)?;
    let caller = k.caller();
    let text = k.with_state(|sys, _, _| sys.objects.nt_query_symbolic_link_object(&caller, Handle(h)))?;
    let s = abi::read_unicode_header(k, target)?;
    let bytes = abi::utf16_bytes(&text);
    if (s.maximum_length as usize) < bytes.len() {
        abi::write_opt_u32(k, ret, bytes.len() as u32)?;
        return Ok(STATUS_BUFFER_TOO_SMALL);
    }
    let mut data = bytes.clone();
    if s.maximum_length as usize >= bytes.len() + 2 {
        data.extend_from_slice(&[0, 0]);
    }
    abi::write(k, s.buffer, &data)?;
    abi::write(k, target, &(bytes.len() as u16).to_le_bytes())?;
    abi::write_opt_u32(k, ret, data.len() as u32)?;
    Ok(STATUS_SUCCESS)
}

/// `NtCreateEvent`: `EventType` is `NotificationEvent` or
/// `SynchronizationEvent` (`wdm.h`), else `STATUS_INVALID_PARAMETER` (to
/// be confirmed by the spec agent).
pub fn create_event<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let (out, access, attrs) = open_args(k, regs)?;
    let [_, _, _, kind, initial] = abi::args::<K, 5>(k, regs)?;
    let kind = match kind as u32 {
        NOTIFICATION_EVENT => EventType::Notification,
        SYNCHRONIZATION_EVENT => EventType::Synchronization,
        _ => return Err(STATUS_INVALID_PARAMETER),
    };
    let caller = k.caller();
    let c = k.with_state(|sys, _, _| {
        sys.objects.nt_create_event(&caller, &attrs.as_object_attributes(), access, kind, initial as u8 != 0)
    })?;
    return_handle(k, out, c.handle)?;
    Ok(c.status)
}

pub fn open_event<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let (out, access, attrs) = open_args(k, regs)?;
    let caller = k.caller();
    let h = k.with_state(|sys, _, _| sys.objects.nt_open_event(&caller, &attrs.as_object_attributes(), access))?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

/// `NtSetEvent` / `NtResetEvent`: the previous state (0 or 1) goes to the
/// optional `PreviousState`.
pub fn set_event<K: NtKernel>(k: &mut K, regs: &SyscallRegs, set: bool) -> Result<NtStatus> {
    let [h, prev] = abi::args::<K, 2>(k, regs)?;
    let caller = k.caller();
    let was = k.with_state(|sys, _, _| {
        if set { sys.objects.nt_set_event(&caller, Handle(h)) } else { sys.objects.nt_reset_event(&caller, Handle(h)) }
    })?;
    abi::write_opt_u32(k, prev, was as u32)?;
    Ok(STATUS_SUCCESS)
}

#[cfg(test)]
mod name_tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// `(status, return length, Length, MaximumLength, bytes written, name)`.
    fn ask(path: Option<&str>, length: usize) -> (u32, u32, u16, u16, usize, alloc::string::String) {
        let a = file_name_answer(&w("\\Device\\NamedPipe"), path.map(w).as_deref(), length, 0x1000);
        if a.bytes.is_empty() {
            return (a.status.0, a.return_length, 0, 0, 0, alloc::string::String::new());
        }
        let s = UnicodeString::from_bytes(a.bytes[..16].try_into().unwrap());
        assert_eq!(s.buffer, 0x1010);
        let chars: Vec<u16> = a.bytes[16..16 + usize::from(s.length) / 2 * 2]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        (
            a.status.0,
            a.return_length,
            s.length,
            s.maximum_length,
            a.bytes.len(),
            alloc::string::String::from_utf16_lossy(&chars),
        )
    }

    /// The lines of specs/windows/ntdll.dll/NtQueryObject.tests/windows.json for
    /// the pipe `PolySpec4_ObjA`.
    #[test]
    fn a_named_pipe_by_buffer_size() {
        let p = Some("\\PolySpec4_ObjA");
        let full = "\\Device\\NamedPipe\\PolySpec4_ObjA";
        for n in [82, 83, 84, 88, 528] {
            assert_eq!(ask(p, n), (0, 82, 64, 66, 82, full.into()), "{n}");
        }
        for n in [0, 1, 2, 4, 8, 12, 15] {
            assert_eq!(ask(p, n), (0xC000_0004, 82, 0, 0, 0, "".into()), "{n}");
        }
        for n in [16, 17, 18, 19, 20, 24, 32, 34, 48, 50, 51] {
            assert_eq!(ask(p, n), (0x8000_0005, 82, 0, 0, 0, "".into()), "{n}");
        }
        for n in [52, 53] {
            assert_eq!(ask(p, n), (0xC000_0039, 0, 34, 36, 52, "\\Device\\NamedPipe".into()), "{n}");
        }
        assert_eq!(ask(p, 54), (0x8000_0005, 82, 36, 38, 54, "\\Device\\NamedPipe\\".into()));
        assert_eq!(ask(p, 60), (0x8000_0005, 82, 42, 44, 60, "\\Device\\NamedPipe\\Pol".into()));
        assert_eq!(ask(p, 79), (0x8000_0005, 82, 61, 63, 79, "\\Device\\NamedPipe\\PolySpec4_Ob".into()));
        assert_eq!(ask(p, 81), (0x8000_0005, 82, 63, 65, 81, "\\Device\\NamedPipe\\PolySpec4_Obj".into()));
        // The NUL after the name is written when it fits.
        let a = file_name_answer(&w("\\Device\\NamedPipe"), Some(&w("\\P")), 528, 0);
        assert_eq!(a.bytes.len(), 16 + 38 + 2);
        assert_eq!(&a.bytes[a.bytes.len() - 2..], [0, 0]);
    }

    #[test]
    fn an_anonymous_pipe_has_only_the_devices_name() {
        assert_eq!(ask(None, 528), (0xC000_0039, 0, 34, 36, 52, "\\Device\\NamedPipe".into()));
        assert_eq!(ask(None, 52), (0xC000_0039, 0, 34, 36, 52, "\\Device\\NamedPipe".into()));
        assert_eq!(ask(None, 16), (0x8000_0005, 52, 0, 0, 0, "".into()));
        assert_eq!(ask(None, 8), (0xC000_0004, 52, 0, 0, 0, "".into()));
    }
}
