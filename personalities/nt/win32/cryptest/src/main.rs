//! `cryptest.exe` (ADR 0096): the program the kernel's ring-3 test
//! `nt_crypto_program_hashes_encrypts_and_verifies_a_chain` runs. It
//! imports from `bcrypt.dll`, `crypt32.dll` and `kernel32.dll` and checks
//! hashing and HMAC (FIPS 180-4, RFC 1321, RFC 4231 answers), the system
//! RNG, AES-GCM (the GCM specification's test case 3) and CBC, PBKDF2
//! (RFC 6070), ECDSA and ECDH, certificate decoding and name strings,
//! memory and system stores, chain building to a root added to `ROOT`
//! with the base and SSL policies, and `CryptProtectData`.
//!
//! It exits through `ExitProcess(PASS)` when every check passed; a failed
//! check prints `cryptest: step N failed (got G)` and exits with
//! `0x80 + N`.

#![no_std]
#![no_main]
#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;

type Handle = usize;
type NtStatus = u32;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ExitProcess(code: u32) -> !;
    fn GetLastError() -> u32;
    fn GetStdHandle(which: u32) -> isize;
    fn WriteFile(h: isize, buf: *const u8, len: u32, written: *mut u32, ov: *mut u8) -> i32;
    fn LocalFree(p: *mut u8) -> *mut u8;
}

#[link(name = "bcrypt", kind = "raw-dylib")]
unsafe extern "system" {
    fn BCryptOpenAlgorithmProvider(ph: *mut Handle, id: *const u16, imp: *const u16, flags: u32) -> NtStatus;
    fn BCryptCloseAlgorithmProvider(h: Handle, flags: u32) -> NtStatus;
    fn BCryptGetProperty(h: Handle, p: *const u16, out: *mut u8, cb: u32, res: *mut u32, flags: u32) -> NtStatus;
    fn BCryptSetProperty(h: Handle, p: *const u16, input: *const u8, cb: u32, flags: u32) -> NtStatus;
    fn BCryptCreateHash(h: Handle, ph: *mut Handle, obj: *mut u8, cbo: u32, s: *const u8, cbs: u32, f: u32)
    -> NtStatus;
    fn BCryptHashData(h: Handle, input: *const u8, cb: u32, flags: u32) -> NtStatus;
    fn BCryptFinishHash(h: Handle, out: *mut u8, cb: u32, flags: u32) -> NtStatus;
    fn BCryptDuplicateHash(h: Handle, ph: *mut Handle, obj: *mut u8, cbo: u32, flags: u32) -> NtStatus;
    fn BCryptDestroyHash(h: Handle) -> NtStatus;
    fn BCryptHash(h: Handle, s: *const u8, cbs: u32, i: *const u8, cbi: u32, o: *mut u8, cbo: u32) -> NtStatus;
    fn BCryptGenRandom(h: Handle, buf: *mut u8, cb: u32, flags: u32) -> NtStatus;
    fn BCryptGenerateSymmetricKey(
        h: Handle,
        ph: *mut Handle,
        obj: *mut u8,
        cbo: u32,
        s: *const u8,
        cbs: u32,
        f: u32,
    ) -> NtStatus;
    fn BCryptEncrypt(
        k: Handle,
        i: *const u8,
        cbi: u32,
        pad: *mut u8,
        iv: *mut u8,
        cbiv: u32,
        o: *mut u8,
        cbo: u32,
        res: *mut u32,
        f: u32,
    ) -> NtStatus;
    fn BCryptDecrypt(
        k: Handle,
        i: *const u8,
        cbi: u32,
        pad: *mut u8,
        iv: *mut u8,
        cbiv: u32,
        o: *mut u8,
        cbo: u32,
        res: *mut u32,
        f: u32,
    ) -> NtStatus;
    fn BCryptDestroyKey(k: Handle) -> NtStatus;
    fn BCryptDeriveKeyPBKDF2(
        prf: Handle,
        pw: *const u8,
        cbpw: u32,
        salt: *const u8,
        cbsalt: u32,
        it: u64,
        out: *mut u8,
        cbo: u32,
        f: u32,
    ) -> NtStatus;
    fn BCryptGenerateKeyPair(h: Handle, ph: *mut Handle, bits: u32, f: u32) -> NtStatus;
    fn BCryptFinalizeKeyPair(k: Handle, f: u32) -> NtStatus;
    fn BCryptExportKey(k: Handle, ek: Handle, t: *const u16, o: *mut u8, cbo: u32, res: *mut u32, f: u32) -> NtStatus;
    fn BCryptImportKeyPair(
        h: Handle,
        ik: Handle,
        t: *const u16,
        ph: *mut Handle,
        i: *const u8,
        cbi: u32,
        f: u32,
    ) -> NtStatus;
    fn BCryptSignHash(
        k: Handle,
        pad: *const u8,
        i: *const u8,
        cbi: u32,
        o: *mut u8,
        cbo: u32,
        res: *mut u32,
        f: u32,
    ) -> NtStatus;
    fn BCryptVerifySignature(
        k: Handle,
        pad: *const u8,
        h: *const u8,
        cbh: u32,
        s: *const u8,
        cbs: u32,
        f: u32,
    ) -> NtStatus;
    fn BCryptSecretAgreement(a: Handle, b: Handle, ph: *mut Handle, f: u32) -> NtStatus;
    fn BCryptDeriveKey(
        s: Handle,
        kdf: *const u16,
        p: *const u8,
        o: *mut u8,
        cbo: u32,
        res: *mut u32,
        f: u32,
    ) -> NtStatus;
    fn BCryptDestroySecret(s: Handle) -> NtStatus;
}

#[link(name = "crypt32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CryptStringToBinaryA(
        s: *const u8,
        cch: u32,
        flags: u32,
        out: *mut u8,
        len: *mut u32,
        skip: *mut u32,
        used: *mut u32,
    ) -> i32;
    fn CryptBinaryToStringA(data: *const u8, len: u32, flags: u32, out: *mut u8, cch: *mut u32) -> i32;
    fn CertCreateCertificateContext(enc: u32, data: *const u8, len: u32) -> *const u8;
    fn CertFreeCertificateContext(c: *const u8) -> i32;
    fn CertGetNameStringW(c: *const u8, kind: u32, flags: u32, para: *const u8, out: *mut u16, cch: u32) -> u32;
    fn CryptDecodeObjectEx(
        enc: u32,
        t: *const u8,
        data: *const u8,
        len: u32,
        flags: u32,
        para: *const u8,
        out: *mut u8,
        out_len: *mut u32,
    ) -> i32;
    fn CertOpenStore(prov: *const u8, enc: u32, hprov: usize, flags: u32, para: *const u8) -> Handle;
    fn CertOpenSystemStoreW(hprov: usize, name: *const u16) -> Handle;
    fn CertCloseStore(h: Handle, flags: u32) -> i32;
    fn CertAddEncodedCertificateToStore(
        h: Handle,
        enc: u32,
        d: *const u8,
        len: u32,
        disp: u32,
        out: *mut *const u8,
    ) -> i32;
    fn CertEnumCertificatesInStore(h: Handle, prev: *const u8) -> *const u8;
    fn CertFindCertificateInStore(h: Handle, enc: u32, ff: u32, t: u32, para: *const u8, prev: *const u8) -> *const u8;
    fn CertGetCertificateContextProperty(c: *const u8, prop: u32, out: *mut u8, len: *mut u32) -> i32;
    fn CertGetCertificateChain(
        engine: usize,
        c: *const u8,
        time: *const u64,
        store: Handle,
        para: *const u8,
        flags: u32,
        r: *const u8,
        out: *mut *const u8,
    ) -> i32;
    fn CertVerifyCertificateChainPolicy(policy: *const u8, chain: *const u8, para: *const u8, status: *mut u8) -> i32;
    fn CertFreeCertificateChain(chain: *const u8);
    fn CryptProtectData(
        i: *const u8,
        desc: *const u16,
        ent: *const u8,
        r: *const u8,
        p: *const u8,
        f: u32,
        o: *mut u8,
    ) -> i32;
    fn CryptUnprotectData(
        i: *const u8,
        desc: *mut *mut u16,
        ent: *const u8,
        r: *const u8,
        p: *const u8,
        f: u32,
        o: *mut u8,
    ) -> i32;
}

const PASS: u32 = 0x43;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;

const STATUS_SUCCESS: u32 = 0;
const STATUS_INVALID_HANDLE: u32 = 0xC000_0008;
const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;
const STATUS_BUFFER_TOO_SMALL: u32 = 0xC000_0023;
const STATUS_NOT_FOUND: u32 = 0xC000_0225;
const STATUS_INVALID_SIGNATURE: u32 = 0xC000_A000;
const STATUS_AUTH_TAG_MISMATCH: u32 = 0xC000_A002;
const X509_ASN_ENCODING: u32 = 1;
const CERT_E_CHAINING: u32 = 0x800B_010A;
const CERT_E_CN_NO_MATCH: u32 = 0x800B_010F;
const CRYPT_E_NOT_FOUND: u32 = 0x8009_2004;
const ERROR_INVALID_DATA: u32 = 13;

/// A NUL-terminated UTF-16 copy of an ASCII literal.
macro_rules! w {
    ($s:literal) => {{
        const S: &[u8] = $s.as_bytes();
        const W: [u16; S.len() + 1] = {
            let mut o = [0u16; S.len() + 1];
            let mut i = 0;
            while i < S.len() {
                o[i] = S[i] as u16;
                i += 1;
            }
            o
        };
        W.as_ptr()
    }};
}

macro_rules! a {
    ($s:literal) => {
        concat!($s, "\0").as_ptr()
    };
}

fn out(bytes: &[u8]) {
    // SAFETY: kernel32 calls with valid buffers.
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h != 0 && h != -1 {
            let mut n = 0;
            WriteFile(h, bytes.as_ptr(), bytes.len() as u32, &mut n, core::ptr::null_mut());
        }
    }
}

fn out_num(mut v: u32) {
    let mut b = [0u8; 10];
    let mut i = b.len();
    loop {
        i -= 1;
        b[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    out(&b[i..]);
}

fn out_hex(v: u32) {
    let mut b = *b"0x00000000";
    for i in 0..8 {
        let d = (v >> (28 - 4 * i)) & 15;
        b[2 + i] = if d < 10 { b'0' + d as u8 } else { b'a' + d as u8 - 10 };
    }
    out(&b);
}

static STEP: AtomicU32 = AtomicU32::new(0);

/// Check `ok` as the next step, reporting `got` on failure.
fn check_got(ok: bool, got: u32) {
    let step = STEP.fetch_add(1, Ordering::Relaxed) + 1;
    if !ok {
        out(b"cryptest: step ");
        out_num(step);
        out(b" failed (got ");
        out_hex(got);
        out(b")\n");
        // SAFETY: never returns.
        unsafe { ExitProcess(0x80 + (step & 0x7F)) }
    }
}

/// Check an `NTSTATUS`.
fn st(got: NtStatus, want: NtStatus) {
    check_got(got == want, got);
}

fn check(ok: bool) {
    // SAFETY: a plain kernel32 call.
    let e = unsafe { GetLastError() };
    check_got(ok, e);
}

fn check_fail(r: i32, want: u32) {
    // SAFETY: a plain kernel32 call.
    let e = unsafe { GetLastError() };
    check_got(r == 0 && e == want, e);
}

fn hex<const N: usize>(s: &[u8]) -> [u8; N] {
    let d = |c: u8| if c.is_ascii_digit() { c - b'0' } else { (c | 0x20) - b'a' + 10 };
    let mut o = [0u8; N];
    for (i, x) in o.iter_mut().enumerate() {
        *x = (d(s[2 * i]) << 4) | d(s[2 * i + 1]);
    }
    o
}

fn u32_at(p: *const u8, off: usize) -> u32 {
    // SAFETY: callers pass structures that hold a DWORD at `off`.
    unsafe { p.add(off).cast::<u32>().read_unaligned() }
}

fn ptr_at(p: *const u8, off: usize) -> *const u8 {
    // SAFETY: callers pass structures that hold a pointer at `off`.
    unsafe { p.add(off).cast::<*const u8>().read_unaligned() }
}

fn wide_eq(p: *const u16, s: &[u8]) -> bool {
    // SAFETY: NUL-terminated strings of at least `s.len()` + 1 units, or shorter ones that end first.
    (0..=s.len()).all(|i| unsafe { *p.add(i) } == if i < s.len() { u16::from(s[i]) } else { 0 })
}

const ROOT_PEM: &str = include_str!("../../../../../crates/poly-x509/src/testdata/root.pem");
const INT_PEM: &str = include_str!("../../../../../crates/poly-x509/src/testdata/int.pem");
const LEAF_PEM: &str = include_str!("../../../../../crates/poly-x509/src/testdata/leaf.pem");

#[unsafe(no_mangle)]
pub extern "system" fn cryptest_main(_peb: *mut u8) -> u32 {
    out(b"cryptest: start\n");
    // SAFETY: every call is a bcrypt/crypt32/kernel32 export with valid arguments.
    unsafe {
        hashing();
        random();
        aes();
        asymmetric();
        certificates();
        protect();
        out(b"cryptest: all ");
        out_num(STEP.load(Ordering::Relaxed));
        out(b" checks passed\n");
        ExitProcess(PASS)
    }
}

unsafe fn hashing() {
    let abc256 = hex::<32>(b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut alg = 0;
        st(BCryptOpenAlgorithmProvider(&mut alg, w!("SHA256"), core::ptr::null(), 0), STATUS_SUCCESS);
        let (mut v, mut n) = (0u32, 0u32);
        st(BCryptGetProperty(alg, w!("HashDigestLength"), (&raw mut v).cast(), 4, &mut n, 0), STATUS_SUCCESS);
        check_got(v == 32 && n == 4, v);
        st(BCryptGetProperty(alg, w!("ObjectLength"), core::ptr::null_mut(), 0, &mut n, 0), STATUS_SUCCESS);
        check_got(n == 4, n);
        st(BCryptGetProperty(alg, w!("HashDigestLength"), (&raw mut v).cast(), 2, &mut n, 0), STATUS_BUFFER_TOO_SMALL);
        let mut h = 0;
        st(BCryptCreateHash(alg, &mut h, core::ptr::null_mut(), 0, core::ptr::null(), 0, 0), STATUS_SUCCESS);
        st(BCryptHashData(h, b"a".as_ptr(), 1, 0), STATUS_SUCCESS);
        let mut dup = 0;
        st(BCryptDuplicateHash(h, &mut dup, core::ptr::null_mut(), 0, 0), STATUS_SUCCESS);
        st(BCryptHashData(h, b"bc".as_ptr(), 2, 0), STATUS_SUCCESS);
        let mut d = [0u8; 32];
        st(BCryptFinishHash(h, d.as_mut_ptr(), 31, 0), STATUS_INVALID_PARAMETER);
        st(BCryptFinishHash(h, d.as_mut_ptr(), 32, 0), STATUS_SUCCESS);
        check(d == abc256);
        st(BCryptHashData(dup, b"bc".as_ptr(), 2, 0), STATUS_SUCCESS);
        let mut d2 = [0u8; 32];
        st(BCryptFinishHash(dup, d2.as_mut_ptr(), 32, 0), STATUS_SUCCESS);
        check(d2 == abc256);
        st(BCryptDestroyHash(h), STATUS_SUCCESS);
        st(BCryptDestroyHash(dup), STATUS_SUCCESS);
        st(BCryptDestroyHash(h), STATUS_INVALID_HANDLE);
        st(BCryptCloseAlgorithmProvider(alg, 0), STATUS_SUCCESS);
        st(BCryptCloseAlgorithmProvider(alg, 0), STATUS_INVALID_HANDLE);
        // The pseudo-handles (BCRYPT_SHA1_ALG_HANDLE, BCRYPT_MD5_ALG_HANDLE).
        let mut s1 = [0u8; 20];
        st(BCryptHash(0x31, core::ptr::null(), 0, b"abc".as_ptr(), 3, s1.as_mut_ptr(), 20), STATUS_SUCCESS);
        check(s1 == hex::<20>(b"a9993e364706816aba3e25717850c26c9cd0d89d"));
        let mut m5 = [0u8; 16];
        st(BCryptHash(0x21, core::ptr::null(), 0, b"abc".as_ptr(), 3, m5.as_mut_ptr(), 16), STATUS_SUCCESS);
        check(m5 == hex::<16>(b"900150983cd24fb0d6963f7d28e17f72"));
        // HMAC-SHA-256, RFC 4231 test case 2.
        let mut mac = 0;
        st(BCryptOpenAlgorithmProvider(&mut mac, w!("SHA256"), core::ptr::null(), 8), STATUS_SUCCESS);
        let mut hm = 0;
        st(BCryptCreateHash(mac, &mut hm, core::ptr::null_mut(), 0, b"Jefe".as_ptr(), 4, 0), STATUS_SUCCESS);
        let msg = b"what do ya want for nothing?";
        st(BCryptHashData(hm, msg.as_ptr(), msg.len() as u32, 0), STATUS_SUCCESS);
        st(BCryptFinishHash(hm, d.as_mut_ptr(), 32, 0), STATUS_SUCCESS);
        check(d == hex::<32>(b"5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"));
        st(BCryptDestroyHash(hm), STATUS_SUCCESS);
        st(BCryptCloseAlgorithmProvider(mac, 0), STATUS_SUCCESS);
        let mut nope = 0;
        st(BCryptOpenAlgorithmProvider(&mut nope, w!("NOPE"), core::ptr::null(), 0), STATUS_NOT_FOUND);
    }
}

unsafe fn random() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
        st(BCryptGenRandom(0, a.as_mut_ptr(), 32, 2), STATUS_SUCCESS);
        st(BCryptGenRandom(0, b.as_mut_ptr(), 32, 2), STATUS_SUCCESS);
        check(a != [0; 32] && a != b);
        st(BCryptGenRandom(0, a.as_mut_ptr(), 32, 0), STATUS_INVALID_HANDLE);
        let mut rng = 0;
        st(BCryptOpenAlgorithmProvider(&mut rng, w!("RNG"), core::ptr::null(), 0), STATUS_SUCCESS);
        st(BCryptGenRandom(rng, a.as_mut_ptr(), 32, 0), STATUS_SUCCESS);
        st(BCryptGenRandom(rng, a.as_mut_ptr(), 32, 2), STATUS_INVALID_PARAMETER);
        st(BCryptGenRandom(0x81, b.as_mut_ptr(), 32, 0), STATUS_SUCCESS);
        st(BCryptCloseAlgorithmProvider(rng, 0), STATUS_SUCCESS);
    }
}

/// `BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO` (x64, 88 bytes).
#[repr(C)]
struct AuthInfo {
    cb_size: u32,
    version: u32,
    nonce: *const u8,
    cb_nonce: u32,
    aad: *const u8,
    cb_aad: u32,
    tag: *mut u8,
    cb_tag: u32,
    mac_context: *mut u8,
    cb_mac_context: u32,
    cb_aad_done: u32,
    cb_data: u64,
    flags: u32,
}

unsafe fn aes() {
    // The GCM specification's test case 3.
    let key = hex::<16>(b"feffe9928665731c6d6a8f9467308308");
    let iv = hex::<12>(b"cafebabefacedbaddecaf888");
    let pt = hex::<64>(
        b"d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255",
    );
    let ct = hex::<64>(
        b"42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091473f5985",
    );
    let want_tag = hex::<16>(b"4d5c2af327cd64a62cf35abd2ba6fab4");
    // SAFETY: valid arguments throughout.
    unsafe {
        check_got(core::mem::size_of::<AuthInfo>() == 88, core::mem::size_of::<AuthInfo>() as u32);
        let mut alg = 0;
        st(BCryptOpenAlgorithmProvider(&mut alg, w!("AES"), core::ptr::null(), 0), STATUS_SUCCESS);
        let gcm = w!("ChainingModeGCM");
        st(BCryptSetProperty(alg, w!("ChainingMode"), gcm.cast(), 32, 0), STATUS_SUCCESS);
        let mut lens = [0u32; 3];
        let mut n = 0;
        st(BCryptGetProperty(alg, w!("AuthTagLength"), lens.as_mut_ptr().cast(), 12, &mut n, 0), STATUS_SUCCESS);
        check(lens == [12, 16, 1]);
        let mut k = 0;
        st(BCryptGenerateSymmetricKey(alg, &mut k, core::ptr::null_mut(), 0, key.as_ptr(), 16, 0), STATUS_SUCCESS);
        let mut tag = [0u8; 16];
        let mut info = AuthInfo {
            cb_size: 88,
            version: 1,
            nonce: iv.as_ptr(),
            cb_nonce: 12,
            aad: core::ptr::null(),
            cb_aad: 0,
            tag: tag.as_mut_ptr(),
            cb_tag: 16,
            mac_context: core::ptr::null_mut(),
            cb_mac_context: 0,
            cb_aad_done: 0,
            cb_data: 0,
            flags: 0,
        };
        let info_p = (&raw mut info).cast::<u8>();
        let mut buf = [0u8; 64];
        st(
            BCryptEncrypt(k, pt.as_ptr(), 64, info_p, core::ptr::null_mut(), 0, buf.as_mut_ptr(), 64, &mut n, 0),
            STATUS_SUCCESS,
        );
        check(n == 64 && buf == ct && tag == want_tag);
        let mut back = [0u8; 64];
        st(
            BCryptDecrypt(k, buf.as_ptr(), 64, info_p, core::ptr::null_mut(), 0, back.as_mut_ptr(), 64, &mut n, 0),
            STATUS_SUCCESS,
        );
        check(back == pt);
        *info.tag.add(3) ^= 1;
        st(
            BCryptDecrypt(k, buf.as_ptr(), 64, info_p, core::ptr::null_mut(), 0, back.as_mut_ptr(), 64, &mut n, 0),
            STATUS_AUTH_TAG_MISMATCH,
        );
        st(BCryptDestroyKey(k), STATUS_SUCCESS);
        // CBC with PKCS#7 padding: a size query, then a round trip.
        let cbc = w!("ChainingModeCBC");
        st(BCryptSetProperty(alg, w!("ChainingMode"), cbc.cast(), 32, 0), STATUS_SUCCESS);
        st(BCryptGenerateSymmetricKey(alg, &mut k, core::ptr::null_mut(), 0, key.as_ptr(), 16, 0), STATUS_SUCCESS);
        let mut ivb = [7u8; 16];
        let msg = b"seventeen bytes!!";
        st(
            BCryptEncrypt(
                k,
                msg.as_ptr(),
                17,
                core::ptr::null_mut(),
                ivb.as_mut_ptr(),
                16,
                core::ptr::null_mut(),
                0,
                &mut n,
                1,
            ),
            STATUS_SUCCESS,
        );
        check_got(n == 32, n);
        st(
            BCryptEncrypt(
                k,
                msg.as_ptr(),
                17,
                core::ptr::null_mut(),
                ivb.as_mut_ptr(),
                16,
                buf.as_mut_ptr(),
                32,
                &mut n,
                1,
            ),
            STATUS_SUCCESS,
        );
        let mut ivb = [7u8; 16];
        st(
            BCryptDecrypt(
                k,
                buf.as_ptr(),
                32,
                core::ptr::null_mut(),
                ivb.as_mut_ptr(),
                16,
                back.as_mut_ptr(),
                64,
                &mut n,
                1,
            ),
            STATUS_SUCCESS,
        );
        check(n == 17 && &back[..17] == msg);
        st(BCryptDestroyKey(k), STATUS_SUCCESS);
        st(BCryptCloseAlgorithmProvider(alg, 0), STATUS_SUCCESS);
        // PBKDF2-HMAC-SHA1, RFC 6070 test 2.
        let mut prf = 0;
        st(BCryptOpenAlgorithmProvider(&mut prf, w!("SHA1"), core::ptr::null(), 8), STATUS_SUCCESS);
        let mut dk = [0u8; 20];
        st(
            BCryptDeriveKeyPBKDF2(prf, b"password".as_ptr(), 8, b"salt".as_ptr(), 4, 2, dk.as_mut_ptr(), 20, 0),
            STATUS_SUCCESS,
        );
        check(dk == hex::<20>(b"ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957"));
        st(BCryptCloseAlgorithmProvider(prf, 0), STATUS_SUCCESS);
    }
}

unsafe fn asymmetric() {
    // SAFETY: valid arguments throughout.
    unsafe {
        // ECDSA P-256: generate, sign, export the public half, verify with it.
        let mut alg = 0;
        st(BCryptOpenAlgorithmProvider(&mut alg, w!("ECDSA_P256"), core::ptr::null(), 0), STATUS_SUCCESS);
        let mut k = 0;
        st(BCryptGenerateKeyPair(alg, &mut k, 256, 0), STATUS_SUCCESS);
        st(BCryptFinalizeKeyPair(k, 0), STATUS_SUCCESS);
        let hash = [0x5au8; 32];
        let mut n = 0;
        st(
            BCryptSignHash(k, core::ptr::null(), hash.as_ptr(), 32, core::ptr::null_mut(), 0, &mut n, 0),
            STATUS_SUCCESS,
        );
        check_got(n == 64, n);
        let mut sig = [0u8; 64];
        st(BCryptSignHash(k, core::ptr::null(), hash.as_ptr(), 32, sig.as_mut_ptr(), 64, &mut n, 0), STATUS_SUCCESS);
        let mut blob = [0u8; 72];
        st(BCryptExportKey(k, 0, w!("ECCPUBLICBLOB"), blob.as_mut_ptr(), 72, &mut n, 0), STATUS_SUCCESS);
        check(n == 72 && u32_at(blob.as_ptr(), 0) == 0x3153_4345);
        let mut pk = 0;
        st(BCryptImportKeyPair(alg, 0, w!("ECCPUBLICBLOB"), &mut pk, blob.as_ptr(), 72, 0), STATUS_SUCCESS);
        st(BCryptVerifySignature(pk, core::ptr::null(), hash.as_ptr(), 32, sig.as_ptr(), 64, 0), STATUS_SUCCESS);
        sig[10] ^= 1;
        st(
            BCryptVerifySignature(pk, core::ptr::null(), hash.as_ptr(), 32, sig.as_ptr(), 64, 0),
            STATUS_INVALID_SIGNATURE,
        );
        st(BCryptDestroyKey(pk), STATUS_SUCCESS);
        st(BCryptDestroyKey(k), STATUS_SUCCESS);
        st(BCryptCloseAlgorithmProvider(alg, 0), STATUS_SUCCESS);
        // ECDH P-256: both sides agree on the secret.
        st(BCryptOpenAlgorithmProvider(&mut alg, w!("ECDH_P256"), core::ptr::null(), 0), STATUS_SUCCESS);
        let (mut a, mut b) = (0, 0);
        st(BCryptGenerateKeyPair(alg, &mut a, 256, 0), STATUS_SUCCESS);
        st(BCryptFinalizeKeyPair(a, 0), STATUS_SUCCESS);
        st(BCryptGenerateKeyPair(alg, &mut b, 256, 0), STATUS_SUCCESS);
        st(BCryptFinalizeKeyPair(b, 0), STATUS_SUCCESS);
        let (mut s1, mut s2) = (0, 0);
        st(BCryptSecretAgreement(a, b, &mut s1, 0), STATUS_SUCCESS);
        st(BCryptSecretAgreement(b, a, &mut s2, 0), STATUS_SUCCESS);
        let (mut z1, mut z2) = ([0u8; 32], [1u8; 32]);
        st(BCryptDeriveKey(s1, w!("TRUNCATE"), core::ptr::null(), z1.as_mut_ptr(), 32, &mut n, 0), STATUS_SUCCESS);
        st(BCryptDeriveKey(s2, w!("TRUNCATE"), core::ptr::null(), z2.as_mut_ptr(), 32, &mut n, 0), STATUS_SUCCESS);
        check(n == 32 && z1 == z2 && z1 != [0; 32]);
        st(BCryptDestroySecret(s1), STATUS_SUCCESS);
        st(BCryptDestroySecret(s2), STATUS_SUCCESS);
        st(BCryptDestroyKey(a), STATUS_SUCCESS);
        st(BCryptDestroyKey(b), STATUS_SUCCESS);
        st(BCryptCloseAlgorithmProvider(alg, 0), STATUS_SUCCESS);
    }
}

/// A certificate's DER from its PEM, through `CryptStringToBinaryA`.
unsafe fn der(pem: &str, buf: &mut [u8; 2048]) -> u32 {
    let mut len = buf.len() as u32;
    let (mut skip, mut used) = (9, 9);
    // SAFETY: a counted string and our buffer.
    let ok = unsafe {
        CryptStringToBinaryA(pem.as_ptr(), pem.len() as u32, 0, buf.as_mut_ptr(), &mut len, &mut skip, &mut used)
    };
    check(ok != 0 && skip == 0 && used == 0 && len > 300);
    len
}

unsafe fn certificates() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let (mut lb, mut ib, mut rb) = ([0u8; 2048], [0u8; 2048], [0u8; 2048]);
        let ll = der(LEAF_PEM, &mut lb);
        let il = der(INT_PEM, &mut ib);
        let rl = der(ROOT_PEM, &mut rb);
        // Base64 back and forth.
        let mut cch = 0u32;
        check(CryptBinaryToStringA(lb.as_ptr(), ll, 1, core::ptr::null_mut(), &mut cch) != 0);
        let mut text = [0u8; 4096];
        let mut have = text.len() as u32;
        check(CryptBinaryToStringA(lb.as_ptr(), ll, 1, text.as_mut_ptr(), &mut have) != 0);
        check_got(have + 1 == cch, have);
        let mut again = [0u8; 2048];
        let mut al = again.len() as u32;
        check(
            CryptStringToBinaryA(
                text.as_ptr(),
                have,
                1,
                again.as_mut_ptr(),
                &mut al,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            ) != 0,
        );
        check(al == ll && again[..ll as usize] == lb[..ll as usize]);
        // A context, its CERT_INFO and name strings.
        let leaf = CertCreateCertificateContext(X509_ASN_ENCODING, lb.as_ptr(), ll);
        check(!leaf.is_null());
        check_got(u32_at(leaf, 16) == ll, u32_at(leaf, 16));
        let info = ptr_at(leaf, 24);
        check_got(u32_at(info, 0) == 2, u32_at(info, 0));
        let mut name = [0u16; 64];
        let n = CertGetNameStringW(leaf, 4, 0, core::ptr::null(), name.as_mut_ptr(), 64);
        check(n == 17 && wide_eq(name.as_ptr(), b"www.example.test"));
        let n = CertGetNameStringW(leaf, 4, 1, core::ptr::null(), name.as_mut_ptr(), 64);
        check(n == 26 && wide_eq(name.as_ptr(), b"Poly Test Intermediate CA"));
        check(CertGetNameStringW(leaf, 6, 0, core::ptr::null(), core::ptr::null_mut(), 0) == 17);
        // CryptDecodeObjectEx: the subject name, allocated (CERT_NAME_INFO: 5 RDNs).
        let mut p: *mut u8 = core::ptr::null_mut();
        let mut size = 0u32;
        let subject = ptr_at(info, 88);
        let subject_len = u32_at(info, 80);
        check(
            CryptDecodeObjectEx(
                1,
                core::ptr::without_provenance::<u8>(7),
                subject,
                subject_len,
                0x8000,
                core::ptr::null(),
                (&raw mut p).cast(),
                &mut size,
            ) != 0,
        );
        check_got(u32_at(p, 0) == 5, u32_at(p, 0));
        LocalFree(p);
        check_fail(
            CryptDecodeObjectEx(
                1,
                core::ptr::without_provenance::<u8>(7),
                subject,
                3,
                0x8000,
                core::ptr::null(),
                (&raw mut p).cast(),
                &mut size,
            ),
            0x8009_3102,
        );
        // A memory store with the intermediate; a find by subject text.
        let mem = CertOpenStore(core::ptr::without_provenance::<u8>(2), 0, 0, 0, core::ptr::null());
        check(mem != 0);
        check(CertAddEncodedCertificateToStore(mem, X509_ASN_ENCODING, ib.as_ptr(), il, 1, core::ptr::null_mut()) != 0);
        check_fail(
            CertAddEncodedCertificateToStore(mem, X509_ASN_ENCODING, ib.as_ptr(), il, 1, core::ptr::null_mut()),
            0x8009_2005,
        );
        let first = CertEnumCertificatesInStore(mem, core::ptr::null());
        check(!first.is_null());
        check(CertEnumCertificatesInStore(mem, first).is_null() && GetLastError() == CRYPT_E_NOT_FOUND);
        let found = CertFindCertificateInStore(mem, 1, 0, (8 << 16) | 7, w!("intermediate").cast(), core::ptr::null());
        check(!found.is_null());
        CertFreeCertificateContext(found);
        // Without the root trusted (or the intermediate known) the chain is partial.
        let mut chain: *const u8 = core::ptr::null();
        let when = 133_000_000_000_000_000u64; // 2022-06
        let later = when + 10_000_000 * 86_400 * 365 * 5; // 2027
        check(CertGetCertificateChain(0, leaf, &later, 0, core::ptr::null(), 0, core::ptr::null(), &mut chain) != 0);
        let mut status = [0u8; 24];
        check(
            CertVerifyCertificateChainPolicy(
                core::ptr::without_provenance::<u8>(1),
                chain,
                core::ptr::null(),
                status.as_mut_ptr(),
            ) != 0,
        );
        check_got(u32_at(status.as_ptr(), 4) == CERT_E_CHAINING, u32_at(status.as_ptr(), 4));
        CertFreeCertificateChain(chain);
        // The root into the current user's ROOT store (a file), found again after reopening.
        let root_store = CertOpenSystemStoreW(0, w!("ROOT"));
        check(root_store != 0);
        check(
            CertAddEncodedCertificateToStore(root_store, X509_ASN_ENCODING, rb.as_ptr(), rl, 3, core::ptr::null_mut())
                != 0,
        );
        check(CertCloseStore(root_store, 0) != 0);
        let root_store = CertOpenStore(a!("System"), 0, 0, 1 << 16, w!("root").cast());
        check(root_store != 0);
        let mut thumb = [0u8; 20];
        let mut tl = 20u32;
        let rctx = CertCreateCertificateContext(X509_ASN_ENCODING, rb.as_ptr(), rl);
        check(CertGetCertificateContextProperty(rctx, 3, thumb.as_mut_ptr(), &mut tl) != 0 && tl == 20);
        let blob = (20u32, 0u32, thumb.as_ptr());
        let r = CertFindCertificateInStore(root_store, 1, 0, 1 << 16, (&raw const blob).cast(), core::ptr::null());
        check(!r.is_null());
        CertFreeCertificateContext(r);
        CertFreeCertificateContext(rctx);
        CertCloseStore(root_store, 0);
        // Now the chain: leaf -> intermediate (from the extra store) -> trusted root.
        check(CertGetCertificateChain(0, leaf, &later, mem, core::ptr::null(), 0, core::ptr::null(), &mut chain) != 0);
        check_got(u32_at(chain, 4) == 0, u32_at(chain, 4));
        let simple = ptr_at(ptr_at(chain, 16), 0);
        check_got(u32_at(simple, 12) == 3, u32_at(simple, 12));
        check(
            CertVerifyCertificateChainPolicy(
                core::ptr::without_provenance::<u8>(1),
                chain,
                core::ptr::null(),
                status.as_mut_ptr(),
            ) != 0,
        );
        check_got(u32_at(status.as_ptr(), 4) == 0, u32_at(status.as_ptr(), 4));
        // The SSL policy with the right and a wrong server name.
        let mut https = (16u32, 2u32, 0u32, 0u32, w!("www.example.test"));
        let mut para = (24u32, 0u32, (&raw mut https).cast::<u8>());
        check(
            CertVerifyCertificateChainPolicy(
                core::ptr::without_provenance::<u8>(4),
                chain,
                (&raw const para).cast(),
                status.as_mut_ptr(),
            ) != 0,
        );
        check_got(u32_at(status.as_ptr(), 4) == 0, u32_at(status.as_ptr(), 4));
        https.4 = w!("evil.example");
        para.2 = (&raw mut https).cast::<u8>();
        check(
            CertVerifyCertificateChainPolicy(
                core::ptr::without_provenance::<u8>(4),
                chain,
                (&raw const para).cast(),
                status.as_mut_ptr(),
            ) != 0,
        );
        check_got(u32_at(status.as_ptr(), 4) == CERT_E_CN_NO_MATCH, u32_at(status.as_ptr(), 4));
        CertFreeCertificateChain(chain);
        CertFreeCertificateContext(first);
        check(CertCloseStore(mem, 0) != 0);
        check(CertFreeCertificateContext(leaf) != 0);
        let _ = when;
    }
}

unsafe fn protect() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let secret = b"hunter2";
        let input = (7u32, 0u32, secret.as_ptr());
        let entropy = (4u32, 0u32, b"salt".as_ptr());
        let mut blob = (0u32, 0u32, core::ptr::null_mut::<u8>());
        check(
            CryptProtectData(
                (&raw const input).cast(),
                w!("mail"),
                (&raw const entropy).cast(),
                core::ptr::null(),
                core::ptr::null(),
                1,
                (&raw mut blob).cast(),
            ) != 0,
        );
        check(blob.0 > 7 && !blob.2.is_null());
        let mut back = (0u32, 0u32, core::ptr::null_mut::<u8>());
        let mut desc: *mut u16 = core::ptr::null_mut();
        check(
            CryptUnprotectData(
                (&raw const blob).cast(),
                &mut desc,
                (&raw const entropy).cast(),
                core::ptr::null(),
                core::ptr::null(),
                1,
                (&raw mut back).cast(),
            ) != 0,
        );
        check(back.0 == 7 && core::slice::from_raw_parts(back.2, 7) == secret && wide_eq(desc, b"mail"));
        LocalFree(back.2);
        LocalFree(desc.cast());
        check_fail(
            CryptUnprotectData(
                (&raw const blob).cast(),
                core::ptr::null_mut(),
                core::ptr::null(),
                core::ptr::null(),
                core::ptr::null(),
                1,
                (&raw mut back).cast(),
            ),
            ERROR_INVALID_DATA,
        );
        LocalFree(blob.2);
    }
}
