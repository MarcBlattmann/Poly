//! `CryptProtectData` / `CryptUnprotectData` (Learn; `dpapi.h`) in a
//! Poly-local format, honestly weaker than Windows' DPAPI.
//!
//! Windows derives the protection key from the user's logon secret. Poly
//! has one user and no logon secret (ADR 0066, 0083), so the key comes
//! from a **master key file**: 32 random bytes the DLL creates on first
//! use, under the user's profile (`%APPDATA%\Poly\Protect\masterkey`) or,
//! with `CRYPTPROTECT_LOCAL_MACHINE`, under `%ProgramData%`. A blob is
//! useless without that file, so a blob copied to another machine or
//! profile does not open; but anyone who can read the profile can read
//! the key. That is the whole promise (ADR 0096).
//!
//! The blob: the 8 bytes Windows' blobs start with (`01 00 00 00 d0 8c 9d df`, the provider version
//! and the start of the provider GUID; the sheets of `CryptProtectData` observe them), flags (u32
//! LE: bit 0 = machine key), a 16-byte salt, a 12-byte nonce, the description's length in bytes (u32 LE), the
//! description (UTF-16LE), the ciphertext and a 16-byte tag. The key is
//! HMAC-SHA-256(master key, `"Poly DPAPI v1"` || salt || optional
//! entropy); AES-256-GCM encrypts the data with everything before the
//! ciphertext as additional data, so the description and flags are
//! authenticated too.

use crate::status::*;
use alloc::vec::Vec;
use poly_crypto::HashAlg;
use poly_crypto::aes::Aes;
use poly_crypto::gcm::Gcm;

/// `CRYPTPROTECT_*` flags (`dpapi.h`).
pub const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
pub const CRYPTPROTECT_LOCAL_MACHINE: u32 = 0x4;
pub const CRYPTPROTECT_AUDIT: u32 = 0x10;
pub const CRYPTPROTECT_VERIFY_PROTECTION: u32 = 0x40;

const MAGIC: &[u8; 8] = &[0x01, 0x00, 0x00, 0x00, 0xD0, 0x8C, 0x9D, 0xDF];
/// The magic of the first version of the format (Poly's own, 4 bytes): still opened, no longer made.
const OLD_MAGIC: &[u8; 4] = b"PDP1";
const LABEL: &[u8] = b"Poly DPAPI v1";
const HEADER: usize = 8 + 4 + 16 + 12 + 4;
const OLD_HEADER: usize = 4 + 4 + 16 + 12 + 4;

fn key(master: &[u8], salt: &[u8], entropy: &[u8]) -> Gcm {
    let mut m = poly_crypto::Hmac::new(HashAlg::Sha256, master);
    m.update(LABEL);
    m.update(salt);
    m.update(entropy);
    let k = m.finish();
    // HMAC-SHA-256 gives 32 bytes: always an AES-256 key.
    Gcm::new(Aes::new(&k).expect("a 32-byte AES key"))
}

/// Whether a blob was made with the machine key (to pick the master key
/// before opening it).
pub fn is_machine_blob(blob: &[u8]) -> bool {
    match layout(blob) {
        Some(base) => blob[base - 4] & 1 != 0,
        None => false,
    }
}

/// The offset of the flags' end (where the salt starts) when `blob` starts like one of ours and is
/// long enough for its fixed part and the tag, else `None`.
fn layout(blob: &[u8]) -> Option<usize> {
    if blob.len() >= HEADER + 16 && &blob[..8] == MAGIC {
        Some(12)
    } else if blob.len() >= OLD_HEADER + 16 && &blob[..4] == OLD_MAGIC {
        Some(8)
    } else {
        None
    }
}

/// Protect `data`: `random` must give 28 fresh bytes (salt and nonce).
pub fn protect(
    master: &[u8],
    data: &[u8],
    description: &[u16],
    entropy: &[u8],
    machine: bool,
    random: &[u8; 28],
) -> Vec<u8> {
    let mut blob = Vec::with_capacity(HEADER + description.len() * 2 + data.len() + 16);
    blob.extend_from_slice(MAGIC);
    blob.extend_from_slice(&u32::from(machine).to_le_bytes());
    blob.extend_from_slice(random);
    blob.extend_from_slice(&((description.len() * 2) as u32).to_le_bytes());
    for c in description {
        blob.extend_from_slice(&c.to_le_bytes());
    }
    let aad_len = blob.len();
    let mut ct = data.to_vec();
    let mut tag = [0u8; 16];
    key(master, &random[..16], entropy).seal(&random[16..], &blob[..aad_len], &mut ct, &mut tag);
    blob.extend_from_slice(&ct);
    blob.extend_from_slice(&tag);
    blob
}

/// Open a blob: the data and the description. Something that is not shaped like a blob at all (plain
/// data, an 8-byte prefix, nothing) is `ERROR_INVALID_PARAMETER`; a blob that is damaged, or was
/// made with another master key or entropy, is `ERROR_INVALID_DATA` (observed on Windows).
pub fn unprotect(master: &[u8], blob: &[u8], entropy: &[u8]) -> Result<(Vec<u8>, Vec<u16>), u32> {
    let Some(base) = layout(blob) else { return Err(ERROR_INVALID_PARAMETER) };
    let header = base + 16 + 12 + 4;
    let len_at = header - 4;
    let dlen = u32::from_le_bytes([blob[len_at], blob[len_at + 1], blob[len_at + 2], blob[len_at + 3]]) as usize;
    let aad_len = header.checked_add(dlen).ok_or(ERROR_INVALID_DATA)?;
    if !dlen.is_multiple_of(2) || blob.len() < aad_len + 16 {
        return Err(ERROR_INVALID_DATA);
    }
    let (salt, nonce) = (&blob[base..base + 16], &blob[base + 16..base + 28]);
    let mut data = blob[aad_len..blob.len() - 16].to_vec();
    let tag = &blob[blob.len() - 16..];
    if !key(master, salt, entropy).open(nonce, &blob[..aad_len], &mut data, tag) {
        return Err(ERROR_INVALID_DATA);
    }
    let desc = blob[header..aad_len].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Ok((data, desc))
}

// ----- CryptProtectMemory (ADR 0155) ------------------------------------------------------

/// `CRYPTPROTECTMEMORY_BLOCK_SIZE`: the data's size must be a multiple of it (`dpapi.h`).
pub const CRYPTPROTECTMEMORY_BLOCK_SIZE: usize = 16;
/// `CRYPTPROTECTMEMORY_SAME_PROCESS`, `_CROSS_PROCESS`, `_SAME_LOGON`.
pub const CRYPTPROTECTMEMORY_SAME_PROCESS: u32 = 0;
pub const CRYPTPROTECTMEMORY_CROSS_PROCESS: u32 = 1;
pub const CRYPTPROTECTMEMORY_SAME_LOGON: u32 = 2;
/// Flag 4 is accepted by the protecting call and can never be undone (observed on Windows:
/// `ERROR_ACCESS_DENIED` from the unprotecting call).
pub const CRYPTPROTECTMEMORY_UNDOABLE: u32 = 4;

const MEMORY_LABEL: &[u8] = b"Poly CryptProtectMemory v1";

/// `CryptProtectMemory` / `CryptUnprotectMemory` (Learn): encrypt or decrypt `data` in place. The
/// key comes from `secret` and the flag: for `SAME_PROCESS` the secret is the process's own random
/// key (only this process can decrypt), for `CROSS_PROCESS` and `SAME_LOGON` it is the user's master
/// key (any process of the user can). AES-256 in CBC mode without padding, so the size stays the
/// same; the IV is derived with the key (the data is memory protected against being read from a
/// dump, not stored). No data, a size that is not a multiple of 8, or a flag Learn does not list
/// is `ERROR_INVALID_PARAMETER`; flag 4 protects but cannot be undone.
pub fn protect_memory(secret: &[u8], flags: u32, data: &mut [u8], encrypt: bool) -> Result<(), u32> {
    if (flags > CRYPTPROTECTMEMORY_SAME_LOGON && flags != CRYPTPROTECTMEMORY_UNDOABLE)
        || data.is_empty()
        || !data.len().is_multiple_of(CRYPTPROTECTMEMORY_BLOCK_SIZE / 2)
    {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if flags == CRYPTPROTECTMEMORY_UNDOABLE && !encrypt {
        return Err(ERROR_ACCESS_DENIED);
    }
    let derive = |extra: &[u8]| {
        let mut m = poly_crypto::Hmac::new(HashAlg::Sha256, secret);
        m.update(MEMORY_LABEL);
        m.update(&[flags as u8]);
        m.update(extra);
        m.finish()
    };
    let aes = Aes::new(&derive(b"key")).expect("a 32-byte AES key");
    let mut iv = [0u8; 16];
    iv.copy_from_slice(&derive(b"iv")[..16]);
    // A size of 8 mod 16 is accepted too (observed on Windows: 8 bytes protect and unprotect): the
    // last 8 bytes are XORed with a key stream block taken from the last cipher text block (or the IV).
    let head = data.len() - data.len() % 16;
    let (body, tail) = data.split_at_mut(head);
    let stream = |last: [u8; 16]| {
        let mut b = last;
        aes.encrypt_block(&mut b);
        b
    };
    if encrypt {
        if !poly_crypto::modes::cbc_encrypt(&aes, &mut iv, body) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let ks = stream(if body.is_empty() { iv } else { body[head - 16..].try_into().expect("16 bytes") });
        tail.iter_mut().zip(ks).for_each(|(t, k)| *t ^= k);
    } else {
        let ks = stream(if body.is_empty() { iv } else { body[head - 16..].try_into().expect("16 bytes") });
        tail.iter_mut().zip(ks).for_each(|(t, k)| *t ^= k);
        if !poly_crypto::modes::cbc_decrypt(&aes, &mut iv, body) {
            return Err(ERROR_INVALID_PARAMETER);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_tampering() {
        let master = [9u8; 32];
        let desc: Vec<u16> = "Mail password".encode_utf16().collect();
        let blob = protect(&master, b"hunter2", &desc, b"extra", false, &[3; 28]);
        assert!(!is_machine_blob(&blob));
        let (data, d) = unprotect(&master, &blob, b"extra").unwrap();
        assert_eq!((data.as_slice(), d), (&b"hunter2"[..], desc));
        assert_eq!(unprotect(&master, &blob, b""), Err(ERROR_INVALID_DATA));
        assert_eq!(unprotect(&[8u8; 32], &blob, b"extra"), Err(ERROR_INVALID_DATA));
        let mut bad = blob.clone();
        bad[49] ^= 1; // the description is authenticated
        assert_eq!(unprotect(&master, &bad, b"extra"), Err(ERROR_INVALID_DATA));
        assert_eq!(unprotect(&master, &blob[..20], b"extra"), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(unprotect(&master, &blob[..blob.len() - 1], b"extra"), Err(ERROR_INVALID_DATA));
        assert_eq!(unprotect(&master, &[7u8; 32], b""), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(unprotect(&master, &[], b""), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(blob[..8], [1, 0, 0, 0, 0xD0, 0x8C, 0x9D, 0xDF]);
        let m = protect(&master, b"", &[], b"", true, &[4; 28]);
        assert!(is_machine_blob(&m));
        assert_eq!(unprotect(&master, &m, b"").unwrap().0, b"");
    }

    #[test]
    fn memory_protection_round_trips_in_place_and_depends_on_key_and_flag() {
        let secret = [5u8; 32];
        let plain = *b"0123456789abcdef0123456789abcdef";
        let mut d = plain;
        protect_memory(&secret, CRYPTPROTECTMEMORY_SAME_PROCESS, &mut d, true).unwrap();
        assert_ne!(d, plain);
        // Same blocks of plain text do not give the same cipher text (CBC).
        assert_ne!(d[..16], d[16..]);
        let enc = d;
        protect_memory(&secret, CRYPTPROTECTMEMORY_SAME_PROCESS, &mut d, false).unwrap();
        assert_eq!(d, plain);
        // Another flag or another secret gives other bytes, and does not decrypt.
        let mut e = plain;
        protect_memory(&secret, CRYPTPROTECTMEMORY_SAME_LOGON, &mut e, true).unwrap();
        assert_ne!(e, enc);
        let mut f = enc;
        protect_memory(&[6u8; 32], CRYPTPROTECTMEMORY_SAME_PROCESS, &mut f, false).unwrap();
        assert_ne!(f, plain);
    }

    #[test]
    fn memory_protection_checks_size_and_flags() {
        let mut d = [0u8; 20];
        assert_eq!(protect_memory(&[1; 32], 0, &mut d[..17], true), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(protect_memory(&[1; 32], 0, &mut d[..15], true), Err(ERROR_INVALID_PARAMETER));
        let mut e = *b"12345678";
        protect_memory(&[1; 32], 0, &mut e, true).unwrap();
        assert_ne!(&e, b"12345678");
        protect_memory(&[1; 32], 0, &mut e, false).unwrap();
        assert_eq!(&e, b"12345678");
        let mut f = *b"0123456789abcdef01234567";
        protect_memory(&[1; 32], 0, &mut f, true).unwrap();
        protect_memory(&[1; 32], 0, &mut f, false).unwrap();
        assert_eq!(&f, b"0123456789abcdef01234567");
        assert_eq!(protect_memory(&[1; 32], 0, &mut d[..16], true), Ok(()));
        assert_eq!(protect_memory(&[1; 32], 3, &mut d[..16], true), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(protect_memory(&[1; 32], 1, &mut [], false), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(protect_memory(&[1; 32], 4, &mut d[..16], true), Ok(()));
        assert_eq!(protect_memory(&[1; 32], 4, &mut d[..16], false), Err(ERROR_ACCESS_DENIED));
        assert_eq!(protect_memory(&[1; 32], 0x10, &mut d[..16], true), Err(ERROR_INVALID_PARAMETER));
    }
}
