//! Our `crypt32.dll` (ADR 0096): certificate contexts and stores (memory
//! stores and the system stores `ROOT`, `MY`, `CA` ... kept as files),
//! finding, name strings, `CryptDecodeObjectEx`, the Base64/hex string
//! conversions, chain building and policy checks against the `ROOT`
//! store, and `CryptProtectData` in Poly's documented local format.
//!
//! The decisions are `poly_wincrypt`'s (`decode`, `certstr`, `store`,
//! `dpapi`) and `poly_x509`'s (parsing, chain building); this file keeps
//! the objects and the callers' memory. Store handles and certificate
//! contexts are boxed objects checked against lists of live ones.
//!
//! Layouts (x64, `wincrypt.h`): `CERT_CONTEXT` = `dwCertEncodingType` 0,
//! `pbCertEncoded` 8, `cbCertEncoded` 16, `pCertInfo` 24, `hCertStore` 32.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use poly_crypto::HashAlg;
use poly_win32_cryptrt::{self as rt, Handles, SpinLock};
use poly_wincrypt::certstr::{self, NamePara};
use poly_wincrypt::decode::{self, X509_ASN_ENCODING};
use poly_wincrypt::flat::Flat;
use poly_wincrypt::status::*;
use poly_wincrypt::store::{self, AddAction, FindPara, Location, Provider};
use poly_x509::Certificate;
use poly_x509::chain::{self, error as trust};

type Bool = i32;
const TRUE: Bool = 1;

fn fail<T: Default>(e: u32) -> T {
    rt::set_last_error(e);
    T::default()
}

// ----- Certificate contexts ----------------------------------------------------------------

/// `CERT_CONTEXT` followed by Poly's bookkeeping; the context pointer is
/// the box's address (the first field).
#[repr(C)]
struct Ctx {
    encoding: u32,
    encoded_ptr: *const u8,
    encoded_len: u32,
    info: *const u8,
    store: usize,
    // Poly's part.
    refs: u32,
    cert: Certificate,
    info_block: Vec<u8>,
    friendly: Option<Vec<u16>>,
}

static CONTEXTS: Handles = Handles::new();
/// Reference counts and store lists change under this lock.
static LOCK: SpinLock<()> = SpinLock::new(());

/// A new context (one reference) for a parsed certificate.
fn new_ctx(cert: Certificate, store: usize) -> usize {
    let mut f = Flat::new(decode::CERT_INFO_SIZE);
    decode::put_cert_info(&mut f, 0, &cert);
    let mut block = alloc::vec![0u8; f.len()];
    let base = block.as_ptr() as usize;
    block.copy_from_slice(&f.finish(base));
    let ctx = Ctx {
        encoding: X509_ASN_ENCODING | decode::PKCS_7_ASN_ENCODING,
        encoded_ptr: cert.encoded.as_ptr(),
        encoded_len: cert.encoded.len() as u32,
        info: block.as_ptr(),
        store,
        refs: 1,
        cert,
        info_block: block,
        friendly: None,
    };
    CONTEXTS.add(ctx)
}

/// The context behind a pointer a caller passed.
unsafe fn ctx<'a>(p: usize) -> Option<&'a mut Ctx> {
    if CONTEXTS.contains(p) {
        // SAFETY: a live Box<Ctx>.
        Some(unsafe { &mut *(p as *mut Ctx) })
    } else {
        None
    }
}

fn ctx_ref(p: usize) -> usize {
    LOCK.with(|_| {
        // SAFETY: under the lock.
        if let Some(c) = unsafe { ctx(p) } {
            c.refs += 1;
        }
    });
    p
}

fn ctx_release(p: usize) {
    let free = LOCK.with(|_| {
        // SAFETY: under the lock.
        match unsafe { ctx(p) } {
            Some(c) => {
                c.refs -= 1;
                c.refs == 0
            }
            None => false,
        }
    });
    if free && CONTEXTS.remove(p) {
        // SAFETY: made by `new_ctx`, now unregistered.
        unsafe { Handles::free::<Ctx>(p) };
    }
}

// ----- Stores -------------------------------------------------------------------------------

struct Store {
    refs: u32,
    /// Contexts (one reference each held by the store).
    certs: Vec<usize>,
    /// Where added certificates are written (system stores).
    dir: Option<String>,
    readonly: bool,
}

static STORES: Handles = Handles::new();

unsafe fn store_obj<'a>(h: usize) -> Option<&'a mut Store> {
    if STORES.contains(h) {
        // SAFETY: a live Box<Store>.
        Some(unsafe { &mut *(h as *mut Store) })
    } else {
        None
    }
}

/// The root directory of a location.
fn location_root(l: Location) -> Option<String> {
    match l {
        Location::LocalMachine => rt::env("ProgramData").or_else(|| Some(String::from("C:\\ProgramData"))),
        Location::CurrentUser => rt::env("APPDATA"),
    }
}

/// The certificates of a system store's directories (the user's store
/// also shows the machine's, read-only).
fn load_system(l: Location, name: &str) -> (Vec<Certificate>, Option<String>) {
    let Some(sub) = store::store_subdir(name) else { return (Vec::new(), None) };
    let mut dirs = Vec::new();
    let own = location_root(l).map(|r| alloc::format!("{r}\\{sub}"));
    if let Some(d) = &own {
        dirs.push(d.clone());
    }
    if l == Location::CurrentUser
        && store::user_store_includes_machine(name)
        && let Some(r) = location_root(Location::LocalMachine)
    {
        dirs.push(alloc::format!("{r}\\{sub}"));
    }
    let mut certs: Vec<Certificate> = Vec::new();
    for d in &dirs {
        for f in rt::list_dir(d) {
            let Some(bytes) = rt::read_file(&alloc::format!("{d}\\{f}")) else { continue };
            if let Some(c) = store::parse_store_file(&bytes)
                && !certs.iter().any(|x| x.encoded == c.encoded)
            {
                certs.push(c);
            }
        }
    }
    (certs, own)
}

fn new_store(certs: Vec<Certificate>, dir: Option<String>, readonly: bool) -> usize {
    let h = STORES.add(Store { refs: 1, certs: Vec::new(), dir, readonly });
    let ctxs: Vec<usize> = certs.into_iter().map(|c| new_ctx(c, h)).collect();
    // SAFETY: just made.
    if let Some(s) = unsafe { store_obj(h) } {
        s.certs = ctxs;
    }
    h
}

/// `CertOpenStore`: the memory provider and the system providers
/// (`pvPara` the store name). Other providers: `ERROR_FILE_NOT_FOUND`
/// (to be confirmed: Windows' error for a provider it cannot find).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertOpenStore(
    provider: *const u8,
    _encoding: u32,
    _prov: usize,
    flags: u32,
    para: *const u8,
) -> usize {
    let p = provider as usize;
    let prov = if p < 0x1_0000 {
        store::provider(Some(p), None)
    } else {
        // SAFETY: a NUL-terminated provider name.
        store::provider(None, unsafe { rt::ansi_str(provider) }.as_deref())
    };
    let readonly = flags & store::CERT_STORE_READONLY_FLAG != 0;
    match prov {
        Some(Provider::Memory) => new_store(Vec::new(), None, readonly),
        Some(Provider::SystemW | Provider::SystemA) => {
            let name = if prov == Some(Provider::SystemA) {
                // SAFETY: a NUL-terminated name.
                unsafe { rt::ansi_str(para) }
            } else {
                // SAFETY: a NUL-terminated name.
                unsafe { rt::wide_str(para.cast()) }
            };
            let Some(name) = name else { return fail(E_INVALIDARG) };
            if store::store_subdir(&name).is_none() {
                return fail(ERROR_FILE_NOT_FOUND);
            }
            if flags & store::CERT_STORE_DELETE_FLAG != 0 {
                return fail(ERROR_NOT_SUPPORTED);
            }
            let (certs, dir) = load_system(store::location(flags), &name);
            new_store(certs, dir, readonly)
        }
        Some(Provider::Collection) | None => fail(ERROR_FILE_NOT_FOUND),
    }
}

/// `CertOpenSystemStoreW`: the current user's system store.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertOpenSystemStoreW(prov: usize, name: *const u16) -> usize {
    // SAFETY: the caller's name, passed on.
    unsafe {
        CertOpenStore(
            store::CERT_STORE_PROV_SYSTEM_W as *const u8,
            0,
            prov,
            store::CERT_SYSTEM_STORE_CURRENT_USER,
            name.cast(),
        )
    }
}

/// `CertOpenSystemStoreA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertOpenSystemStoreA(prov: usize, name: *const u8) -> usize {
    // SAFETY: the caller's name, passed on.
    unsafe {
        CertOpenStore(
            store::CERT_STORE_PROV_SYSTEM_A as *const u8,
            0,
            prov,
            store::CERT_SYSTEM_STORE_CURRENT_USER,
            name,
        )
    }
}

/// `CertDuplicateStore`.
#[unsafe(no_mangle)]
pub extern "system" fn CertDuplicateStore(h: usize) -> usize {
    // SAFETY: under the lock.
    LOCK.with(|_| match unsafe { store_obj(h) } {
        Some(s) => {
            s.refs += 1;
            h
        }
        None => 0,
    })
}

/// `CertCloseStore`: the last close releases the store's references to
/// its contexts (contexts the caller still holds stay valid).
/// `CERT_CLOSE_STORE_CHECK_FLAG` is accepted; Poly never reports
/// `CRYPT_E_PENDING_CLOSE`.
#[unsafe(no_mangle)]
pub extern "system" fn CertCloseStore(h: usize, _flags: u32) -> Bool {
    if h == 0 {
        return TRUE;
    }
    let last = LOCK.with(|_| {
        // SAFETY: under the lock.
        match unsafe { store_obj(h) } {
            Some(s) => {
                s.refs -= 1;
                Some(s.refs == 0)
            }
            None => None,
        }
    });
    match last {
        None => fail(ERROR_INVALID_HANDLE),
        Some(false) => TRUE,
        Some(true) => {
            STORES.remove(h);
            // SAFETY: unregistered: ours alone now.
            let s = unsafe { Box::from_raw(h as *mut Store) };
            for c in s.certs {
                ctx_release(c);
            }
            TRUE
        }
    }
}

/// The context after `prev` in the store that satisfies `pick` (with a
/// new reference); `prev` is released (Learn: always freed).
fn next_in_store(h: usize, prev: usize, pick: &dyn Fn(&Certificate) -> bool) -> usize {
    let found = LOCK.with(|_| {
        // SAFETY: under the lock.
        let s = unsafe { store_obj(h) }?;
        let start =
            if prev == 0 { 0 } else { s.certs.iter().position(|&c| c == prev).map_or(s.certs.len(), |i| i + 1) };
        s.certs[start..].iter().copied().find(|&c| {
            // SAFETY: a context the store holds.
            unsafe { ctx(c) }.is_some_and(|x| pick(&x.cert))
        })
    });
    if prev != 0 {
        ctx_release(prev);
    }
    match found {
        Some(c) => ctx_ref(c),
        None => fail(CRYPT_E_NOT_FOUND),
    }
}

/// `CertEnumCertificatesInStore`.
#[unsafe(no_mangle)]
pub extern "system" fn CertEnumCertificatesInStore(h: usize, prev: usize) -> usize {
    // SAFETY: only checked.
    if unsafe { store_obj(h) }.is_none() {
        return fail(ERROR_INVALID_HANDLE);
    }
    next_in_store(h, prev, &|_| true)
}

/// Read `pvFindPara` for a find type.
unsafe fn find_para(find_type: u32, para: *const u8) -> Option<FindPara> {
    let blob = |p: *const u8| -> Vec<u8> {
        // SAFETY: a CRYPT_DATA_BLOB: cbData, then pbData at 8.
        unsafe {
            let n = p.cast::<u32>().read_unaligned() as usize;
            rt::bytes(p.add(8).cast::<*const u8>().read_unaligned(), n).to_vec()
        }
    };
    if para.is_null() && find_type != store::CERT_FIND_ANY {
        return None;
    }
    // SAFETY: for the string find types `para` is a NUL-terminated string.
    let wide = || unsafe { rt::wide_str(para.cast()) };
    // SAFETY: as above.
    let ansi = || unsafe { rt::ansi_str(para) };
    Some(match find_type {
        store::CERT_FIND_ANY => FindPara::Any,
        store::CERT_FIND_SHA1_HASH => FindPara::Hash(HashAlg::Sha1, blob(para)),
        store::CERT_FIND_MD5_HASH => FindPara::Hash(HashAlg::Md5, blob(para)),
        store::CERT_FIND_SHA256_HASH => FindPara::Hash(HashAlg::Sha256, blob(para)),
        store::CERT_FIND_SUBJECT_NAME => FindPara::Name { issuer: false, encoded: blob(para) },
        store::CERT_FIND_ISSUER_NAME => FindPara::Name { issuer: true, encoded: blob(para) },
        store::CERT_FIND_SUBJECT_STR_W => FindPara::Str { issuer: false, text: wide()? },
        store::CERT_FIND_ISSUER_STR_W => FindPara::Str { issuer: true, text: wide()? },
        store::CERT_FIND_SUBJECT_STR_A => FindPara::Str { issuer: false, text: ansi()? },
        store::CERT_FIND_ISSUER_STR_A => FindPara::Str { issuer: true, text: ansi()? },
        store::CERT_FIND_PUBLIC_KEY => {
            // CERT_PUBLIC_KEY_INFO: Algorithm.pszObjId at 0, PublicKey (bit blob) at 24.
            // SAFETY: the caller's structure.
            let oid = unsafe { rt::ansi_str(para.cast::<*const u8>().read_unaligned()) }?;
            FindPara::PublicKey { oid, key: blob(para.wrapping_add(24)) }
        }
        store::CERT_FIND_EXISTING => {
            // SAFETY: a context pointer; only read.
            let c = unsafe { ctx(para as usize) }?;
            FindPara::Existing(c.cert.encoded.clone())
        }
        store::CERT_FIND_ISSUER_OF => {
            // SAFETY: as above.
            let c = unsafe { ctx(para as usize) }?;
            FindPara::IssuerOf(c.cert.encoded.clone())
        }
        store::CERT_FIND_SUBJECT_CERT => {
            // CERT_INFO: SerialNumber at 8, Issuer at 48.
            FindPara::SubjectCert { issuer: blob(para.wrapping_add(48)), serial_le: blob(para.wrapping_add(8)) }
        }
        store::CERT_FIND_KEY_IDENTIFIER => FindPara::KeyId(blob(para)),
        store::CERT_FIND_ENHKEY_USAGE => {
            // CERT_ENHKEY_USAGE: cUsageIdentifier, rgpszUsageIdentifier at 8.
            // SAFETY: the caller's structure and strings.
            let usages = unsafe {
                let n = para.cast::<u32>().read_unaligned() as usize;
                let arr = para.add(8).cast::<*const *const u8>().read_unaligned();
                (0..n).filter_map(|i| rt::ansi_str(arr.add(i).read_unaligned())).collect()
            };
            FindPara::Usage(usages)
        }
        _ => return None,
    })
}

/// `CertFindCertificateInStore`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertFindCertificateInStore(
    h: usize,
    _encoding: u32,
    _find_flags: u32,
    find_type: u32,
    para: *const u8,
    prev: usize,
) -> usize {
    // SAFETY: only checked.
    if unsafe { store_obj(h) }.is_none() {
        return fail(ERROR_INVALID_HANDLE);
    }
    // SAFETY: the caller's find parameter for this type.
    let Some(p) = (unsafe { find_para(find_type, para) }) else {
        if prev != 0 {
            ctx_release(prev);
        }
        return fail(E_INVALIDARG);
    };
    next_in_store(h, prev, &|c| store::find_matches(c, &p))
}

/// Add a parsed certificate to a store with a disposition; the store's
/// context (with a new reference for the caller when `out` is given).
fn add_to_store(h: usize, cert: Certificate, disposition: u32, out: *mut usize) -> Bool {
    // SAFETY: only read.
    let Some(s) = (unsafe { store_obj(h) }) else { return fail(ERROR_INVALID_HANDLE) };
    if s.readonly {
        return fail(ERROR_ACCESS_DENIED);
    }
    let existing = LOCK.with(|_| {
        s.certs.iter().copied().find(|&c| {
            // SAFETY: under the lock.
            unsafe { ctx(c) }.is_some_and(|x| x.cert.encoded == cert.encoded)
        })
    });
    let newer = existing.is_some_and(|e| {
        // SAFETY: a store context.
        unsafe { ctx(e) }.is_some_and(|x| cert.not_before > x.cert.not_before)
    });
    let action = match store::add_action(disposition, existing.is_some(), newer) {
        Ok(a) => a,
        Err(e) => return fail(e),
    };
    let result = match (action, existing) {
        (AddAction::UseExisting, Some(e)) => e,
        (AddAction::Replace { inherit }, Some(e)) => {
            // SAFETY: a store context.
            let friendly = if inherit { unsafe { ctx(e) }.and_then(|x| x.friendly.clone()) } else { None };
            let n = new_ctx(cert, h);
            // SAFETY: just made.
            if let Some(x) = unsafe { ctx(n) } {
                x.friendly = friendly;
            }
            LOCK.with(|_| {
                if let Some(i) = s.certs.iter().position(|&c| c == e) {
                    s.certs[i] = n;
                }
            });
            ctx_release(e);
            n
        }
        _ => {
            if let Some(d) = &s.dir {
                rt::create_dirs(d);
                let path = alloc::format!("{d}\\{}", store::cert_file_name(&cert));
                if !rt::write_file(&path, &cert.encoded) {
                    return fail(ERROR_ACCESS_DENIED);
                }
            }
            let n = new_ctx(cert, h);
            LOCK.with(|_| s.certs.push(n));
            n
        }
    };
    if !out.is_null() {
        // SAFETY: the caller's slot.
        unsafe { out.write(ctx_ref(result)) };
    }
    TRUE
}

/// `CertAddEncodedCertificateToStore`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertAddEncodedCertificateToStore(
    h: usize,
    encoding: u32,
    data: *const u8,
    len: u32,
    disposition: u32,
    out: *mut usize,
) -> Bool {
    if encoding & X509_ASN_ENCODING == 0 {
        return fail(E_INVALIDARG);
    }
    // SAFETY: the caller's encoding.
    match Certificate::parse(unsafe { rt::bytes(data, len as usize) }) {
        Ok(c) => add_to_store(h, c, disposition, out),
        Err(e) => fail(decode::asn1_error(e)),
    }
}

/// `CertAddCertificateContextToStore`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertAddCertificateContextToStore(
    h: usize,
    context: usize,
    disposition: u32,
    out: *mut usize,
) -> Bool {
    // SAFETY: only read.
    let Some(c) = (unsafe { ctx(context) }) else { return fail(E_INVALIDARG) };
    let cert = c.cert.clone();
    add_to_store(h, cert, disposition, out)
}

/// `CertDeleteCertificateFromStore`: removed from its store (and its file
/// for a system store); the context passed is freed.
#[unsafe(no_mangle)]
pub extern "system" fn CertDeleteCertificateFromStore(context: usize) -> Bool {
    // SAFETY: only read.
    let Some(c) = (unsafe { ctx(context) }) else { return fail(E_INVALIDARG) };
    let h = c.store;
    // SAFETY: only read.
    if let Some(s) = unsafe { store_obj(h) } {
        if s.readonly {
            ctx_release(context);
            return fail(ERROR_ACCESS_DENIED);
        }
        let held = LOCK.with(|_| match s.certs.iter().position(|&x| x == context) {
            Some(i) => {
                s.certs.remove(i);
                true
            }
            None => false,
        });
        if let Some(d) = &s.dir {
            rt::delete_file(&alloc::format!("{d}\\{}", store::cert_file_name(&c.cert)));
        }
        if held {
            ctx_release(context);
        }
    }
    ctx_release(context);
    TRUE
}

/// `CertCreateCertificateContext`: a context in no store (`hCertStore`
/// NULL; to be confirmed).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertCreateCertificateContext(encoding: u32, data: *const u8, len: u32) -> usize {
    if encoding & X509_ASN_ENCODING == 0 {
        return fail(E_INVALIDARG);
    }
    // SAFETY: the caller's encoding.
    match Certificate::parse(unsafe { rt::bytes(data, len as usize) }) {
        Ok(c) => new_ctx(c, 0),
        Err(e) => fail(decode::asn1_error(e)),
    }
}

/// `CertDuplicateCertificateContext`.
#[unsafe(no_mangle)]
pub extern "system" fn CertDuplicateCertificateContext(context: usize) -> usize {
    if CONTEXTS.contains(context) { ctx_ref(context) } else { 0 }
}

/// `CertFreeCertificateContext` (always TRUE, Learn).
#[unsafe(no_mangle)]
pub extern "system" fn CertFreeCertificateContext(context: usize) -> Bool {
    if context != 0 {
        ctx_release(context);
    }
    TRUE
}

// ----- Properties, names, small helpers -----------------------------------------------------

/// Copy `data` to a `pvData`/`pcbData` pair (Learn's size protocol:
/// NULL = size query, short = `ERROR_MORE_DATA`).
unsafe fn out_bytes(data: &[u8], out: *mut u8, len: *mut u32) -> Bool {
    if len.is_null() {
        return fail(E_INVALIDARG);
    }
    // SAFETY: the caller's DWORD.
    let have = unsafe { len.read() } as usize;
    // SAFETY: as above.
    unsafe { len.write(data.len() as u32) };
    if out.is_null() {
        return TRUE;
    }
    if have < data.len() {
        return fail(ERROR_MORE_DATA);
    }
    // SAFETY: the caller's buffer of `have` bytes.
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), out, data.len()) };
    TRUE
}

/// `CERT_*_PROP_ID` (`wincrypt.h`).
const CERT_SHA1_HASH_PROP_ID: u32 = 3;
const CERT_MD5_HASH_PROP_ID: u32 = 4;
const CERT_FRIENDLY_NAME_PROP_ID: u32 = 11;
const CERT_KEY_IDENTIFIER_PROP_ID: u32 = 20;
const CERT_SUBJECT_PUBLIC_KEY_MD5_HASH_PROP_ID: u32 = 25;
const CERT_SHA256_HASH_PROP_ID: u32 = 107;

/// `CertGetCertificateContextProperty`: the hashes, the key identifier
/// and a friendly name set on the context.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertGetCertificateContextProperty(
    context: usize,
    prop: u32,
    out: *mut u8,
    len: *mut u32,
) -> Bool {
    // SAFETY: only read.
    let Some(c) = (unsafe { ctx(context) }) else { return fail(E_INVALIDARG) };
    let data = match prop {
        CERT_SHA1_HASH_PROP_ID => c.cert.thumbprint(HashAlg::Sha1),
        CERT_MD5_HASH_PROP_ID => c.cert.thumbprint(HashAlg::Md5),
        CERT_SHA256_HASH_PROP_ID => c.cert.thumbprint(HashAlg::Sha256),
        CERT_SUBJECT_PUBLIC_KEY_MD5_HASH_PROP_ID => poly_crypto::digest(HashAlg::Md5, &c.cert.public_key_info.key),
        CERT_KEY_IDENTIFIER_PROP_ID => {
            c.cert.subject_key_id().unwrap_or_else(|| poly_crypto::digest(HashAlg::Sha1, &c.cert.public_key_info.key))
        }
        CERT_FRIENDLY_NAME_PROP_ID => match &c.friendly {
            Some(w) => w.iter().chain([&0]).flat_map(|u| u.to_le_bytes()).collect(),
            None => return fail(CRYPT_E_NOT_FOUND),
        },
        _ => return fail(CRYPT_E_NOT_FOUND),
    };
    // SAFETY: the caller's buffer and DWORD.
    unsafe { out_bytes(&data, out, len) }
}

/// `CertSetCertificateContextProperty`: the friendly name (a
/// `CRYPT_DATA_BLOB` of UTF-16; NULL removes it), kept on the context only.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertSetCertificateContextProperty(
    context: usize,
    prop: u32,
    _flags: u32,
    data: *const u8,
) -> Bool {
    // SAFETY: only this thread changes it.
    let Some(c) = (unsafe { ctx(context) }) else { return fail(E_INVALIDARG) };
    if prop != CERT_FRIENDLY_NAME_PROP_ID {
        return fail(E_INVALIDARG);
    }
    if data.is_null() {
        c.friendly = None;
        return TRUE;
    }
    // SAFETY: a CRYPT_DATA_BLOB of UTF-16 text.
    let bytes = unsafe {
        rt::bytes(data.add(8).cast::<*const u8>().read_unaligned(), data.cast::<u32>().read_unaligned() as usize)
    };
    let units: Vec<u16> =
        bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).take_while(|&u| u != 0).collect();
    c.friendly = Some(units);
    TRUE
}

/// Write a name string the `CertGetNameString`/`CertNameToStr` way: the
/// count of characters including the NUL; NULL or 0 = size query; a
/// short buffer gets a truncated, terminated string.
unsafe fn out_wstr(s: &str, out: *mut u16, cch: u32) -> u32 {
    let w: Vec<u16> = s.encode_utf16().collect();
    if out.is_null() || cch == 0 {
        return w.len() as u32 + 1;
    }
    let n = w.len().min(cch as usize - 1);
    // SAFETY: the caller's buffer of `cch` characters.
    unsafe {
        core::ptr::copy_nonoverlapping(w.as_ptr(), out, n);
        out.add(n).write(0);
    }
    n as u32 + 1
}

unsafe fn out_astr(s: &str, out: *mut u8, cch: u32) -> u32 {
    let a: Vec<u8> = s.chars().map(|c| if (c as u32) < 256 { c as u32 as u8 } else { b'?' }).collect();
    if out.is_null() || cch == 0 {
        return a.len() as u32 + 1;
    }
    let n = a.len().min(cch as usize - 1);
    // SAFETY: the caller's buffer of `cch` characters.
    unsafe {
        core::ptr::copy_nonoverlapping(a.as_ptr(), out, n);
        out.add(n).write(0);
    }
    n as u32 + 1
}

unsafe fn name_string(context: usize, kind: u32, flags: u32, para: *const u8) -> Option<String> {
    // SAFETY: only read.
    let c = unsafe { ctx(context) }?;
    let p = match kind {
        // SAFETY: a DWORD.
        certstr::CERT_NAME_RDN_TYPE if !para.is_null() => {
            NamePara::StrType(unsafe { para.cast::<u32>().read_unaligned() })
        }
        // SAFETY: a NUL-terminated OID.
        certstr::CERT_NAME_ATTR_TYPE => unsafe { rt::ansi_str(para) }.map_or(NamePara::None, NamePara::Oid),
        _ => NamePara::None,
    };
    let friendly = c.friendly.as_ref().and_then(|w| String::from_utf16(w).ok());
    Some(certstr::get_name_string(&c.cert, kind, flags, &p, friendly.as_deref()))
}

/// `CertGetNameStringW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertGetNameStringW(
    context: usize,
    kind: u32,
    flags: u32,
    para: *const u8,
    out: *mut u16,
    cch: u32,
) -> u32 {
    // SAFETY: the caller's arguments.
    let s = unsafe { name_string(context, kind, flags, para) }.unwrap_or_default();
    // SAFETY: the caller's buffer.
    unsafe { out_wstr(&s, out, cch) }
}

/// `CertGetNameStringA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertGetNameStringA(
    context: usize,
    kind: u32,
    flags: u32,
    para: *const u8,
    out: *mut u8,
    cch: u32,
) -> u32 {
    // SAFETY: the caller's arguments.
    let s = unsafe { name_string(context, kind, flags, para) }.unwrap_or_default();
    // SAFETY: the caller's buffer.
    unsafe { out_astr(&s, out, cch) }
}

/// `CertNameToStrW` of a `CERT_NAME_BLOB`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertNameToStrW(
    _encoding: u32,
    name: *const u8,
    str_type: u32,
    out: *mut u16,
    cch: u32,
) -> u32 {
    if name.is_null() {
        // SAFETY: the caller's buffer.
        return unsafe { out_wstr("", out, cch) };
    }
    // SAFETY: a CRYPT_DATA_BLOB.
    let enc = unsafe {
        rt::bytes(name.add(8).cast::<*const u8>().read_unaligned(), name.cast::<u32>().read_unaligned() as usize)
    };
    let s = poly_x509::name::Name::parse(enc).map(|n| certstr::name_to_str(&n, str_type)).unwrap_or_default();
    // SAFETY: the caller's buffer.
    unsafe { out_wstr(&s, out, cch) }
}

/// `CertGetPublicKeyLength` of a `CERT_PUBLIC_KEY_INFO` (0 for kinds Poly
/// does not know).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertGetPublicKeyLength(_encoding: u32, info: *const u8) -> u32 {
    if info.is_null() {
        return fail(E_INVALIDARG);
    }
    let blob = |p: *const u8| -> Vec<u8> {
        // SAFETY: a blob in the caller's structure.
        unsafe {
            rt::bytes(p.add(8).cast::<*const u8>().read_unaligned(), p.cast::<u32>().read_unaligned() as usize).to_vec()
        }
    };
    // SAFETY: the caller's structure.
    let Some(oid) = (unsafe { rt::ansi_str(info.cast::<*const u8>().read_unaligned()) }) else { return 0 };
    let k = poly_x509::cert::PublicKeyInfo {
        encoded: Vec::new(),
        algorithm: poly_x509::cert::AlgorithmId { oid, params: blob(info.wrapping_add(8)) },
        unused_bits: 0,
        key: blob(info.wrapping_add(24)),
    };
    k.public_key().map_or(0, |p| p.bits() as u32)
}

/// `CertVerifyTimeValidity`: -1 before `NotBefore`, 1 after `NotAfter`,
/// else 0 (NULL time: now).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertVerifyTimeValidity(time: *const u64, info: *const u8) -> i32 {
    // SAFETY: the caller's FILETIME, or now.
    let t = if time.is_null() { rt::now() } else { unsafe { time.read_unaligned() } };
    // SAFETY: CERT_INFO's NotBefore at 64, NotAfter at 72.
    let (nb, na) =
        unsafe { (info.add(64).cast::<u64>().read_unaligned(), info.add(72).cast::<u64>().read_unaligned()) };
    if t < nb {
        -1
    } else if t > na {
        1
    } else {
        0
    }
}

// ----- CryptDecodeObject(Ex) ----------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
unsafe fn decode_common(
    encoding: u32,
    struct_type: *const u8,
    data: *const u8,
    len: u32,
    flags: u32,
    alloc_fn: Option<unsafe extern "system" fn(usize) -> *mut u8>,
    out: *mut u8,
    out_len: *mut u32,
) -> Bool {
    if encoding & X509_ASN_ENCODING == 0 {
        return fail(E_INVALIDARG);
    }
    let t = struct_type as usize;
    let t = if t < 0x1_0000 {
        t
    } else {
        // SAFETY: a NUL-terminated OID.
        match unsafe { rt::ansi_str(struct_type) }.as_deref().and_then(decode::struct_type_by_oid) {
            Some(t) => t,
            None => return fail(ERROR_FILE_NOT_FOUND),
        }
    };
    // SAFETY: the caller's encoding.
    let f = match decode::decode(t, unsafe { rt::bytes(data, len as usize) }, flags) {
        Ok(f) => f,
        Err(e) => return fail(e),
    };
    if out_len.is_null() {
        return fail(E_INVALIDARG);
    }
    let size = f.len();
    if flags & decode::CRYPT_DECODE_ALLOC_FLAG != 0 {
        if out.is_null() {
            return fail(E_INVALIDARG);
        }
        // SAFETY: the caller's allocator, or LocalAlloc.
        let p = match alloc_fn {
            Some(a) => unsafe { a(size.max(1)) },
            None => rt::local_alloc(size),
        };
        if p.is_null() {
            return fail(ERROR_NOT_ENOUGH_MEMORY);
        }
        let bytes = f.finish(p as usize);
        // SAFETY: a fresh block of `size` bytes; `out` receives its address.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, size);
            out.cast::<*mut u8>().write_unaligned(p);
            out_len.write(size as u32);
        }
        return TRUE;
    }
    // SAFETY: the caller's DWORD.
    let have = unsafe { out_len.read() } as usize;
    // SAFETY: as above.
    unsafe { out_len.write(size as u32) };
    if out.is_null() {
        return TRUE;
    }
    if have < size {
        return fail(ERROR_MORE_DATA);
    }
    let bytes = f.finish(out as usize);
    // SAFETY: the caller's buffer of at least `size` bytes.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), out, size) };
    TRUE
}

/// `CryptDecodeObjectEx`: the structure types of `poly_wincrypt::decode`;
/// an unknown type is `ERROR_FILE_NOT_FOUND` (no OID function, to be
/// confirmed). `CRYPT_DECODE_ALLOC_FLAG` uses `pDecodePara`'s allocator
/// (`CRYPT_DECODE_PARA`: `cbSize`, `pfnAlloc` at 8) or `LocalAlloc`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CryptDecodeObjectEx(
    encoding: u32,
    struct_type: *const u8,
    data: *const u8,
    len: u32,
    flags: u32,
    para: *const u8,
    out: *mut u8,
    out_len: *mut u32,
) -> Bool {
    let alloc_fn = if para.is_null() {
        None
    } else {
        // SAFETY: CRYPT_DECODE_PARA's pfnAlloc (NULL or a function).
        unsafe { para.add(8).cast::<Option<unsafe extern "system" fn(usize) -> *mut u8>>().read_unaligned() }
    };
    // SAFETY: the caller's arguments.
    unsafe { decode_common(encoding, struct_type, data, len, flags, alloc_fn, out, out_len) }
}

/// `CryptDecodeObject`: as `CryptDecodeObjectEx` without allocation.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CryptDecodeObject(
    encoding: u32,
    struct_type: *const u8,
    data: *const u8,
    len: u32,
    flags: u32,
    out: *mut u8,
    out_len: *mut u32,
) -> Bool {
    // SAFETY: the caller's arguments.
    unsafe {
        decode_common(encoding, struct_type, data, len, flags & !decode::CRYPT_DECODE_ALLOC_FLAG, None, out, out_len)
    }
}

// ----- String conversions -------------------------------------------------------------------

/// The `CryptBinaryToString` size protocol: NULL = the size with the NUL;
/// otherwise `*pcch` is the buffer size in, the characters without the NUL
/// out, and a short buffer is `ERROR_MORE_DATA`.
unsafe fn out_text<T: Copy>(units: &[T], zero: T, out: *mut T, cch: *mut u32) -> Bool {
    if cch.is_null() {
        return fail(E_INVALIDARG);
    }
    if out.is_null() {
        // SAFETY: the caller's DWORD.
        unsafe { cch.write(units.len() as u32 + 1) };
        return TRUE;
    }
    // SAFETY: as above.
    let have = unsafe { cch.read() } as usize;
    if have < units.len() + 1 {
        // SAFETY: as above.
        unsafe { cch.write(units.len() as u32 + 1) };
        return fail(ERROR_MORE_DATA);
    }
    // SAFETY: the caller's buffer of `have` units.
    unsafe {
        core::ptr::copy_nonoverlapping(units.as_ptr(), out, units.len());
        out.add(units.len()).write(zero);
        cch.write(units.len() as u32);
    }
    TRUE
}

/// `CryptBinaryToStringW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CryptBinaryToStringW(
    data: *const u8,
    len: u32,
    flags: u32,
    out: *mut u16,
    cch: *mut u32,
) -> Bool {
    // SAFETY: the caller's bytes.
    match certstr::binary_to_string(unsafe { rt::bytes(data, len as usize) }, flags) {
        Ok(s) => {
            let w: Vec<u16> = s.encode_utf16().collect();
            // SAFETY: the caller's buffer and count.
            unsafe { out_text(&w, 0, out, cch) }
        }
        Err(e) => fail(e),
    }
}

/// `CryptBinaryToStringA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CryptBinaryToStringA(
    data: *const u8,
    len: u32,
    flags: u32,
    out: *mut u8,
    cch: *mut u32,
) -> Bool {
    // SAFETY: the caller's bytes.
    match certstr::binary_to_string(unsafe { rt::bytes(data, len as usize) }, flags) {
        // SAFETY: the caller's buffer and count.
        Ok(s) => unsafe { out_text(s.as_bytes(), 0, out, cch) },
        Err(e) => fail(e),
    }
}

unsafe fn string_to_binary(s: String, flags: u32, out: *mut u8, len: *mut u32, skip: *mut u32, used: *mut u32) -> Bool {
    let d = match certstr::string_to_binary(&s, flags) {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    // SAFETY: the caller's optional DWORDs.
    unsafe {
        if !skip.is_null() {
            skip.write(d.skip as u32);
        }
        if !used.is_null() {
            used.write(d.format);
        }
        out_bytes(&d.data, out, len)
    }
}

/// `CryptStringToBinaryW` (`cchString` 0: NUL-terminated).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CryptStringToBinaryW(
    text: *const u16,
    cch: u32,
    flags: u32,
    out: *mut u8,
    len: *mut u32,
    skip: *mut u32,
    used: *mut u32,
) -> Bool {
    if text.is_null() {
        return fail(E_INVALIDARG);
    }
    // SAFETY: the caller's string of `cch` characters, or NUL-terminated.
    let n = if cch == 0 { unsafe { rt::wcslen(text) } } else { cch as usize };
    // SAFETY: as above.
    let s = String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(text, n) });
    // SAFETY: the caller's outputs.
    unsafe { string_to_binary(s, flags, out, len, skip, used) }
}

/// `CryptStringToBinaryA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CryptStringToBinaryA(
    text: *const u8,
    cch: u32,
    flags: u32,
    out: *mut u8,
    len: *mut u32,
    skip: *mut u32,
    used: *mut u32,
) -> Bool {
    if text.is_null() {
        return fail(E_INVALIDARG);
    }
    let s = if cch == 0 {
        // SAFETY: NUL-terminated.
        unsafe { rt::ansi_str(text) }.unwrap_or_default()
    } else {
        // SAFETY: `cch` characters.
        unsafe { rt::bytes(text, cch as usize) }.iter().map(|&b| char::from(b)).collect()
    };
    // SAFETY: the caller's outputs.
    unsafe { string_to_binary(s, flags, out, len, skip, used) }
}

// ----- Chains -------------------------------------------------------------------------------

/// A chain context: the block programs read and what Poly needs to free
/// and judge it.
struct ChainBox {
    block: Vec<u8>,
    contexts: Vec<usize>,
    errors: Vec<u32>,
    end: usize,
    refs: u32,
}

static CHAINS: SpinLock<Vec<usize>> = SpinLock::new(Vec::new());

fn chain_box<'a>(p: usize) -> Option<&'a mut ChainBox> {
    CHAINS
        .with(|l| {
            l.iter().copied().find(|&b| {
                // SAFETY: a live Box<ChainBox>.
                unsafe { &*(b as *const ChainBox) }.block.as_ptr() as usize == p
            })
        })
        // SAFETY: a live Box<ChainBox>.
        .map(|b| unsafe { &mut *(b as *mut ChainBox) })
}

const CERT_CHAIN_REVOCATION_CHECK_END_CERT: u32 = 0x1000_0000;
const CERT_CHAIN_REVOCATION_CHECK_CHAIN: u32 = 0x2000_0000;
const CERT_CHAIN_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT: u32 = 0x4000_0000;
/// `HCCE_LOCAL_MACHINE`.
const HCCE_LOCAL_MACHINE: usize = 1;

/// The certificates of a store handle (copies).
fn store_certs(h: usize) -> Vec<Certificate> {
    LOCK.with(|_| {
        // SAFETY: under the lock.
        let Some(s) = (unsafe { store_obj(h) }) else { return Vec::new() };
        s.certs
            .iter()
            .filter_map(|&c| {
                // SAFETY: a context the store holds.
                unsafe { ctx(c) }.map(|x| x.cert.clone())
            })
            .collect()
    })
}

/// `CertGetCertificateChain`: one simple chain built from the end
/// certificate's store, `hAdditionalStore`, the `CA` stores and the `ROOT`
/// stores (the trusted ones) of the engine's location, checked at
/// `pTime` (NULL: now) for `pChainPara`'s requested usages (all of them,
/// whatever the match type; to be confirmed). Revocation is never
/// checked: a request for it marks the elements
/// `CERT_TRUST_REVOCATION_STATUS_UNKNOWN | CERT_TRUST_IS_OFFLINE_REVOCATION`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertGetCertificateChain(
    engine: usize,
    context: usize,
    time: *const u64,
    additional: usize,
    para: *const u8,
    flags: u32,
    _reserved: *const u8,
    out: *mut usize,
) -> Bool {
    // SAFETY: only read.
    let Some(end) = (unsafe { ctx(context) }) else { return fail(E_INVALIDARG) };
    if out.is_null() {
        return fail(E_INVALIDARG);
    }
    // SAFETY: the caller's FILETIME, or now.
    let now = if time.is_null() { rt::now() } else { unsafe { time.read_unaligned() } };
    let usages: Vec<String> = if para.is_null() {
        Vec::new()
    } else {
        // SAFETY: CERT_CHAIN_PARA: RequestedUsage.Usage at 8 (count) and 16 (array).
        unsafe {
            let n = para.add(8).cast::<u32>().read_unaligned() as usize;
            let arr = para.add(16).cast::<*const *const u8>().read_unaligned();
            if arr.is_null() {
                Vec::new()
            } else {
                (0..n).filter_map(|i| rt::ansi_str(arr.add(i).read_unaligned())).collect()
            }
        }
    };
    let location = if engine == HCCE_LOCAL_MACHINE { Location::LocalMachine } else { Location::CurrentUser };
    let mut pool: Vec<Certificate> = Vec::new();
    let mut trusted: Vec<bool> = Vec::new();
    let add = |certs: Vec<Certificate>, t: bool, pool: &mut Vec<Certificate>, trusted: &mut Vec<bool>| {
        for c in certs {
            match pool.iter().position(|p| p.encoded == c.encoded) {
                Some(i) => trusted[i] |= t,
                None => {
                    pool.push(c);
                    trusted.push(t);
                }
            }
        }
    };
    add(load_system(location, "ROOT").0, true, &mut pool, &mut trusted);
    if end.store != 0 {
        add(store_certs(end.store), false, &mut pool, &mut trusted);
    }
    if additional != 0 {
        add(store_certs(additional), false, &mut pool, &mut trusted);
    }
    add(load_system(location, "CA").0, false, &mut pool, &mut trusted);
    let refs: Vec<&Certificate> = pool.iter().collect();
    let built =
        chain::build(&chain::Request { end: &end.cert, pool: &refs, trusted: &trusted, time: now, usages: &usages });
    let mut errors: Vec<u32> = built.elements.iter().map(|e| e.error).collect();
    let infos: Vec<u32> = built.elements.iter().map(|e| e.info).collect();
    let n = errors.len();
    let rev = flags
        & (CERT_CHAIN_REVOCATION_CHECK_END_CERT
            | CERT_CHAIN_REVOCATION_CHECK_CHAIN
            | CERT_CHAIN_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT);
    if rev != 0 {
        let upto = match rev {
            CERT_CHAIN_REVOCATION_CHECK_END_CERT => 1,
            CERT_CHAIN_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT => n.saturating_sub(1).max(1),
            _ => n,
        };
        for e in errors.iter_mut().take(upto) {
            *e |= trust::REVOCATION_STATUS_UNKNOWN | trust::IS_OFFLINE_REVOCATION;
        }
    }
    let chain_error = errors.iter().fold(0, |a, e| a | e);
    // The contexts of the elements (the end's own, new ones for the pool's).
    let contexts: Vec<usize> = built
        .elements
        .iter()
        .map(|e| match e.pool_index {
            None => ctx_ref(context),
            Some(i) => new_ctx(pool[i].clone(), 0),
        })
        .collect();
    // CERT_CHAIN_CONTEXT (72) -> rgpChain[1] -> CERT_SIMPLE_CHAIN (40) ->
    // rgpElement[n] -> CERT_CHAIN_ELEMENT (56 each).
    let mut f = Flat::new(72);
    f.put_u32(0, 72);
    f.put_u32(4, chain_error);
    f.put_u32(8, built.info);
    f.put_u32(12, 1);
    let rgp = f.reserve(8);
    f.put_ptr(16, rgp);
    let simple = f.reserve(40);
    f.put_ptr(rgp, simple);
    f.put_u32(simple, 40);
    f.put_u32(simple + 4, chain_error);
    f.put_u32(simple + 8, built.info);
    f.put_u32(simple + 12, n as u32);
    let elems = f.reserve(8 * n);
    f.put_ptr(simple + 16, elems);
    for i in 0..n {
        let el = f.reserve(56);
        f.put_ptr(elems + 8 * i, el);
        f.put_u32(el, 56);
        f.put_u64(el + 8, contexts[i] as u64);
        f.put_u32(el + 16, errors[i]);
        f.put_u32(el + 20, infos[i]);
    }
    let mut block = alloc::vec![0u8; f.len()];
    let base = block.as_ptr() as usize;
    block.copy_from_slice(&f.finish(base));
    let b = Box::into_raw(Box::new(ChainBox { block, contexts, errors, end: context, refs: 1 })) as usize;
    CHAINS.with(|l| l.push(b));
    // SAFETY: the caller's slot.
    unsafe { out.write(base) };
    TRUE
}

/// `CertDuplicateCertificateChain`.
#[unsafe(no_mangle)]
pub extern "system" fn CertDuplicateCertificateChain(chain_ctx: usize) -> usize {
    match chain_box(chain_ctx) {
        Some(b) => {
            CHAINS.with(|_| b.refs += 1);
            chain_ctx
        }
        None => 0,
    }
}

/// `CertFreeCertificateChain`.
#[unsafe(no_mangle)]
pub extern "system" fn CertFreeCertificateChain(chain_ctx: usize) {
    let found = CHAINS.with(|l| {
        let i = l.iter().position(|&b| {
            // SAFETY: a live Box<ChainBox>.
            unsafe { &*(b as *const ChainBox) }.block.as_ptr() as usize == chain_ctx
        })?;
        // SAFETY: as above, under the lock.
        let cb = unsafe { &mut *(l[i] as *mut ChainBox) };
        cb.refs -= 1;
        if cb.refs == 0 { Some(l.swap_remove(i)) } else { None }
    });
    if let Some(b) = found {
        // SAFETY: unregistered: ours alone now.
        let cb = unsafe { Box::from_raw(b as *mut ChainBox) };
        for c in &cb.contexts {
            ctx_release(*c);
        }
    }
}

/// `CertVerifyCertificateChainPolicy`: the base, SSL (with
/// `HTTPSPolicyCallbackData`'s server name and ignore flags) and basic
/// constraints policies; others (Authenticode, NT auth ...) are
/// `CRYPT_E_NOT_FOUND` and FALSE (to be confirmed).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CertVerifyCertificateChainPolicy(
    policy: *const u8,
    chain_ctx: usize,
    para: *const u8,
    status: *mut u8,
) -> Bool {
    let policy = policy as usize;
    if !matches!(
        policy,
        store::CERT_CHAIN_POLICY_BASE | store::CERT_CHAIN_POLICY_SSL | store::CERT_CHAIN_POLICY_BASIC_CONSTRAINTS
    ) {
        return fail(CRYPT_E_NOT_FOUND);
    }
    let Some(b) = chain_box(chain_ctx) else { return fail(E_INVALIDARG) };
    if status.is_null() {
        return fail(E_INVALIDARG);
    }
    // SAFETY: only read.
    let Some(end) = (unsafe { ctx(b.end) }) else { return fail(E_INVALIDARG) };
    let (flags, extra) = if para.is_null() {
        (0, core::ptr::null())
    } else {
        // SAFETY: CERT_CHAIN_POLICY_PARA: dwFlags at 4, pvExtraPolicyPara at 8.
        unsafe { (para.add(4).cast::<u32>().read_unaligned(), para.add(8).cast::<*const u8>().read_unaligned()) }
    };
    let ssl = if policy == store::CERT_CHAIN_POLICY_SSL && !extra.is_null() {
        // SAFETY: HTTPSPolicyCallbackData: fdwChecks at 8, pwszServerName at 16.
        unsafe {
            Some(store::SslPara {
                checks: extra.add(8).cast::<u32>().read_unaligned(),
                server_name: rt::wide_str(extra.add(16).cast::<*const u16>().read_unaligned()),
            })
        }
    } else {
        None
    };
    let s = store::verify_policy(policy, &b.errors, &end.cert, flags, ssl.as_ref());
    // SAFETY: CERT_CHAIN_POLICY_STATUS: dwError at 4, lChainIndex at 8, lElementIndex at 12.
    unsafe {
        status.add(4).cast::<u32>().write_unaligned(s.error);
        status.add(8).cast::<i32>().write_unaligned(s.chain_index);
        status.add(12).cast::<i32>().write_unaligned(s.element_index);
    }
    TRUE
}

// ----- Data protection ----------------------------------------------------------------------

/// The master key of the user (or the machine): made on first use.
fn master_key(machine: bool) -> Option<Vec<u8>> {
    let root = location_root(if machine { Location::LocalMachine } else { Location::CurrentUser })?;
    let dir = alloc::format!("{root}\\Poly\\Protect");
    let path = alloc::format!("{dir}\\masterkey");
    if let Some(k) = rt::read_file(&path)
        && k.len() == 32
    {
        return Some(k);
    }
    let mut k = alloc::vec![0u8; 32];
    if !rt::random(&mut k) {
        return None;
    }
    rt::create_dirs(&dir);
    if !rt::write_file(&path, &k) {
        return None;
    }
    Some(k)
}

/// A `DATA_BLOB` (`cbData` at 0, `pbData` at 8).
unsafe fn data_blob<'a>(p: *const u8) -> &'a [u8] {
    if p.is_null() {
        return &[];
    }
    // SAFETY: the caller's DATA_BLOB.
    unsafe { rt::bytes(p.add(8).cast::<*const u8>().read_unaligned(), p.cast::<u32>().read_unaligned() as usize) }
}

/// Fill an output `DATA_BLOB` with a `LocalAlloc`ed copy.
unsafe fn set_blob(p: *mut u8, data: &[u8]) -> bool {
    let m = rt::local_copy(data);
    if m.is_null() {
        return false;
    }
    // SAFETY: the caller's DATA_BLOB.
    unsafe {
        p.cast::<u32>().write_unaligned(data.len() as u32);
        p.add(8).cast::<*mut u8>().write_unaligned(m);
    }
    true
}

/// `CryptProtectData` (Poly's format, `poly_wincrypt::dpapi`). No prompt
/// is ever shown.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CryptProtectData(
    input: *const u8,
    description: *const u16,
    entropy: *const u8,
    _reserved: *const u8,
    _prompt: *const u8,
    flags: u32,
    out: *mut u8,
) -> Bool {
    if input.is_null() || out.is_null() {
        return fail(E_INVALIDARG);
    }
    let machine = flags & poly_wincrypt::dpapi::CRYPTPROTECT_LOCAL_MACHINE != 0;
    let Some(master) = master_key(machine) else { return fail(ERROR_ACCESS_DENIED) };
    let mut r = [0u8; 28];
    if !rt::random(&mut r) {
        return fail(NTE_BAD_DATA);
    }
    let desc: Vec<u16> = if description.is_null() {
        Vec::new()
    } else {
        // SAFETY: a NUL-terminated string.
        unsafe { core::slice::from_raw_parts(description, rt::wcslen(description)) }.to_vec()
    };
    // SAFETY: the caller's blobs.
    let blob =
        unsafe { poly_wincrypt::dpapi::protect(&master, data_blob(input), &desc, data_blob(entropy), machine, &r) };
    // SAFETY: the caller's output blob.
    if unsafe { set_blob(out, &blob) } { TRUE } else { fail(ERROR_NOT_ENOUGH_MEMORY) }
}

/// `CryptUnprotectData`: the description is returned `LocalAlloc`ed.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CryptUnprotectData(
    input: *const u8,
    description: *mut *mut u16,
    entropy: *const u8,
    _reserved: *const u8,
    _prompt: *const u8,
    _flags: u32,
    out: *mut u8,
) -> Bool {
    if input.is_null() || out.is_null() {
        return fail(E_INVALIDARG);
    }
    // SAFETY: the caller's blob.
    let blob = unsafe { data_blob(input) };
    let Some(master) = master_key(poly_wincrypt::dpapi::is_machine_blob(blob)) else {
        return fail(ERROR_ACCESS_DENIED);
    };
    // SAFETY: the caller's entropy blob.
    let (data, desc) = match poly_wincrypt::dpapi::unprotect(&master, blob, unsafe { data_blob(entropy) }) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    if !description.is_null() {
        let bytes: Vec<u8> = desc.iter().chain([&0]).flat_map(|u| u.to_le_bytes()).collect();
        // SAFETY: the caller's slot.
        unsafe { description.write(rt::local_copy(&bytes).cast()) };
    }
    // SAFETY: the caller's output blob.
    if unsafe { set_blob(out, &data) } { TRUE } else { fail(ERROR_NOT_ENOUGH_MEMORY) }
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn crypt32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
