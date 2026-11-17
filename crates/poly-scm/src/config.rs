//! A service's configuration, how it is checked, and the registry values it is kept in
//! (`HKLM\SYSTEM\CurrentControlSet\Services\<name>`, SERVICES.md section 1).

use crate::consts::*;
use crate::wire::{FailureActions, join_multi, split_multi};
use alloc::vec::Vec;
use poly_registry::consts::{REG_BINARY, REG_DWORD, REG_EXPAND_SZ, REG_MULTI_SZ, REG_SZ};

/// What the database keeps of a service.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// The name as it was created.
    pub name: Vec<u16>,
    /// `None`: no `DisplayName` value (the display name is the name).
    pub display: Option<Vec<u16>>,
    pub ty: u32,
    pub start: u32,
    pub error: u32,
    /// `ImagePath` as given.
    pub path: Vec<u16>,
    pub group: Vec<u16>,
    pub deps: Vec<Vec<u16>>,
    /// `ObjectName`.
    pub account: Vec<u16>,
    pub description: Option<Vec<u16>>,
    pub failure: Option<FailureActions>,
    pub failure_flag: Option<u32>,
    pub delayed: Option<u32>,
    pub sid_type: Option<u32>,
    pub preshutdown: Option<u32>,
    pub delete_flag: bool,
    /// Created by a 32-bit program (`WOW64` 0x14c).
    pub wow64: bool,
}

/// The values the manager writes (and removes when they have no content).
const MANAGED: [&str; 15] = [
    "Type",
    "Start",
    "ErrorControl",
    "ImagePath",
    "DisplayName",
    "ObjectName",
    "DependOnService",
    "Description",
    "FailureActions",
    "FailureActionsOnNonCrashFailures",
    "DelayedAutostart",
    "ServiceSidType",
    "PreshutdownTimeout",
    "DeleteFlag",
    "WOW64",
];

/// `IMAGE_FILE_MACHINE_I386`, the `WOW64` value of a service a 32-bit program created.
pub const WOW64_I386: u32 = 0x14C;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn sz(s: &[u16]) -> Vec<u8> {
    s.iter().chain(core::iter::once(&0)).flat_map(|c| c.to_le_bytes()).collect()
}

fn dword(v: u32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

/// The units of a string value (up to its first NUL).
pub fn units_of(data: &[u8]) -> Vec<u16> {
    let all: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let end = all.iter().position(|&c| c == 0).unwrap_or(all.len());
    all[..end].to_vec()
}

fn dword_of(data: &[u8]) -> Option<u32> {
    (data.len() >= 4).then(|| u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}

/// The `FailureActions` binary: reset (4), two zero pointers (8), the count (4), the offset of the
/// actions (0x14 when there are some), then type and delay for each (the sheet of
/// `ChangeServiceConfig2W`).
pub fn failure_bytes(f: &FailureActions) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&f.reset.to_le_bytes());
    v.extend_from_slice(&[0; 8]);
    v.extend_from_slice(&(f.actions.len() as u32).to_le_bytes());
    v.extend_from_slice(&(if f.actions.is_empty() { 0u32 } else { 0x14 }).to_le_bytes());
    for &(t, d) in &f.actions {
        v.extend_from_slice(&t.to_le_bytes());
        v.extend_from_slice(&d.to_le_bytes());
    }
    v
}

fn failure_of(data: &[u8]) -> Option<FailureActions> {
    let reset = dword_of(data)?;
    let n = dword_of(data.get(12..)?)? as usize;
    let at = dword_of(data.get(16..)?)? as usize;
    let at = if at == 0 { 20 } else { at };
    let actions = (0..n.min(64))
        .map(|i| Some((dword_of(data.get(at + 8 * i..)?)?, dword_of(data.get(at + 8 * i + 4..)?)?)))
        .collect::<Option<Vec<_>>>()?;
    Some(FailureActions { reset, reboot_msg: None, command: None, actions })
}

impl Config {
    /// The display name the calls return: the `DisplayName`, else the name.
    pub fn display_name(&self) -> &[u16] {
        self.display.as_deref().unwrap_or(&self.name)
    }

    /// The registry values, in the order they are written: `(name, kind, data)`.
    pub fn values(&self) -> Vec<(Vec<u16>, u32, Vec<u8>)> {
        let mut v = Vec::new();
        v.push((w("Type"), REG_DWORD, dword(self.ty)));
        v.push((w("Start"), REG_DWORD, dword(self.start)));
        v.push((w("ErrorControl"), REG_DWORD, dword(self.error)));
        v.push((w("ImagePath"), REG_EXPAND_SZ, sz(&self.path)));
        if let Some(d) = &self.display {
            v.push((w("DisplayName"), REG_SZ, sz(d)));
        }
        v.push((w("ObjectName"), REG_SZ, sz(&self.account)));
        if !self.deps.is_empty() {
            let units = join_multi(&self.deps);
            v.push((w("DependOnService"), REG_MULTI_SZ, units.iter().flat_map(|c| c.to_le_bytes()).collect()));
        }
        if let Some(d) = &self.description {
            v.push((w("Description"), REG_SZ, sz(d)));
        }
        if let Some(f) = &self.failure {
            v.push((w("FailureActions"), REG_BINARY, failure_bytes(f)));
        }
        if let Some(x) = self.failure_flag {
            v.push((w("FailureActionsOnNonCrashFailures"), REG_DWORD, dword(x)));
        }
        if let Some(x) = self.delayed {
            v.push((w("DelayedAutostart"), REG_DWORD, dword(x)));
        }
        if let Some(x) = self.sid_type {
            v.push((w("ServiceSidType"), REG_DWORD, dword(x)));
        }
        if let Some(x) = self.preshutdown {
            v.push((w("PreshutdownTimeout"), REG_DWORD, dword(x)));
        }
        if self.delete_flag {
            v.push((w("DeleteFlag"), REG_DWORD, dword(1)));
        }
        if self.wow64 {
            v.push((w("WOW64"), REG_DWORD, dword(WOW64_I386)));
        }
        v
    }

    /// The names of the values the manager owns.
    pub fn managed_names() -> impl Iterator<Item = Vec<u16>> {
        MANAGED.iter().map(|s| w(s))
    }

    /// A service read back from its key's values (`None` when the key is not a service: no
    /// `Type`, or a driver, which Poly does not run).
    pub fn from_values(name: &[u16], values: &[(Vec<u16>, u32, Vec<u8>)]) -> Option<Config> {
        let get = |n: &str| {
            let n = w(n);
            values.iter().find(|(k, _, _)| eq_fold(k, &n)).map(|(_, kind, d)| (*kind, d.as_slice()))
        };
        let num = |n: &str| get(n).and_then(|(_, d)| dword_of(d));
        let text = |n: &str| get(n).map(|(_, d)| units_of(d));
        let ty = num("Type")?;
        if ty & SERVICE_WIN32 == 0 {
            return None;
        }
        let deps = get("DependOnService")
            .map(|(_, d)| split_multi(&d.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()))
            .unwrap_or_default();
        Some(Config {
            name: name.to_vec(),
            display: text("DisplayName"),
            ty,
            start: num("Start").unwrap_or(SERVICE_DEMAND_START),
            error: num("ErrorControl").unwrap_or(1),
            path: text("ImagePath").unwrap_or_default(),
            group: text("Group").unwrap_or_default(),
            deps,
            account: text("ObjectName").unwrap_or_else(|| w("LocalSystem")),
            description: text("Description"),
            failure: get("FailureActions").and_then(|(_, d)| failure_of(d)),
            failure_flag: num("FailureActionsOnNonCrashFailures"),
            delayed: num("DelayedAutostart"),
            sid_type: num("ServiceSidType"),
            preshutdown: num("PreshutdownTimeout"),
            delete_flag: num("DeleteFlag") == Some(1),
            wow64: num("WOW64").is_some(),
        })
    }
}

/// Case-insensitive equality of two names (the units of the Basic Multilingual Plane upper-cased).
pub fn eq_fold(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| poly_security::service::upcase(x) == poly_security::service::upcase(y))
}

/// The key a name is found under.
pub fn fold(a: &[u16]) -> Vec<u16> {
    a.iter().map(|&c| poly_security::service::upcase(c)).collect()
}

/// Check a service name (`CreateServiceW`, `OpenServiceW`): 1783 above 256 units, 123 when empty
/// or with `\` or `/`.
pub fn check_name(name: &[u16]) -> Result<(), u32> {
    if name.len() > MAX_NAME {
        return Err(RPC_X_BAD_STUB_DATA);
    }
    if name.is_empty() || name.iter().any(|&c| c == u16::from(b'\\') || c == u16::from(b'/')) {
        return Err(ERROR_INVALID_NAME);
    }
    Ok(())
}

/// Whether `ty` is a type a service can be created with.
pub fn type_valid(ty: u32) -> bool {
    matches!(
        ty,
        SERVICE_KERNEL_DRIVER
            | SERVICE_FILE_SYSTEM_DRIVER
            | SERVICE_WIN32_OWN_PROCESS
            | SERVICE_WIN32_SHARE_PROCESS
            | 0x110
            | 0x120
    )
}

/// Whether start type `start` goes with type `ty`: boot and system start only for drivers, and
/// Poly loads no drivers at boot (the sheet: refused for the test's path).
pub fn start_valid(ty: u32, start: u32) -> bool {
    match start {
        SERVICE_AUTO_START | SERVICE_DEMAND_START | SERVICE_DISABLED => true,
        SERVICE_SYSTEM_START => ty & SERVICE_DRIVER != 0,
        _ => false,
    }
}

/// Expand `%NAME%` in `text` with `lookup` (a name it does not know stays as it is).
pub fn expand(text: &[u16], lookup: &dyn Fn(&[u16]) -> Option<Vec<u16>>) -> Vec<u16> {
    let pct = u16::from(b'%');
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if text[i] == pct
            && let Some(len) = text[i + 1..].iter().position(|&c| c == pct)
            && len > 0
        {
            let name = &text[i + 1..i + 1 + len];
            if let Some(v) = lookup(name) {
                out.extend_from_slice(&v);
                i += len + 2;
                continue;
            }
        }
        out.push(text[i]);
        i += 1;
    }
    out
}

/// The program a command line names: its first word, quoted or not.
pub fn image_of(command: &[u16]) -> Vec<u16> {
    let q = u16::from(b'"');
    let trimmed: &[u16] = {
        let start = command.iter().position(|&c| c != u16::from(b' ')).unwrap_or(command.len());
        &command[start..]
    };
    if trimmed.first() == Some(&q) {
        let rest = &trimmed[1..];
        let end = rest.iter().position(|&c| c == q).unwrap_or(rest.len());
        return rest[..end].to_vec();
    }
    let end = trimmed.iter().position(|&c| c == u16::from(b' ') || c == u16::from(b'\t')).unwrap_or(trimmed.len());
    trimmed[..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_actions_bytes_are_the_sheets() {
        let f = FailureActions {
            reset: 86400,
            reboot_msg: None,
            command: None,
            actions: alloc::vec![(1, 1000), (1, 2000), (0, 0)],
        };
        let hex: alloc::string::String = failure_bytes(&f).iter().map(|b| alloc::format!("{b:02x}")).collect();
        assert_eq!(hex, "805101000000000000000000030000001400000001000000e803000001000000d00700000000000000000000");
        assert_eq!(failure_of(&failure_bytes(&f)), Some(f));
    }

    #[test]
    fn values_round_trip() {
        let c = Config {
            name: w("S"),
            display: Some(w("Shown")),
            ty: 0x10,
            start: 2,
            error: 1,
            path: w("\"C:\\x.exe\" --a"),
            deps: alloc::vec![w("RPCSS")],
            account: w("LocalSystem"),
            description: Some(w("d")),
            wow64: true,
            ..Config::default()
        };
        let back = Config::from_values(&w("S"), &c.values()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn names_and_paths() {
        assert_eq!(check_name(&w("a\\b")), Err(ERROR_INVALID_NAME));
        assert_eq!(check_name(&[78; 257]), Err(RPC_X_BAD_STUB_DATA));
        assert!(check_name(&[78; 256]).is_ok());
        assert_eq!(image_of(&w("\"C:\\a b\\x.exe\" --child run")), w("C:\\a b\\x.exe"));
        assert_eq!(image_of(&w("C:\\x.exe /a")), w("C:\\x.exe"));
        let env = |n: &[u16]| (n == w("SystemRoot").as_slice()).then(|| w("C:\\Windows"));
        assert_eq!(expand(&w("%SystemRoot%\\x %NOPE% 100%"), &env), w("C:\\Windows\\x %NOPE% 100%"));
        assert!(eq_fold(&w("RpcSs"), &w("RPCSS")));
    }
}
