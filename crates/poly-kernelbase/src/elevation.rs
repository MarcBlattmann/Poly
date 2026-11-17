//! User Account Control's rules for starting a program (ADR 0222): the execution level an
//! executable's manifest requests, and whether a caller may start it without a consent.
//!
//! Learn ("Application Manifests", `trustInfo` / `requestedExecutionLevel`): the level is
//! `asInvoker`, `highestAvailable` or `requireAdministrator`; the manifest is the image's
//! `RT_MANIFEST` resource (24) with id 1, or, for an image that has none, an external
//! `<name>.exe.manifest` beside it. The spec sheets (`kernel32.dll/CreateProcessElevation.yaml`,
//! `shell32.dll/ShellExecuteExW.Elevation.yaml`): `CreateProcessW` of an image that requests
//! `requireAdministrator`, or `highestAvailable` when the user is an administrator, from a caller
//! that is not elevated fails with `ERROR_ELEVATION_REQUIRED` (740) and creates nothing;
//! `ShellExecuteEx` turns that into a consent prompt, as it does for the verb `runas`.

pub use crate::verinfo::ReadAt;
use crate::verinfo;
use alloc::vec::Vec;

/// `ERROR_ELEVATION_REQUIRED` (`winerror.h`).
pub const ERROR_ELEVATION_REQUIRED: u32 = 740;
/// `ERROR_CANCELLED` (`winerror.h`): the user said no to the consent prompt.
pub const ERROR_CANCELLED: u32 = 1223;
/// `RT_MANIFEST` and the id of a program's own manifest.
const RT_MANIFEST: u32 = 24;
const MANIFEST_ID: u32 = 1;
/// The most of an external manifest that is read.
pub const MAX_EXTERNAL_MANIFEST: usize = 1 << 20;

/// What a manifest's `requestedExecutionLevel` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// No manifest, no level, or `asInvoker`: the caller's token.
    AsInvoker,
    /// `highestAvailable`: the most the user can have (an administrator's full token).
    HighestAvailable,
    /// `requireAdministrator`.
    RequireAdministrator,
}

/// The text without case (UTF-8, or UTF-16 LE as some resource compilers write it).
fn lower_ascii(text: &[u8]) -> Vec<u8> {
    if text.len() >= 2 && (text[1] == 0 || text.starts_with(&[0xFF, 0xFE])) {
        text.chunks(2).filter(|c| c.get(1).is_none_or(|&h| h == 0)).map(|c| c[0].to_ascii_lowercase()).collect()
    } else {
        text.iter().map(|c| c.to_ascii_lowercase()).collect()
    }
}

/// The level a manifest's text requests: the `level` attribute of its `requestedExecutionLevel`
/// element (attribute names and values compared without case).
pub fn requested_level(manifest: &[u8]) -> Level {
    let text = lower_ascii(manifest);
    let find = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).position(|w| w == needle);
    let Some(at) = find(&text, b"requestedexecutionlevel") else { return Level::AsInvoker };
    let rest = &text[at..];
    let end = rest.iter().position(|&c| c == b'>').unwrap_or(rest.len());
    let element = &rest[..end];
    // `level` as a whole attribute name: preceded by white space.
    let mut i = 0;
    while let Some(p) = find(&element[i..], b"level") {
        let p = i + p;
        i = p + 5;
        if p == 0 || !element[p - 1].is_ascii_whitespace() {
            continue;
        }
        let mut j = p + 5;
        while element.get(j).is_some_and(u8::is_ascii_whitespace) {
            j += 1;
        }
        if element.get(j) != Some(&b'=') {
            continue;
        }
        j += 1;
        while element.get(j).is_some_and(u8::is_ascii_whitespace) {
            j += 1;
        }
        let Some(&quote) = element.get(j).filter(|&&q| q == b'"' || q == b'\'') else { continue };
        let value = &element[j + 1..];
        let value = &value[..value.iter().position(|&c| c == quote).unwrap_or(value.len())];
        return match value {
            b"requireadministrator" => Level::RequireAdministrator,
            b"highestavailable" => Level::HighestAvailable,
            _ => Level::AsInvoker,
        };
    }
    Level::AsInvoker
}

/// The embedded manifest of the PE file `read` reads (`RT_MANIFEST` 1, the first language).
pub fn embedded_manifest(read: ReadAt<'_>) -> Option<Vec<u8>> {
    verinfo::read_resource(read, RT_MANIFEST, MANIFEST_ID).ok()
}

/// The level of an image: its embedded manifest's, else the external manifest's (`external`,
/// the bytes of `<name>.exe.manifest` if there is one), else [`Level::AsInvoker`].
pub fn image_level(read: ReadAt<'_>, external: Option<&[u8]>) -> Level {
    match embedded_manifest(read) {
        Some(m) => requested_level(&m),
        None => external.map_or(Level::AsInvoker, requested_level),
    }
}

/// Whether starting an image of `level` needs an elevation the caller does not have: the caller
/// is `elevated` (its token's `TokenElevation`), and `admin` says whether the user is an
/// administrator who could elevate (a filtered token with a full one linked).
pub fn needs_elevation(level: Level, elevated: bool, admin: bool) -> bool {
    match level {
        Level::AsInvoker => false,
        Level::RequireAdministrator => !elevated,
        Level::HighestAvailable => !elevated && admin,
    }
}

/// The path of the external manifest of `image` (UTF-16, no NUL): `<image>.manifest`.
pub fn external_manifest_path(image: &[u16]) -> Vec<u16> {
    let mut p = image.to_vec();
    p.extend(".manifest".encode_utf16());
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(level: &str) -> Vec<u8> {
        alloc::format!(
            "<?xml version='1.0' encoding='UTF-8' standalone='yes'?>\n<assembly xmlns='urn:schemas-microsoft-com:asm.v1' \
             manifestVersion='1.0'>\n  <trustInfo xmlns=\"urn:schemas-microsoft-com:asm.v3\">\n    <security>\n      \
             <requestedPrivileges>\n        <requestedExecutionLevel level=\"{level}\" uiAccess=\"false\"/>\n      \
             </requestedPrivileges>\n    </security>\n  </trustInfo>\n</assembly>\n"
        )
        .into_bytes()
    }

    #[test]
    fn the_three_levels_and_none() {
        assert_eq!(requested_level(&manifest("asInvoker")), Level::AsInvoker);
        assert_eq!(requested_level(&manifest("requireAdministrator")), Level::RequireAdministrator);
        assert_eq!(requested_level(&manifest("highestAvailable")), Level::HighestAvailable);
        assert_eq!(requested_level(b"<assembly/>"), Level::AsInvoker);
        assert_eq!(requested_level(b""), Level::AsInvoker);
        // Single quotes, other spacing, case, and `uiAccess` first.
        assert_eq!(
            requested_level(b"<requestedExecutionLevel uiAccess='false' LEVEL = 'RequireAdministrator'/>"),
            Level::RequireAdministrator
        );
    }

    #[test]
    fn utf16_manifests_are_read() {
        let m: Vec<u8> = "<requestedExecutionLevel level=\"requireAdministrator\"/>"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(requested_level(&m), Level::RequireAdministrator);
    }

    #[test]
    fn who_needs_a_consent() {
        use Level::*;
        // A filtered administrator: both requests need it.
        assert!(needs_elevation(RequireAdministrator, false, true));
        assert!(needs_elevation(HighestAvailable, false, true));
        assert!(!needs_elevation(AsInvoker, false, true));
        // Elevated: nothing does.
        assert!(!needs_elevation(RequireAdministrator, true, true));
        assert!(!needs_elevation(HighestAvailable, true, true));
        // Not an administrator: highestAvailable runs as the user.
        assert!(!needs_elevation(HighestAvailable, false, false));
        assert!(needs_elevation(RequireAdministrator, false, false));
    }

    #[test]
    fn an_image_without_a_manifest_takes_the_external_one() {
        // Not a PE file at all: no embedded manifest.
        let file = [0u8; 64];
        let mut read = |at: u64, buf: &mut [u8]| -> Result<usize, u32> {
            let at = at as usize;
            if at >= file.len() {
                return Ok(0);
            }
            let n = buf.len().min(file.len() - at);
            buf[..n].copy_from_slice(&file[at..at + n]);
            Ok(n)
        };
        assert_eq!(image_level(&mut read, Some(&manifest("requireAdministrator"))), Level::RequireAdministrator);
        assert_eq!(image_level(&mut read, None), Level::AsInvoker);
        let p: Vec<u16> = "C:\\a.exe".encode_utf16().collect();
        assert_eq!(String::from_utf16(&external_manifest_path(&p)).unwrap(), "C:\\a.exe.manifest");
    }

    extern crate std;
    use std::string::String;
}
