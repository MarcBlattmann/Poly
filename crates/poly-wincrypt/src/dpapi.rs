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
//! The blob: `"PDP1"`, flags (u32 LE: bit 0 = machine key), a 16-byte
//! salt, a 12-byte nonce, the description's length in bytes (u32 LE), the
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

const MAGIC: &[u8; 4] = b"PDP1";
const LABEL: &[u8] = b"Poly DPAPI v1";
const HEADER: usize = 4 + 4 + 16 + 12 + 4;

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
    blob.len() >= HEADER && &blob[..4] == MAGIC && blob[4] & 1 != 0
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

/// Open a blob: the data and the description. `ERROR_INVALID_DATA` for
/// anything that is not an intact blob of this master key and entropy
/// (to be confirmed: Windows' error for a damaged blob).
pub fn unprotect(master: &[u8], blob: &[u8], entropy: &[u8]) -> Result<(Vec<u8>, Vec<u16>), u32> {
    if blob.len() < HEADER + 16 || &blob[..4] != MAGIC {
        return Err(ERROR_INVALID_DATA);
    }
    let dlen = u32::from_le_bytes([blob[36], blob[37], blob[38], blob[39]]) as usize;
    let aad_len = HEADER.checked_add(dlen).ok_or(ERROR_INVALID_DATA)?;
    if !dlen.is_multiple_of(2) || blob.len() < aad_len + 16 {
        return Err(ERROR_INVALID_DATA);
    }
    let (salt, nonce) = (&blob[8..24], &blob[24..36]);
    let mut data = blob[aad_len..blob.len() - 16].to_vec();
    let tag = &blob[blob.len() - 16..];
    if !key(master, salt, entropy).open(nonce, &blob[..aad_len], &mut data, tag) {
        return Err(ERROR_INVALID_DATA);
    }
    let desc = blob[HEADER..aad_len].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Ok((data, desc))
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
        bad[45] ^= 1; // the description is authenticated
        assert_eq!(unprotect(&master, &bad, b"extra"), Err(ERROR_INVALID_DATA));
        assert_eq!(unprotect(&master, &blob[..20], b"extra"), Err(ERROR_INVALID_DATA));
        let m = protect(&master, b"", &[], b"", true, &[4; 28]);
        assert!(is_machine_blob(&m));
        assert_eq!(unprotect(&master, &m, b"").unwrap().0, b"");
    }
}
