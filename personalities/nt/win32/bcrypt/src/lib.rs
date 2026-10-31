//! Our `bcrypt.dll` (ADR 0096): CNG's primitive functions. Algorithm
//! providers, hashes and HMACs (MD5, SHA-1, SHA-256/384/512), the
//! system-preferred random number generator, AES in ECB/CBC/CFB/GCM, key
//! blobs, PBKDF2, RSA and ECDSA signatures, RSA encryption and ECDH key
//! agreement with `BCryptDeriveKey`'s KDFs.
//!
//! Every decision is `poly_wincrypt`'s (`cng`, `keys`), the algorithms
//! `poly_crypto`'s; this file reads and writes the callers' memory and
//! keeps the objects. Handles are addresses of boxed objects, checked
//! against the list of live ones before use (an unknown handle is
//! `STATUS_INVALID_HANDLE`). Callers' object buffers (`pbHashObject`,
//! `pbKeyObject`) are never written: the objects live on the process heap.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::vec::Vec;
use poly_win32_cryptrt::{self as rt, Handles};
use poly_wincrypt::cng::{self, AlgKind, AuthParams, ChainMode, HashObject, SymKey};
use poly_wincrypt::keys::{self, AsymKey, Padding};
use poly_wincrypt::status::*;

type Handle = usize;

// The objects are boxed handles already; their sizes do not matter.
#[allow(clippy::large_enum_variant)]
enum Obj {
    Alg {
        kind: AlgKind,
        mode: ChainMode,
    },
    Hash(HashObject),
    Sym(SymKey),
    Asym(AsymKey),
    /// A key pair `BCryptGenerateKeyPair` made, until `BCryptFinalizeKeyPair`.
    Pending {
        kind: AlgKind,
        bits: u32,
    },
    Secret(Vec<u8>),
}

static HANDLES: Handles = Handles::new();

/// The object behind a live handle.
///
/// # Safety
/// No other reference to the same object is alive (CNG objects are not
/// used from two threads at once, Learn: "Threading Issues").
unsafe fn obj<'a>(h: Handle) -> Option<&'a mut Obj> {
    if HANDLES.contains(h) {
        // SAFETY: a live Box<Obj> made by `new_handle`.
        Some(unsafe { &mut *(h as *mut Obj) })
    } else {
        None
    }
}

/// A shared reference to the object behind a live handle.
///
/// # Safety
/// No mutable reference to the same object is alive.
unsafe fn obj_ref<'a>(h: Handle) -> Option<&'a Obj> {
    if HANDLES.contains(h) {
        // SAFETY: a live Box<Obj> made by `new_handle`.
        Some(unsafe { &*(h as *const Obj) })
    } else {
        None
    }
}

fn new_handle(o: Obj) -> Handle {
    HANDLES.add(o)
}

fn destroy(h: Handle, check: fn(&Obj) -> bool) -> NtStatus {
    // SAFETY: looked up before it is removed.
    match unsafe { obj(h) } {
        Some(o) if check(o) => {
            if HANDLES.remove(h) {
                // SAFETY: made by `new_handle`, now unregistered.
                unsafe { Handles::free::<Obj>(h) };
            }
            STATUS_SUCCESS
        }
        _ => STATUS_INVALID_HANDLE,
    }
}

/// An algorithm provider: a pseudo-handle or an opened provider.
fn alg(h: Handle) -> Option<(AlgKind, ChainMode)> {
    if let Some(k) = cng::pseudo_handle(h) {
        return Some((k, ChainMode::Cbc));
    }
    // SAFETY: only read.
    match unsafe { obj_ref(h) } {
        Some(Obj::Alg { kind, mode }) => Some((*kind, *mode)),
        _ => None,
    }
}

/// Write `data` to an output buffer the CNG way: `pcbResult` gets the
/// size; a NULL buffer is a size query; a short one
/// `STATUS_BUFFER_TOO_SMALL`.
unsafe fn output(data: &[u8], out: *mut u8, cb: u32, result: *mut u32) -> NtStatus {
    if result.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: the caller's DWORD.
    unsafe { result.write(data.len() as u32) };
    if out.is_null() {
        return STATUS_SUCCESS;
    }
    if (cb as usize) < data.len() {
        return STATUS_BUFFER_TOO_SMALL;
    }
    // SAFETY: the caller's buffer of `cb` bytes.
    unsafe { core::ptr::copy(data.as_ptr(), out, data.len()) };
    STATUS_SUCCESS
}

fn status<T>(r: Result<T, NtStatus>) -> NtStatus {
    r.map_or_else(|e| e, |_| STATUS_SUCCESS)
}

// ----- Providers and properties -----------------------------------------------------------

/// `BCryptOpenAlgorithmProvider`: `pszImplementation` NULL or the
/// Microsoft Primitive Provider (anything else: `STATUS_NOT_FOUND`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptOpenAlgorithmProvider(
    ph: *mut Handle,
    id: *const u16,
    implementation: *const u16,
    flags: u32,
) -> NtStatus {
    if ph.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: NUL-terminated strings or NULL.
    let Some(id) = (unsafe { rt::wide_str(id) }) else { return STATUS_INVALID_PARAMETER };
    // SAFETY: as above.
    if let Some(i) = unsafe { rt::wide_str(implementation) }
        && i != cng::MS_PRIMITIVE_PROVIDER
    {
        return STATUS_NOT_FOUND;
    }
    match cng::open_algorithm(&id, flags) {
        Ok(kind) => {
            // SAFETY: the caller's handle slot.
            unsafe { ph.write(new_handle(Obj::Alg { kind, mode: ChainMode::Cbc })) };
            STATUS_SUCCESS
        }
        Err(e) => e,
    }
}

/// `BCryptCloseAlgorithmProvider`.
#[unsafe(no_mangle)]
pub extern "system" fn BCryptCloseAlgorithmProvider(h: Handle, _flags: u32) -> NtStatus {
    destroy(h, |o| matches!(o, Obj::Alg { .. }))
}

/// `BCryptGetProperty` of a provider, hash or key.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptGetProperty(
    h: Handle,
    prop: *const u16,
    out: *mut u8,
    cb: u32,
    result: *mut u32,
    _flags: u32,
) -> NtStatus {
    // SAFETY: a NUL-terminated string or NULL.
    let Some(prop) = (unsafe { rt::wide_str(prop) }) else { return STATUS_INVALID_PARAMETER };
    let value = if let Some(k) = cng::pseudo_handle(h) {
        cng::algorithm_property(k, ChainMode::Cbc, &prop)
    } else {
        // SAFETY: only read.
        match unsafe { obj(h) } {
            Some(Obj::Alg { kind, mode }) => cng::algorithm_property(*kind, *mode, &prop),
            Some(Obj::Hash(o)) => o.property(&prop),
            Some(Obj::Sym(k)) => k.property(&prop),
            Some(Obj::Asym(k)) => match prop.as_str() {
                cng::BCRYPT_KEY_LENGTH | cng::BCRYPT_KEY_STRENGTH => Ok(cng::dword(k.bits())),
                cng::BCRYPT_SIGNATURE_LENGTH => Ok(cng::dword(k.signature_len() as u32)),
                cng::BCRYPT_ECC_CURVE_NAME => match k {
                    AsymKey::Ec { curve, .. } => Ok(cng::wide_bytes(cng::curve_name(*curve))),
                    _ => Err(STATUS_NOT_SUPPORTED),
                },
                _ => Err(STATUS_NOT_SUPPORTED),
            },
            Some(_) => Err(STATUS_NOT_SUPPORTED),
            None => Err(STATUS_INVALID_HANDLE),
        }
    };
    match value {
        // SAFETY: the caller's output buffer and DWORD.
        Ok(v) => unsafe { output(&v, out, cb, result) },
        Err(e) => e,
    }
}

/// `BCryptSetProperty`: `ChainingMode` on an AES provider or key,
/// `MessageBlockLength` on an AES key, `ECCCurveName` on the generic ECDSA
/// and ECDH providers.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptSetProperty(
    h: Handle,
    prop: *const u16,
    input: *const u8,
    cb: u32,
    _flags: u32,
) -> NtStatus {
    // SAFETY: a NUL-terminated string or NULL; the caller's input bytes.
    let (Some(prop), value) = (unsafe { rt::wide_str(prop) }, unsafe { rt::bytes(input, cb as usize) }) else {
        return STATUS_INVALID_PARAMETER;
    };
    // SAFETY: no other reference.
    match unsafe { obj(h) } {
        Some(Obj::Alg { kind, mode }) => match (prop.as_str(), *kind) {
            (cng::BCRYPT_CHAINING_MODE, AlgKind::Aes) => {
                match cng::decode_wide(value).and_then(|s| ChainMode::from_name(&s)) {
                    Some(m) => {
                        *mode = m;
                        STATUS_SUCCESS
                    }
                    None => STATUS_NOT_SUPPORTED,
                }
            }
            (cng::BCRYPT_ECC_CURVE_NAME, AlgKind::Ecdsa(None) | AlgKind::Ecdh(None)) => {
                let Some(c) = cng::decode_wide(value).and_then(|s| cng::curve_by_name(&s)) else {
                    return STATUS_NOT_SUPPORTED;
                };
                *kind =
                    if matches!(kind, AlgKind::Ecdsa(_)) { AlgKind::Ecdsa(Some(c)) } else { AlgKind::Ecdh(Some(c)) };
                STATUS_SUCCESS
            }
            _ => STATUS_NOT_SUPPORTED,
        },
        Some(Obj::Sym(k)) => status(k.set_property(&prop, value)),
        Some(_) => STATUS_NOT_SUPPORTED,
        None if cng::pseudo_handle(h).is_some() => STATUS_NOT_SUPPORTED,
        None => STATUS_INVALID_HANDLE,
    }
}

/// `BCryptGetFipsAlgorithmMode`: Poly's providers are not FIPS-validated.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptGetFipsAlgorithmMode(enabled: *mut u8) -> NtStatus {
    if enabled.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: the caller's BOOLEAN.
    unsafe { enabled.write(0) };
    STATUS_SUCCESS
}

// ----- Hashes -------------------------------------------------------------------------------

/// `BCryptCreateHash`. A given object buffer must be at least
/// `ObjectLength` bytes (it is not used).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptCreateHash(
    h: Handle,
    ph: *mut Handle,
    object: *mut u8,
    cb_object: u32,
    secret: *const u8,
    cb_secret: u32,
    flags: u32,
) -> NtStatus {
    let Some((kind, _)) = alg(h) else { return STATUS_INVALID_HANDLE };
    if ph.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    if !object.is_null() && cb_object < cng::OBJECT_LENGTH {
        return STATUS_BUFFER_TOO_SMALL;
    }
    // SAFETY: the caller's secret bytes.
    let secret = unsafe { rt::bytes(secret, cb_secret as usize) };
    match HashObject::new(kind, secret, flags) {
        Ok(o) => {
            // SAFETY: the caller's handle slot.
            unsafe { ph.write(new_handle(Obj::Hash(o))) };
            STATUS_SUCCESS
        }
        Err(e) => e,
    }
}

/// `BCryptHashData`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptHashData(h: Handle, input: *const u8, cb: u32, _flags: u32) -> NtStatus {
    // SAFETY: no other reference; the caller's input bytes.
    match unsafe { obj(h) } {
        Some(Obj::Hash(o)) => status(o.update(unsafe { rt::bytes(input, cb as usize) })),
        _ => STATUS_INVALID_HANDLE,
    }
}

/// `BCryptFinishHash`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptFinishHash(h: Handle, out: *mut u8, cb: u32, _flags: u32) -> NtStatus {
    // SAFETY: no other reference.
    let Some(Obj::Hash(o)) = (unsafe { obj(h) }) else { return STATUS_INVALID_HANDLE };
    if out.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    match o.finish(cb as usize) {
        Ok(d) => {
            // SAFETY: the caller's buffer of `cb` == digest length bytes.
            unsafe { core::ptr::copy_nonoverlapping(d.as_ptr(), out, d.len()) };
            STATUS_SUCCESS
        }
        Err(e) => e,
    }
}

/// `BCryptDuplicateHash`: an independent copy of the state.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptDuplicateHash(
    h: Handle,
    ph: *mut Handle,
    object: *mut u8,
    cb_object: u32,
    _flags: u32,
) -> NtStatus {
    // SAFETY: only read.
    let Some(Obj::Hash(o)) = (unsafe { obj(h) }) else { return STATUS_INVALID_HANDLE };
    if ph.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    if !object.is_null() && cb_object < cng::OBJECT_LENGTH {
        return STATUS_BUFFER_TOO_SMALL;
    }
    let copy = o.clone();
    // SAFETY: the caller's handle slot.
    unsafe { ph.write(new_handle(Obj::Hash(copy))) };
    STATUS_SUCCESS
}

/// `BCryptDestroyHash`.
#[unsafe(no_mangle)]
pub extern "system" fn BCryptDestroyHash(h: Handle) -> NtStatus {
    destroy(h, |o| matches!(o, Obj::Hash(_)))
}

/// `BCryptHash`: one call over one buffer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptHash(
    h: Handle,
    secret: *const u8,
    cb_secret: u32,
    input: *const u8,
    cb_input: u32,
    out: *mut u8,
    cb_out: u32,
) -> NtStatus {
    let Some((kind, _)) = alg(h) else { return STATUS_INVALID_HANDLE };
    if out.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: the caller's buffers.
    let (secret, input) = unsafe { (rt::bytes(secret, cb_secret as usize), rt::bytes(input, cb_input as usize)) };
    match cng::hash_once(kind, secret, input, cb_out as usize) {
        Ok(d) => {
            // SAFETY: the caller's buffer of `cb_out` == digest length bytes.
            unsafe { core::ptr::copy_nonoverlapping(d.as_ptr(), out, d.len()) };
            STATUS_SUCCESS
        }
        Err(e) => e,
    }
}

// ----- Random numbers ----------------------------------------------------------------------

/// `BCryptGenRandom`: an RNG provider, or NULL with
/// `BCRYPT_USE_SYSTEM_PREFERRED_RNG` (Learn); the flag with a handle is
/// `STATUS_INVALID_PARAMETER`. `BCRYPT_RNG_USE_ENTROPY_IN_BUFFER` is
/// ignored (as Learn says Windows 8 and later do).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptGenRandom(h: Handle, buf: *mut u8, cb: u32, flags: u32) -> NtStatus {
    if flags & !(cng::BCRYPT_USE_SYSTEM_PREFERRED_RNG | cng::BCRYPT_RNG_USE_ENTROPY_IN_BUFFER) != 0 {
        return STATUS_INVALID_PARAMETER;
    }
    if flags & cng::BCRYPT_USE_SYSTEM_PREFERRED_RNG != 0 {
        if h != 0 {
            return STATUS_INVALID_PARAMETER;
        }
    } else if alg(h).map(|a| a.0) != Some(AlgKind::Rng) {
        return STATUS_INVALID_HANDLE;
    }
    if cb == 0 {
        return STATUS_SUCCESS;
    }
    if buf.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: the caller's buffer of `cb` bytes.
    if rt::random(unsafe { rt::bytes_mut(buf, cb as usize) }) { STATUS_SUCCESS } else { STATUS_INTERNAL_ERROR }
}

// ----- Symmetric keys ----------------------------------------------------------------------

/// `BCryptGenerateSymmetricKey` (AES); the key takes the provider's
/// chaining mode.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptGenerateSymmetricKey(
    h: Handle,
    ph: *mut Handle,
    object: *mut u8,
    cb_object: u32,
    secret: *const u8,
    cb_secret: u32,
    _flags: u32,
) -> NtStatus {
    let Some((kind, mode)) = alg(h) else { return STATUS_INVALID_HANDLE };
    if kind != AlgKind::Aes {
        return STATUS_INVALID_HANDLE;
    }
    if ph.is_null() || secret.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    if !object.is_null() && cb_object < cng::OBJECT_LENGTH {
        return STATUS_BUFFER_TOO_SMALL;
    }
    // SAFETY: the caller's secret.
    match SymKey::new(unsafe { rt::bytes(secret, cb_secret as usize) }, mode) {
        Ok(k) => {
            // SAFETY: the caller's handle slot.
            unsafe { ph.write(new_handle(Obj::Sym(k))) };
            STATUS_SUCCESS
        }
        Err(e) => e,
    }
}

/// Read a `BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO` (x64 layout, `bcrypt.h`).
struct AuthInfo {
    p: *mut u8,
}

impl AuthInfo {
    unsafe fn u32_at(&self, o: usize) -> u32 {
        // SAFETY: inside the structure (the caller checked cbSize).
        unsafe { self.p.add(o).cast::<u32>().read_unaligned() }
    }
    unsafe fn ptr_at(&self, o: usize) -> *mut u8 {
        // SAFETY: as above.
        unsafe { self.p.add(o).cast::<*mut u8>().read_unaligned() }
    }
    unsafe fn put_u32(&self, o: usize, v: u32) {
        // SAFETY: as above.
        unsafe { self.p.add(o).cast::<u32>().write_unaligned(v) }
    }
}

/// One `BCryptEncrypt`/`BCryptDecrypt` call.
#[allow(clippy::too_many_arguments)]
unsafe fn crypt(
    encrypt: bool,
    h: Handle,
    input: *const u8,
    cb_input: u32,
    padding_info: *mut u8,
    iv: *mut u8,
    cb_iv: u32,
    out: *mut u8,
    cb_out: u32,
    result: *mut u32,
    flags: u32,
) -> NtStatus {
    if result.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: the caller's input bytes (copied before any output is written).
    let data = unsafe { rt::bytes(input, cb_input as usize) };
    // SAFETY: no other reference.
    match unsafe { obj(h) } {
        Some(Obj::Sym(k)) => {
            if k.mode == ChainMode::Gcm {
                if padding_info.is_null() || flags & cng::BCRYPT_BLOCK_PADDING != 0 {
                    return STATUS_INVALID_PARAMETER;
                }
                let info = AuthInfo { p: padding_info };
                // SAFETY: cbSize and dwInfoVersion lead the structure.
                let (size, version) = unsafe { (info.u32_at(0), info.u32_at(4)) };
                if size < cng::AUTH_INFO_SIZE || version != cng::BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION {
                    return STATUS_INVALID_PARAMETER;
                }
                if out.is_null() {
                    // SAFETY: the caller's DWORD.
                    unsafe { result.write(cb_input) };
                    return STATUS_SUCCESS;
                }
                if cb_out < cb_input {
                    return STATUS_BUFFER_TOO_SMALL;
                }
                // SAFETY: the fields of a structure of at least AUTH_INFO_SIZE bytes; the buffers they name.
                let r = unsafe {
                    let nonce = rt::bytes(info.ptr_at(8), info.u32_at(16) as usize);
                    let aad = rt::bytes(info.ptr_at(24), info.u32_at(32) as usize);
                    let tag = rt::bytes_mut(info.ptr_at(40), info.u32_at(48) as usize);
                    let mode_flags = info.u32_at(80);
                    let chain = mode_flags & cng::BCRYPT_AUTH_MODE_CHAIN_CALLS_FLAG != 0;
                    let r = k.gcm(encrypt, data, AuthParams { nonce, aad, tag, chain });
                    if r.is_ok() {
                        let progress = if chain { cng::BCRYPT_AUTH_MODE_IN_PROGRESS_FLAG } else { 0 };
                        info.put_u32(80, (mode_flags & !cng::BCRYPT_AUTH_MODE_IN_PROGRESS_FLAG) | progress);
                    }
                    r
                };
                return match r {
                    // SAFETY: the caller's output buffer and DWORD.
                    Ok(v) => unsafe { output(&v, out, cb_out, result) },
                    Err(e) => e,
                };
            }
            let padding = flags & cng::BCRYPT_BLOCK_PADDING != 0;
            if out.is_null() {
                return match k.output_len(encrypt, data.len(), padding) {
                    Ok(n) => {
                        // SAFETY: the caller's DWORD.
                        unsafe { result.write(n as u32) };
                        STATUS_SUCCESS
                    }
                    Err(e) => e,
                };
            }
            let ivs = if iv.is_null() {
                None
            } else {
                // SAFETY: the caller's IV buffer of `cb_iv` bytes.
                Some(unsafe { rt::bytes_mut(iv, cb_iv as usize) })
            };
            // Check the size before the key's state moves on.
            if encrypt
                && let Ok(n) = k.output_len(true, data.len(), padding)
                && (cb_out as usize) < n
            {
                // SAFETY: the caller's DWORD.
                unsafe { result.write(n as u32) };
                return STATUS_BUFFER_TOO_SMALL;
            }
            let r = if encrypt { k.encrypt(data, ivs, padding) } else { k.decrypt(data, ivs, padding) };
            match r {
                // SAFETY: the caller's output buffer and DWORD.
                Ok(v) => unsafe { output(&v, out, cb_out, result) },
                Err(e) => e,
            }
        }
        Some(Obj::Asym(k)) => {
            // SAFETY: the caller's padding info for these flags.
            let padding = match unsafe { padding(flags, padding_info, true) } {
                Ok(p) => p,
                Err(e) => return e,
            };
            if out.is_null() {
                // SAFETY: the caller's DWORD.
                unsafe { result.write(k.cipher_len() as u32) };
                return STATUS_SUCCESS;
            }
            let r = if encrypt {
                if !rt::random(&mut [0u8; 1]) {
                    return STATUS_INTERNAL_ERROR;
                }
                k.encrypt(&padding, data, &mut rt::rng)
            } else {
                k.decrypt(&padding, data)
            };
            match r {
                // SAFETY: the caller's output buffer and DWORD.
                Ok(v) => unsafe { output(&v, out, cb_out, result) },
                Err(e) => e,
            }
        }
        _ => STATUS_INVALID_HANDLE,
    }
}

/// `BCryptEncrypt`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptEncrypt(
    h: Handle,
    input: *const u8,
    cb_input: u32,
    padding_info: *mut u8,
    iv: *mut u8,
    cb_iv: u32,
    out: *mut u8,
    cb_out: u32,
    result: *mut u32,
    flags: u32,
) -> NtStatus {
    // SAFETY: the caller's arguments, as documented.
    unsafe { crypt(true, h, input, cb_input, padding_info, iv, cb_iv, out, cb_out, result, flags) }
}

/// `BCryptDecrypt`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptDecrypt(
    h: Handle,
    input: *const u8,
    cb_input: u32,
    padding_info: *mut u8,
    iv: *mut u8,
    cb_iv: u32,
    out: *mut u8,
    cb_out: u32,
    result: *mut u32,
    flags: u32,
) -> NtStatus {
    // SAFETY: the caller's arguments, as documented.
    unsafe { crypt(false, h, input, cb_input, padding_info, iv, cb_iv, out, cb_out, result, flags) }
}

/// `BCryptImportKey` (symmetric blobs).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptImportKey(
    h: Handle,
    import_key: Handle,
    blob_type: *const u16,
    ph: *mut Handle,
    object: *mut u8,
    cb_object: u32,
    input: *const u8,
    cb_input: u32,
    _flags: u32,
) -> NtStatus {
    let Some((kind, mode)) = alg(h) else { return STATUS_INVALID_HANDLE };
    if kind != AlgKind::Aes {
        return STATUS_INVALID_HANDLE;
    }
    if import_key != 0 {
        // Wrapped (encrypted) blobs are not supported.
        return STATUS_NOT_SUPPORTED;
    }
    // SAFETY: a NUL-terminated string or NULL.
    let Some(t) = (unsafe { rt::wide_str(blob_type) }) else { return STATUS_INVALID_PARAMETER };
    if ph.is_null() || input.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    if !object.is_null() && cb_object < cng::OBJECT_LENGTH {
        return STATUS_BUFFER_TOO_SMALL;
    }
    // SAFETY: the caller's blob.
    match SymKey::import(&t, unsafe { rt::bytes(input, cb_input as usize) }, mode) {
        Ok(k) => {
            // SAFETY: the caller's handle slot.
            unsafe { ph.write(new_handle(Obj::Sym(k))) };
            STATUS_SUCCESS
        }
        Err(e) => e,
    }
}

/// `BCryptExportKey` (symmetric and asymmetric blobs).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptExportKey(
    h: Handle,
    export_key: Handle,
    blob_type: *const u16,
    out: *mut u8,
    cb_out: u32,
    result: *mut u32,
    _flags: u32,
) -> NtStatus {
    if export_key != 0 {
        return STATUS_NOT_SUPPORTED;
    }
    // SAFETY: a NUL-terminated string or NULL.
    let Some(t) = (unsafe { rt::wide_str(blob_type) }) else { return STATUS_INVALID_PARAMETER };
    // SAFETY: only read.
    let blob = match unsafe { obj(h) } {
        Some(Obj::Sym(k)) => k.export(&t),
        Some(Obj::Asym(k)) => k.export(&t),
        _ => return STATUS_INVALID_HANDLE,
    };
    match blob {
        // SAFETY: the caller's output buffer and DWORD.
        Ok(b) => unsafe { output(&b, out, cb_out, result) },
        Err(e) => e,
    }
}

/// `BCryptDuplicateKey` (symmetric keys).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptDuplicateKey(
    h: Handle,
    ph: *mut Handle,
    _object: *mut u8,
    _cb_object: u32,
    _flags: u32,
) -> NtStatus {
    // SAFETY: only read.
    let Some(Obj::Sym(k)) = (unsafe { obj(h) }) else { return STATUS_INVALID_HANDLE };
    if ph.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let copy = k.duplicate();
    // SAFETY: the caller's handle slot.
    unsafe { ph.write(new_handle(Obj::Sym(copy))) };
    STATUS_SUCCESS
}

/// `BCryptDestroyKey`.
#[unsafe(no_mangle)]
pub extern "system" fn BCryptDestroyKey(h: Handle) -> NtStatus {
    destroy(h, |o| matches!(o, Obj::Sym(_) | Obj::Asym(_) | Obj::Pending { .. }))
}

/// `BCryptDeriveKeyPBKDF2`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptDeriveKeyPBKDF2(
    prf: Handle,
    password: *const u8,
    cb_password: u32,
    salt: *const u8,
    cb_salt: u32,
    iterations: u64,
    out: *mut u8,
    cb_out: u32,
    _flags: u32,
) -> NtStatus {
    let Some((kind, _)) = alg(prf) else { return STATUS_INVALID_HANDLE };
    if out.is_null() || cb_out == 0 {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: the caller's buffers.
    unsafe {
        let pw = rt::bytes(password, cb_password as usize);
        let salt = rt::bytes(salt, cb_salt as usize);
        status(cng::pbkdf2(kind, pw, salt, iterations, rt::bytes_mut(out, cb_out as usize)))
    }
}

// ----- Asymmetric keys ---------------------------------------------------------------------

/// The padding the flags and padding info describe (`bcrypt.h`'s
/// `BCRYPT_PKCS1_PADDING_INFO`, `BCRYPT_PSS_PADDING_INFO`,
/// `BCRYPT_OAEP_PADDING_INFO`, x64 layouts). `cipher`: an encryption call.
unsafe fn padding(flags: u32, info: *const u8, cipher: bool) -> Result<Padding, NtStatus> {
    let pads = flags & (cng::BCRYPT_PAD_NONE | cng::BCRYPT_PAD_PKCS1 | cng::BCRYPT_PAD_OAEP | cng::BCRYPT_PAD_PSS);
    let hash_at = |o: usize| -> Result<Option<poly_crypto::HashAlg>, NtStatus> {
        if info.is_null() {
            return Ok(None);
        }
        // SAFETY: a padding info structure with a string pointer at `o`.
        let s = unsafe { info.add(o).cast::<*const u16>().read_unaligned() };
        // SAFETY: the string it names, or NULL.
        match unsafe { rt::wide_str(s) } {
            None => Ok(None),
            Some(n) => cng::hash_by_name(&n).map(Some).ok_or(STATUS_NOT_SUPPORTED),
        }
    };
    Ok(match pads {
        0 if !cipher => Padding::None,
        cng::BCRYPT_PAD_NONE if cipher => Padding::None,
        cng::BCRYPT_PAD_PKCS1 if cipher => Padding::Pkcs1(None),
        cng::BCRYPT_PAD_PKCS1 => Padding::Pkcs1(hash_at(0)?),
        cng::BCRYPT_PAD_PSS if !cipher && !info.is_null() => {
            let hash = hash_at(0)?.ok_or(STATUS_INVALID_PARAMETER)?;
            // SAFETY: cbSalt follows the string pointer.
            let salt = unsafe { info.add(8).cast::<u32>().read_unaligned() };
            Padding::Pss { hash, salt_len: salt as usize }
        }
        cng::BCRYPT_PAD_OAEP if cipher && !info.is_null() => {
            let hash = hash_at(0)?.ok_or(STATUS_INVALID_PARAMETER)?;
            // SAFETY: pbLabel and cbLabel follow the string pointer.
            let label = unsafe {
                let p = info.add(8).cast::<*const u8>().read_unaligned();
                let n = info.add(16).cast::<u32>().read_unaligned();
                rt::bytes(p, n as usize).to_vec()
            };
            Padding::Oaep { hash, label }
        }
        _ => return Err(STATUS_INVALID_PARAMETER),
    })
}

/// `BCryptImportKeyPair`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptImportKeyPair(
    h: Handle,
    import_key: Handle,
    blob_type: *const u16,
    ph: *mut Handle,
    input: *const u8,
    cb_input: u32,
    _flags: u32,
) -> NtStatus {
    let Some((kind, _)) = alg(h) else { return STATUS_INVALID_HANDLE };
    if import_key != 0 {
        return STATUS_NOT_SUPPORTED;
    }
    // SAFETY: a NUL-terminated string or NULL.
    let Some(t) = (unsafe { rt::wide_str(blob_type) }) else { return STATUS_INVALID_PARAMETER };
    if ph.is_null() || input.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: the caller's blob.
    match AsymKey::import(kind, &t, unsafe { rt::bytes(input, cb_input as usize) }) {
        Ok(k) => {
            // SAFETY: the caller's handle slot.
            unsafe { ph.write(new_handle(Obj::Asym(k))) };
            STATUS_SUCCESS
        }
        Err(e) => e,
    }
}

/// `BCryptGenerateKeyPair`: the key is made by `BCryptFinalizeKeyPair`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptGenerateKeyPair(h: Handle, ph: *mut Handle, bits: u32, _flags: u32) -> NtStatus {
    let Some((kind, _)) = alg(h) else { return STATUS_INVALID_HANDLE };
    if ph.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    match kind {
        AlgKind::Ecdsa(_) | AlgKind::Ecdh(_) => {}
        AlgKind::Rsa { .. } => return STATUS_NOT_SUPPORTED,
        _ => return STATUS_INVALID_HANDLE,
    }
    // SAFETY: the caller's handle slot.
    unsafe { ph.write(new_handle(Obj::Pending { kind, bits })) };
    STATUS_SUCCESS
}

/// `BCryptFinalizeKeyPair`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptFinalizeKeyPair(h: Handle, _flags: u32) -> NtStatus {
    // SAFETY: no other reference.
    let Some(o) = (unsafe { obj(h) }) else { return STATUS_INVALID_HANDLE };
    let Obj::Pending { kind, bits } = *o else { return STATUS_INVALID_HANDLE };
    if !rt::random(&mut [0u8; 1]) {
        return STATUS_INTERNAL_ERROR;
    }
    match AsymKey::generate(kind, bits, &mut rt::rng) {
        Ok(k) => {
            *o = Obj::Asym(k);
            STATUS_SUCCESS
        }
        Err(e) => e,
    }
}

/// `BCryptSignHash`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptSignHash(
    h: Handle,
    padding_info: *const u8,
    input: *const u8,
    cb_input: u32,
    out: *mut u8,
    cb_out: u32,
    result: *mut u32,
    flags: u32,
) -> NtStatus {
    // SAFETY: only read.
    let Some(Obj::Asym(k)) = (unsafe { obj(h) }) else { return STATUS_INVALID_HANDLE };
    // SAFETY: the caller's padding info for these flags.
    let pad = match unsafe { padding(flags, padding_info, false) } {
        Ok(p) => p,
        Err(e) => return e,
    };
    if result.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    if out.is_null() {
        // SAFETY: the caller's DWORD.
        unsafe { result.write(k.signature_len() as u32) };
        return STATUS_SUCCESS;
    }
    if (cb_out as usize) < k.signature_len() {
        return STATUS_BUFFER_TOO_SMALL;
    }
    if !rt::random(&mut [0u8; 1]) {
        return STATUS_INTERNAL_ERROR;
    }
    // SAFETY: the caller's hash.
    match k.sign(&pad, unsafe { rt::bytes(input, cb_input as usize) }, &mut rt::rng) {
        // SAFETY: the caller's output buffer and DWORD.
        Ok(sig) => unsafe { output(&sig, out, cb_out, result) },
        Err(e) => e,
    }
}

/// `BCryptVerifySignature`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptVerifySignature(
    h: Handle,
    padding_info: *const u8,
    hash: *const u8,
    cb_hash: u32,
    sig: *const u8,
    cb_sig: u32,
    flags: u32,
) -> NtStatus {
    // SAFETY: only read.
    let Some(Obj::Asym(k)) = (unsafe { obj(h) }) else { return STATUS_INVALID_HANDLE };
    // SAFETY: the caller's padding info for these flags.
    let pad = match unsafe { padding(flags, padding_info, false) } {
        Ok(p) => p,
        Err(e) => return e,
    };
    // SAFETY: the caller's hash and signature.
    let (hash, sig) = unsafe { (rt::bytes(hash, cb_hash as usize), rt::bytes(sig, cb_sig as usize)) };
    status(k.verify(&pad, hash, sig))
}

/// `BCryptSecretAgreement` (ECDH).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptSecretAgreement(
    private: Handle,
    public: Handle,
    ph: *mut Handle,
    _flags: u32,
) -> NtStatus {
    // SAFETY: only read (the two handles may be the same).
    let (Some(Obj::Asym(a)), Some(Obj::Asym(b))) = (unsafe { obj_ref(private) }, unsafe { obj_ref(public) }) else {
        return STATUS_INVALID_HANDLE;
    };
    if ph.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    match keys::agree(a, b) {
        Ok(z) => {
            // SAFETY: the caller's handle slot.
            unsafe { ph.write(new_handle(Obj::Secret(z))) };
            STATUS_SUCCESS
        }
        Err(e) => e,
    }
}

/// `BCryptDeriveKey` over an agreed secret (`BCryptBufferDesc` parameters,
/// x64 layout: `ulVersion`, `cBuffers`, `pBuffers`; each `BCryptBuffer`
/// `cbBuffer`, `BufferType`, `pvBuffer`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BCryptDeriveKey(
    h: Handle,
    kdf: *const u16,
    params: *const u8,
    out: *mut u8,
    cb_out: u32,
    result: *mut u32,
    _flags: u32,
) -> NtStatus {
    // SAFETY: only read.
    let Some(Obj::Secret(z)) = (unsafe { obj(h) }) else { return STATUS_INVALID_HANDLE };
    // SAFETY: a NUL-terminated string or NULL.
    let Some(kdf) = (unsafe { rt::wide_str(kdf) }) else { return STATUS_INVALID_PARAMETER };
    let mut list: Vec<(u32, &[u8])> = Vec::new();
    if !params.is_null() {
        // SAFETY: the caller's BCryptBufferDesc and the buffers it lists.
        unsafe {
            let n = params.add(4).cast::<u32>().read_unaligned() as usize;
            let bufs = params.add(8).cast::<*const u8>().read_unaligned();
            for i in 0..n {
                let b = bufs.add(16 * i);
                let cb = b.cast::<u32>().read_unaligned() as usize;
                let t = b.add(4).cast::<u32>().read_unaligned();
                let p = b.add(8).cast::<*const u8>().read_unaligned();
                list.push((t, rt::bytes(p, cb)));
            }
        }
    }
    let want = if out.is_null() { None } else { Some(cb_out as usize) };
    match keys::derive(z, &kdf, &list, want) {
        // SAFETY: the caller's output buffer and DWORD.
        Ok(v) => unsafe { output(&v, out, cb_out, result) },
        Err(e) => e,
    }
}

/// `BCryptDestroySecret`.
#[unsafe(no_mangle)]
pub extern "system" fn BCryptDestroySecret(h: Handle) -> NtStatus {
    destroy(h, |o| matches!(o, Obj::Secret(_)))
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn bcrypt_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
