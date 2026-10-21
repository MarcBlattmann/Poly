//! The registry services: arguments copied in, the registry asked through
//! the object manager, structures copied out. Parameters are those of the
//! documented `Zw*` routines (Learn: "ZwCreateKey", "ZwOpenKey",
//! "ZwOpenKeyEx", "ZwDeleteKey", "ZwDeleteValueKey", "ZwEnumerateKey",
//! "ZwEnumerateValueKey", "ZwQueryKey", "ZwQueryValueKey", "ZwSetValueKey",
//! "ZwFlushKey", "ZwNotifyChangeKey"); `NtSetInformationKey` takes
//! `(KeyHandle, KeySetInformationClass, KeySetInformation, Length)`, the
//! order of the `KEY_SET_INFORMATION_CLASS` routine the WDK describes.
//!
//! Access each service needs of the key handle (Learn): query and
//! enumerate values `KEY_QUERY_VALUE`, enumerate subkeys
//! `KEY_ENUMERATE_SUB_KEYS`, set and delete values `KEY_SET_VALUE`, delete
//! the key `DELETE`, notify `KEY_NOTIFY`, `NtQueryKey` `KEY_QUERY_VALUE`
//! except for `KeyNameInformation`. `NtFlushKey` needs none and
//! `NtSetInformationKey` `KEY_SET_VALUE` (both to be confirmed by the spec
//! agent).

use super::{CreateArgs, KeyParse, key_access, key_of_handle};
use crate::abi::{self, Attributes};
use crate::kernel::NtKernel;
use crate::objects::return_handle;
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::status::*;
use poly_ntobj::{Handle, NewObject, NtStatus, OBJ_OPENLINK, ObjectType};
use poly_registry::consts::*;
use poly_registry::info;

pub type Result<T> = core::result::Result<T, NtStatus>;

/// The largest value data Poly stores (its own limit: Learn's is
/// "available memory"); more is `STATUS_INSUFFICIENT_RESOURCES`.
pub const MAX_VALUE_DATA: usize = 16 << 20;

/// `NtCreateKey(KeyHandle*, DesiredAccess, ObjectAttributes*, TitleIndex,
/// Class*, CreateOptions, Disposition*)`. Unknown `CreateOptions` bits are
/// `STATUS_INVALID_PARAMETER` (to be confirmed); `REG_OPTION_OPEN_LINK`
/// opens a final link itself, like `OBJ_OPENLINK`; `REG_OPTION_BACKUP_RESTORE`
/// changes nothing (no security). A name that does not lead into
/// `\REGISTRY` is `STATUS_OBJECT_TYPE_MISMATCH` (to be confirmed).
pub fn create_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, access, oa, _title, class_ptr, options, disposition] = abi::args::<K, 7>(k, regs)?;
    let attrs = abi::read_attributes(k, oa)?;
    let class = if class_ptr == 0 { Vec::new() } else { abi::read_unicode(k, class_ptr)? };
    let options = options as u32;
    if options & !REG_LEGAL_CREATE_OPTIONS != 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let mut attrs = attrs;
    if options & REG_OPTION_OPEN_LINK != 0 {
        attrs.attributes |= OBJ_OPENLINK;
    }
    let now = k.system_time();
    let caller = k.caller();
    let (handle, d) = k.with_state(|sys, _, b| {
        let r = (|| {
            let root = match attrs.root {
                Some(h) => root_key(sys, &caller, h)?,
                None => None,
            };
            let mut hook = KeyParse {
                reg: &mut sys.registry.reg,
                open_link: attrs.attributes & OBJ_OPENLINK != 0,
                create: Some(CreateArgs { options, class: &class, root }),
                now,
                disposition: None,
            };
            let new =
                NewObject::Opaque { kind: ObjectType::Key, cookie: super::NO_INSTANCE, signaled: None, parse: true };
            let c = sys.objects.create_object(
                &caller,
                &attrs.as_object_attributes(),
                key_access(access),
                new,
                &mut hook,
            )?;
            let Some(d) = hook.disposition else {
                // The walk never reached the registry: undo the object.
                let _ = sys.objects.close(&caller, c.handle);
                return Err(STATUS_OBJECT_TYPE_MISMATCH);
            };
            Ok((c.handle, d))
        })();
        sys.registry.deliver(&mut sys.objects, Some(caller.process));
        sys.reap(b);
        r
    })?;
    return_handle(k, out, handle)?;
    abi::write_opt_u32(k, disposition, d.value())?;
    Ok(STATUS_SUCCESS)
}

/// The instance a `RootDirectory` key handle names, and whether that handle
/// may create subkeys; `None` for a root handle that is not a key.
fn root_key(
    sys: &mut crate::system::NtSystem,
    caller: &poly_ntobj::Caller,
    h: Handle,
) -> Result<Option<(poly_registry::OpenId, bool)>> {
    let Ok(id) = sys.objects.object_by_handle(caller, h, 0, Some(ObjectType::Key)) else { return Ok(None) };
    let open = poly_registry::OpenId::from_bits(sys.objects.cookie(id).expect("live"));
    let granted = sys.objects.granted_access(caller, h)?;
    Ok(Some((open, granted & KEY_CREATE_SUB_KEY != 0)))
}

fn open<K: NtKernel>(k: &mut K, out: u64, access: u64, attrs: Attributes, open_options: u32) -> Result<NtStatus> {
    let mut attrs = attrs;
    if open_options & REG_OPTION_OPEN_LINK != 0 {
        attrs.attributes |= OBJ_OPENLINK;
    }
    let now = k.system_time();
    let caller = k.caller();
    let h = k.with_state(|sys, _, b| {
        let mut hook = KeyParse {
            reg: &mut sys.registry.reg,
            open_link: attrs.attributes & OBJ_OPENLINK != 0,
            create: None,
            now,
            disposition: None,
        };
        let r = sys.objects.open_object(
            &caller,
            &attrs.as_object_attributes(),
            key_access(access),
            Some(ObjectType::Key),
            &mut hook,
        );
        sys.reap(b);
        r
    })?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

/// `NtOpenKey(KeyHandle*, DesiredAccess, ObjectAttributes*)`.
pub fn open_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, access, oa] = abi::args::<K, 3>(k, regs)?;
    let attrs = abi::read_attributes(k, oa)?;
    open(k, out, access, attrs, 0)
}

/// `NtOpenKeyEx(KeyHandle*, DesiredAccess, ObjectAttributes*,
/// OpenOptions)`: options other than `REG_OPTION_OPEN_LINK` and
/// `REG_OPTION_BACKUP_RESTORE` are `STATUS_INVALID_PARAMETER_4` (Learn).
pub fn open_key_ex<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, access, oa, options] = abi::args::<K, 4>(k, regs)?;
    let options = options as u32;
    if options & !REG_LEGAL_OPEN_OPTIONS != 0 {
        return Err(STATUS_INVALID_PARAMETER_4);
    }
    let attrs = abi::read_attributes(k, oa)?;
    open(k, out, access, attrs, options)
}

/// Run `f` on the key behind handle `h` (checked for `access`), then
/// complete notifications and reap.
fn with_key<K: NtKernel, R>(
    k: &mut K,
    h: u64,
    access: u32,
    f: impl FnOnce(&mut poly_registry::Registry, poly_registry::KeyId) -> Result<R>,
) -> Result<R> {
    let caller = k.caller();
    k.with_state(|sys, _, b| {
        let r = key_of_handle(sys, &caller, Handle(h), access).and_then(|(_, _, key)| f(&mut sys.registry.reg, key));
        sys.registry.deliver(&mut sys.objects, Some(caller.process));
        sys.reap(b);
        r
    })
}

/// `NtDeleteKey(KeyHandle)`.
pub fn delete_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let h = abi::arg(k, regs, 0)?;
    let now = k.system_time();
    with_key(k, h, DELETE, |reg, key| reg.delete_key(key, now))?;
    Ok(STATUS_SUCCESS)
}

/// A value name argument: a null pointer is the default value (Learn:
/// "ZwSetValueKey": "This parameter can be a NULL pointer if the value
/// entry has no name").
fn value_name<K: NtKernel>(k: &mut K, ptr: u64) -> Result<Vec<u16>> {
    if ptr == 0 { Ok(Vec::new()) } else { abi::read_unicode(k, ptr) }
}

/// `NtDeleteValueKey(KeyHandle, ValueName*)`.
pub fn delete_value_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, name] = abi::args::<K, 2>(k, regs)?;
    let name = value_name(k, name)?;
    let now = k.system_time();
    with_key(k, h, KEY_SET_VALUE, |reg, key| reg.delete_value(key, &name, now))?;
    Ok(STATUS_SUCCESS)
}

/// `NtSetValueKey(KeyHandle, ValueName*, TitleIndex, Type, Data*,
/// DataSize)`.
pub fn set_value_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, name, _title, kind, data, size] = abi::args::<K, 6>(k, regs)?;
    let size = size as u32 as usize;
    if size > MAX_VALUE_DATA {
        return Err(STATUS_INSUFFICIENT_RESOURCES);
    }
    let name = value_name(k, name)?;
    let mut bytes = alloc::vec![0u8; size];
    if size > 0 {
        abi::read(k, data, &mut bytes)?;
    }
    let now = k.system_time();
    with_key(k, h, KEY_SET_VALUE, |reg, key| reg.set_value(key, &name, kind as u32, &bytes, now))?;
    Ok(STATUS_SUCCESS)
}

/// Copy `e` to the caller's buffer by the buffer-size rules of
/// [`info::fit`] and return the status.
fn copy_out<K: NtKernel>(k: &mut K, e: &info::Encoded, buf: u64, length: u64, ret: u64) -> Result<NtStatus> {
    let fit = info::fit(e, length as u32 as usize);
    if !fit.write.is_empty() {
        abi::write(k, buf, fit.write)?;
    }
    abi::write_opt_u32(k, ret, fit.result_length)?;
    Ok(fit.status)
}

/// `NtQueryKey(KeyHandle, KeyInformationClass, KeyInformation, Length,
/// ResultLength*)`.
pub fn query_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    let class = class as u32;
    let e = with_key(k, h, info::query_key_access(class), |reg, key| info::key_information(reg, key, class, false))?;
    copy_out(k, &e, buf, length, ret)
}

/// `NtEnumerateKey(KeyHandle, Index, KeyInformationClass, KeyInformation,
/// Length, ResultLength*)`.
pub fn enumerate_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, index, class, buf, length, ret] = abi::args::<K, 6>(k, regs)?;
    let class = class as u32;
    let e = with_key(k, h, KEY_ENUMERATE_SUB_KEYS, |reg, key| {
        if !matches!(class, KEY_BASIC_INFORMATION | KEY_NODE_INFORMATION | KEY_FULL_INFORMATION) {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let sub = reg.subkey_at(key, index as u32 as usize)?;
        info::key_information(reg, sub, class, true)
    })?;
    copy_out(k, &e, buf, length, ret)
}

fn check_value_buffer(class: u32, buf: u64) -> Result<()> {
    if info::value_class_needs_align64(class) && !buf.is_multiple_of(8) {
        return Err(STATUS_DATATYPE_MISALIGNMENT);
    }
    Ok(())
}

/// `NtQueryValueKey(KeyHandle, ValueName*, KeyValueInformationClass,
/// KeyValueInformation, Length, ResultLength*)`.
pub fn query_value_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, name, class, buf, length, ret] = abi::args::<K, 6>(k, regs)?;
    let class = class as u32;
    check_value_buffer(class, buf)?;
    let name = value_name(k, name)?;
    let e = with_key(k, h, KEY_QUERY_VALUE, |reg, key| info::value_information(reg.value(key, &name)?, class))?;
    copy_out(k, &e, buf, length, ret)
}

/// `NtEnumerateValueKey(KeyHandle, Index, KeyValueInformationClass,
/// KeyValueInformation, Length, ResultLength*)`.
pub fn enumerate_value_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, index, class, buf, length, ret] = abi::args::<K, 6>(k, regs)?;
    let class = class as u32;
    check_value_buffer(class, buf)?;
    let e = with_key(k, h, KEY_QUERY_VALUE, |reg, key| {
        info::value_information(reg.value_at(key, index as u32 as usize)?, class)
    })?;
    copy_out(k, &e, buf, length, ret)
}

/// `NtFlushKey(KeyHandle)`: write the key's hive to its file now (Learn:
/// "ZwFlushKey flushes the entire registry hive for the key"), through
/// [`NtKernel::save_hive`]. A key in no hive, a volatile key and a hive
/// with no changes have nothing to write. A failed write is
/// `STATUS_REGISTRY_IO_FAILED` and leaves the hive to be written later.
pub fn flush_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let h = abi::arg(k, regs, 0)?;
    let save = with_key(k, h, 0, |reg, key| {
        let hive = reg.key(key)?.hive().filter(|hv| reg.hive(*hv).dirty);
        Ok(hive.map(|hv| (hv, reg.hive(hv).file.clone(), reg.save(hv))))
    })?;
    if let Some((hive, file, bytes)) = save
        && k.save_hive(&file, &bytes).is_err()
    {
        k.with_state(|sys, _, _| sys.registry.reg.mark_hive_dirty(hive));
        return Err(STATUS_REGISTRY_IO_FAILED);
    }
    Ok(STATUS_SUCCESS)
}

/// `NtNotifyChangeKey(KeyHandle, Event, ApcRoutine, ApcContext,
/// IoStatusBlock*, CompletionFilter, WatchTree, Buffer, BufferSize,
/// Asynchronous)`: queues the request and returns `STATUS_PENDING`
/// (Learn). Until the kernel can block a thread in a service and deliver
/// user APCs (the processes/threads and synchronisation milestones), a
/// synchronous call or an APC routine is `STATUS_NOT_IMPLEMENTED`; without
/// an event the caller learns of the change only through its I/O status
/// block (Learn says the key handle is signalled then; Poly's key objects
/// are not waitable yet). `Buffer` and `BufferSize` are reserved and
/// ignored.
pub fn notify_change_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, event, apc, _apc_context, iosb, filter, subtree, _buf, _size, asynchronous] = abi::args::<K, 10>(k, regs)?;
    if asynchronous as u8 == 0 || apc != 0 {
        return Err(STATUS_NOT_IMPLEMENTED);
    }
    let caller = k.caller();
    k.with_state(|sys, _, _| {
        super::queue_notify(sys, &caller, Handle(h), Handle(event), iosb, filter as u32, subtree as u8 != 0)
    })?;
    Ok(STATUS_PENDING)
}

/// `NtSetInformationKey(KeyHandle, KeySetInformationClass,
/// KeySetInformation, Length)`: `KeyWriteTimeInformation`
/// (`KEY_WRITE_TIME_INFORMATION`, one `LARGE_INTEGER`) sets the last-write
/// time; a length other than 8 is `STATUS_INFO_LENGTH_MISMATCH`, another
/// class `STATUS_INVALID_INFO_CLASS` (Learn calls them "Reserved for system
/// use"; both statuses to be confirmed).
pub fn set_information_key<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, length] = abi::args::<K, 4>(k, regs)?;
    if class as u32 != KEY_WRITE_TIME_INFORMATION {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    if length as u32 != 8 {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    let time = abi::read_u64(k, buf)?;
    with_key(k, h, KEY_SET_VALUE, |reg, key| reg.set_last_write(key, time))?;
    Ok(STATUS_SUCCESS)
}
