//! SHA-256 of downloads and cached files in pure Rust (`poly-crypto`), so
//! pinned fetches also work on hosts without `sha256sum` (Windows).

use std::io::Read;
use std::path::Path;

/// The SHA-256 of `data`, as lower-case hex.
pub fn hex(data: &[u8]) -> String {
    let mut h = poly_crypto::sha256::Sha256::new();
    h.update(data);
    to_hex(&h.finish())
}

/// The SHA-256 of the file at `path`, as lower-case hex, or `None` if it
/// cannot be read.
pub fn file(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = poly_crypto::sha256::Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => h.update(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(to_hex(&h.finish()))
}

fn to_hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        assert_eq!(hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn a_file_hashes_like_its_bytes() {
        let path = std::env::temp_dir().join(format!("xtask-sha256-{}", std::process::id()));
        let data: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        std::fs::write(&path, &data).unwrap();
        assert_eq!(file(&path), Some(hex(&data)));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(file(&path), None);
    }
}
