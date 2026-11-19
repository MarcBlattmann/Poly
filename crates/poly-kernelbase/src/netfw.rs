//! What our `hnetcfg.dll` answers for the Windows Firewall's COM API (`NetFwPolicy2`,
//! `INetFwRules`, `NetFwRule`): the properties of a rule with the values each accepts, how ports
//! read back, which rules a collection may take, and the profiles a policy reports.
//!
//! Poly has no host firewall: the rules are kept in a table and enforce nothing (the sheet allows
//! that: no test sends traffic). The behaviour is the spec sheet's
//! (`specs/windows/hnetcfg.dll/NetFwPolicy2.yaml`) and Learn's pages for `INetFwPolicy2`,
//! `INetFwRules` and `INetFwRule`; the constants are `icftypes.h`'s and `winerror.h`'s.

use alloc::vec::Vec;

/// `E_INVALIDARG` (`winerror.h`).
pub const E_INVALIDARG: u32 = 0x8007_0057;
/// `E_POINTER` (`winerror.h`).
pub const E_POINTER: u32 = 0x8000_4003;
/// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`: `Item` of a name that is not there.
pub const HR_NOT_FOUND: u32 = 0x8007_0002;
/// `HRESULT_FROM_WIN32(ERROR_INVALID_DATA)`: ports that do not parse.
pub const HR_INVALID_DATA: u32 = 0x8007_000D;
/// `HRESULT_FROM_WIN32(ERROR_NOT_SUPPORTED)`: a profile value that names several profiles.
pub const HR_NOT_SUPPORTED: u32 = 0x8007_0032;

/// `NET_FW_IP_PROTOCOL_ANY` (`icftypes.h`).
pub const PROTOCOL_ANY: i32 = 256;
/// `NET_FW_RULE_DIR_IN`, `NET_FW_RULE_DIR_OUT` (`icftypes.h`).
pub const DIR_IN: i32 = 1;
pub const DIR_OUT: i32 = 2;
/// `NET_FW_ACTION_BLOCK`, `NET_FW_ACTION_ALLOW` (`icftypes.h`).
pub const ACTION_BLOCK: i32 = 0;
pub const ACTION_ALLOW: i32 = 1;
/// `NET_FW_PROFILE2_DOMAIN`, `_PRIVATE`, `_PUBLIC`, `_ALL` (`icftypes.h`).
pub const PROFILE_DOMAIN: i32 = 1;
pub const PROFILE_PRIVATE: i32 = 2;
pub const PROFILE_PUBLIC: i32 = 4;
pub const PROFILE_ALL: i32 = 0x7FFF_FFFF;
/// `VARIANT_TRUE` (`wtypes.h`).
pub const VARIANT_TRUE: i16 = -1;

/// The profile Poly reports as current (`get_CurrentProfileTypes`): a private network.
pub const CURRENT_PROFILES: i32 = PROFILE_PRIVATE;

/// `INetFwPolicy2`'s per-profile getters: one profile bit is a profile, several are not supported,
/// anything else is not a profile.
pub fn check_profile(profile: i32) -> Result<(), u32> {
    match profile {
        PROFILE_DOMAIN | PROFILE_PRIVATE | PROFILE_PUBLIC => Ok(()),
        p if p > 0 && p & !7 == 0 => Err(HR_NOT_SUPPORTED),
        _ => Err(E_INVALIDARG),
    }
}

/// A firewall rule's properties (`INetFwRule`). Strings are UTF-16 without a NUL; `None` is a NULL
/// `BSTR`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub name: Option<Vec<u16>>,
    pub description: Option<Vec<u16>>,
    pub application: Option<Vec<u16>>,
    pub service: Option<Vec<u16>>,
    pub protocol: i32,
    pub local_ports: Option<Vec<u16>>,
    pub remote_ports: Option<Vec<u16>>,
    pub local_addresses: Option<Vec<u16>>,
    pub remote_addresses: Option<Vec<u16>>,
    pub icmp_types: Option<Vec<u16>>,
    pub direction: i32,
    pub interface_types: Option<Vec<u16>>,
    pub enabled: bool,
    pub grouping: Option<Vec<u16>>,
    pub profiles: i32,
    pub edge_traversal: bool,
    pub action: i32,
}

impl Default for Rule {
    /// A new rule (the sheet): no strings, any protocol, inbound, allow, disabled, all profiles.
    fn default() -> Rule {
        Rule {
            name: None,
            description: None,
            application: None,
            service: None,
            protocol: PROTOCOL_ANY,
            local_ports: None,
            remote_ports: None,
            local_addresses: None,
            remote_addresses: None,
            icmp_types: None,
            direction: DIR_IN,
            interface_types: None,
            enabled: false,
            grouping: None,
            profiles: PROFILE_ALL,
            edge_traversal: false,
            action: ACTION_ALLOW,
        }
    }
}

impl Rule {
    /// `put_Protocol`: an IP protocol number (0 to 255) or `NET_FW_IP_PROTOCOL_ANY`.
    pub fn set_protocol(&mut self, v: i32) -> Result<(), u32> {
        if !(0..=PROTOCOL_ANY).contains(&v) {
            return Err(E_INVALIDARG);
        }
        self.protocol = v;
        Ok(())
    }

    /// `put_Direction`: in or out.
    pub fn set_direction(&mut self, v: i32) -> Result<(), u32> {
        if v != DIR_IN && v != DIR_OUT {
            return Err(E_INVALIDARG);
        }
        self.direction = v;
        Ok(())
    }

    /// `put_Action`: block or allow.
    pub fn set_action(&mut self, v: i32) -> Result<(), u32> {
        if v != ACTION_BLOCK && v != ACTION_ALLOW {
            return Err(E_INVALIDARG);
        }
        self.action = v;
        Ok(())
    }

    /// `put_Profiles`: all profiles stay all; any other value keeps its three profile bits (8 and 0
    /// read back as 0, -1 as 7).
    pub fn set_profiles(&mut self, v: i32) {
        self.profiles = if v == PROFILE_ALL { v } else { v & 7 };
    }

    /// `put_LocalPorts` / `put_RemotePorts`: ports need a protocol (not "any"); the text reads back
    /// in its normal form ([`normal_ports`]).
    pub fn set_ports(&mut self, local: bool, text: Option<&[u16]>) -> Result<(), u32> {
        let value = match text {
            None => None,
            Some(t) => {
                if self.protocol == PROTOCOL_ANY {
                    return Err(E_INVALIDARG);
                }
                Some(normal_ports(t)?)
            }
        };
        if local {
            self.local_ports = value;
        } else {
            self.remote_ports = value;
        }
        Ok(())
    }

    /// Whether a collection takes the rule (`INetFwRules::Add`): it needs a name.
    pub fn check_add(&self) -> Result<(), u32> {
        match &self.name {
            Some(n) if !n.is_empty() => Ok(()),
            _ => Err(E_INVALIDARG),
        }
    }

    /// Whether the rule has this name (names compare without regard to ASCII case).
    pub fn is_named(&self, name: &[u16]) -> bool {
        self.name.as_deref().is_some_and(|n| eq_fold(n, name))
    }
}

fn eq_fold(a: &[u16], b: &[u16]) -> bool {
    let fold = |c: u16| if (0x41..=0x5A).contains(&c) { c + 32 } else { c };
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| fold(x) == fold(y))
}

/// The keywords Learn lists for a rule's ports, in the spelling they read back with.
const PORT_KEYWORDS: &[&str] = &["RPC", "RPC-EPMap", "Teredo", "IPHTTPS", "IPHTTPSIn", "IPTLSIn"];

fn number(t: &[u16]) -> Option<u32> {
    if t.is_empty() || t.len() > 5 || !t.iter().all(|c| (0x30..=0x39).contains(c)) {
        return None;
    }
    let n = t.iter().fold(0u32, |n, &c| n * 10 + u32::from(c - 0x30));
    (n <= 65535).then_some(n)
}

/// A port list in the form it reads back: ports, ranges (`5000-5100`) and `*` joined by commas;
/// each keyword followed by a comma (`RPC` reads back `RPC,`); empty items dropped (`5353,` is
/// `5353`); an empty list is `*`. Anything else is `HRESULT_FROM_WIN32(ERROR_INVALID_DATA)`.
pub fn normal_ports(text: &[u16]) -> Result<Vec<u16>, u32> {
    let mut keywords: Vec<u16> = Vec::new();
    let mut ports: Vec<Vec<u16>> = Vec::new();
    for item in text.split(|&c| c == u16::from(b',')) {
        let item = trim(item);
        if item.is_empty() {
            continue;
        }
        if item == [u16::from(b'*')] {
            ports.push(item.to_vec());
        } else if let Some(k) = PORT_KEYWORDS.iter().find(|k| eq_fold(item, &wide(k))) {
            keywords.extend(wide(k));
            keywords.push(u16::from(b','));
        } else if let Some(dash) = item.iter().position(|&c| c == u16::from(b'-')) {
            let (low, high) = (number(&item[..dash]), number(&item[dash + 1..]));
            match (low, high) {
                (Some(l), Some(h)) if l <= h => ports.push(item.to_vec()),
                _ => return Err(HR_INVALID_DATA),
            }
        } else if number(item).is_some() {
            ports.push(item.to_vec());
        } else {
            return Err(HR_INVALID_DATA);
        }
    }
    if keywords.is_empty() && ports.is_empty() {
        return Ok(wide("*"));
    }
    let mut out = keywords;
    for (i, p) in ports.iter().enumerate() {
        if i > 0 {
            out.push(u16::from(b','));
        }
        out.extend_from_slice(p);
    }
    Ok(out)
}

fn trim(t: &[u16]) -> &[u16] {
    let start = t.iter().position(|&c| c != 0x20).unwrap_or(t.len());
    let end = t.iter().rposition(|&c| c != 0x20).map_or(start, |e| e + 1);
    &t[start..end]
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// The rules every process's table starts with: Poly has no host firewall, so its policy is that
/// all traffic passes, which these two enabled rules state.
pub fn builtin_rules() -> Vec<Rule> {
    [
        (DIR_IN, "Poly: all inbound traffic (no host firewall)"),
        (DIR_OUT, "Poly: all outbound traffic (no host firewall)"),
    ]
    .into_iter()
    .map(|(direction, name)| Rule {
        name: Some(wide(name)),
        description: Some(wide("Poly has no host firewall: every connection is allowed.")),
        grouping: Some(wide("Poly")),
        direction,
        enabled: true,
        ..Rule::default()
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        wide(s)
    }

    fn ports(s: &str) -> Result<alloc::string::String, u32> {
        normal_ports(&w(s)).map(|p| alloc::string::String::from_utf16(&p).unwrap())
    }

    #[test]
    fn a_new_rule_has_the_sheets_defaults() {
        let r = Rule::default();
        assert_eq!((r.protocol, r.direction, r.action, r.enabled, r.profiles), (256, 1, 1, false, 0x7FFF_FFFF));
        assert!(r.name.is_none() && r.local_ports.is_none() && r.grouping.is_none() && !r.edge_traversal);
    }

    #[test]
    fn values_and_ranges() {
        let mut r = Rule::default();
        for p in [6, 17, 1, 58, 256, 0, 255] {
            assert_eq!(r.set_protocol(p), Ok(()));
            assert_eq!(r.protocol, p);
        }
        assert_eq!(r.set_protocol(300), Err(E_INVALIDARG));
        assert_eq!(r.set_protocol(-1), Err(E_INVALIDARG));
        assert_eq!(r.protocol, 255);
        assert_eq!(r.set_direction(2), Ok(()));
        for d in [0, 3, -1] {
            assert_eq!(r.set_direction(d), Err(E_INVALIDARG));
        }
        assert_eq!(r.direction, 2);
        assert_eq!(r.set_action(0), Ok(()));
        assert_eq!(r.set_action(2), Err(E_INVALIDARG));
        for (v, back) in [(1, 1), (2, 2), (4, 4), (7, 7), (0x7FFF_FFFF, 0x7FFF_FFFF), (8, 0), (0, 0), (-1, 7)] {
            r.set_profiles(v);
            assert_eq!(r.profiles, back, "{v}");
        }
    }

    #[test]
    fn ports_read_back_in_their_normal_form() {
        assert_eq!(ports("5353"), Ok("5353".into()));
        assert_eq!(ports("80,443"), Ok("80,443".into()));
        assert_eq!(ports("5000-5100"), Ok("5000-5100".into()));
        assert_eq!(ports("*"), Ok("*".into()));
        assert_eq!(ports("RPC"), Ok("RPC,".into()));
        assert_eq!(ports("0"), Ok("0".into()));
        assert_eq!(ports(""), Ok("*".into()));
        assert_eq!(ports("5353,"), Ok("5353".into()));
        for bad in ["abc", "70000", "1-"] {
            assert_eq!(ports(bad), Err(HR_INVALID_DATA), "{bad}");
        }
    }

    #[test]
    fn ports_need_a_protocol() {
        let mut r = Rule::default();
        assert_eq!(r.set_ports(true, Some(&w("5353"))), Err(E_INVALIDARG));
        assert_eq!(r.local_ports, None);
        r.set_protocol(17).unwrap();
        assert_eq!(r.set_ports(true, Some(&w("RPC"))), Ok(()));
        assert_eq!(r.set_ports(true, Some(&w("abc"))), Err(HR_INVALID_DATA));
        assert_eq!(r.local_ports, Some(w("RPC,")));
        r.set_protocol(1).unwrap();
        assert_eq!(r.set_ports(true, Some(&w("5353"))), Ok(()));
    }

    #[test]
    fn profiles_and_adding() {
        assert_eq!(check_profile(1), Ok(()));
        assert_eq!(check_profile(4), Ok(()));
        assert_eq!(check_profile(3), Err(HR_NOT_SUPPORTED));
        for p in [0, 8, 0x7FFF_FFFF, -1] {
            assert_eq!(check_profile(p), Err(E_INVALIDARG), "{p}");
        }
        let mut r = Rule::default();
        assert_eq!(r.check_add(), Err(E_INVALIDARG));
        r.name = Some(w("PolySpecB4 minimal"));
        assert_eq!(r.check_add(), Ok(()));
        assert!(r.is_named(&w("polyspecb4 MINIMAL")));
        assert!(!builtin_rules().is_empty());
        assert!(builtin_rules().iter().all(|r| r.check_add().is_ok()));
    }
}
