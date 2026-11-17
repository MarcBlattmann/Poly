//! The account names a principal or a logon trigger accepts, the name they read back as, and the SID
//! the XML holds (Learn: "Well-known SIDs"; the sheets `IPrincipal.UserId`, `ILogonTrigger.UserId`,
//! `ITaskFolder.RegisterTaskDefinition`).

use alloc::string::String;

/// A well-known account: the name a setter stores, its SID, whether it is a service account (a task
/// for it stores the logon type SERVICE_ACCOUNT), and the other names that mean it.
struct Known {
    name: &'static str,
    sid: &'static str,
    service: bool,
    aliases: &'static [&'static str],
}

const KNOWN: &[Known] = &[
    Known { name: "SYSTEM", sid: "S-1-5-18", service: true, aliases: &["SYSTEM", "NT AUTHORITY\\SYSTEM", "S-1-5-18"] },
    Known {
        name: "LOCAL SERVICE",
        sid: "S-1-5-19",
        service: true,
        aliases: &["LOCAL SERVICE", "LocalService", "NT AUTHORITY\\LocalService", "NT AUTHORITY\\LOCAL SERVICE", "S-1-5-19"],
    },
    Known {
        name: "NETWORK SERVICE",
        sid: "S-1-5-20",
        service: true,
        aliases: &[
            "NETWORK SERVICE",
            "NetworkService",
            "NT AUTHORITY\\NetworkService",
            "NT AUTHORITY\\NETWORK SERVICE",
            "S-1-5-20",
        ],
    },
    Known {
        name: "Administrators",
        sid: "S-1-5-32-544",
        service: false,
        aliases: &["Administrators", "BUILTIN\\Administrators", "S-1-5-32-544"],
    },
    Known { name: "Users", sid: "S-1-5-32-545", service: false, aliases: &["Users", "BUILTIN\\Users", "S-1-5-32-545"] },
];

fn known(s: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.aliases.iter().any(|a| a.eq_ignore_ascii_case(s)))
}

/// The name a setter stores for `s`: a well-known account by its name (`S-1-5-18` → `SYSTEM`),
/// anything else as given.
pub fn canonical(s: &str) -> String {
    match known(s) {
        Some(k) => String::from(k.name),
        None => String::from(s),
    }
}

/// The SID the XML holds for a stored name (`SYSTEM` → `S-1-5-18`); other text as it is.
pub fn xml_form(s: &str) -> String {
    match known(s) {
        Some(k) => String::from(k.sid),
        None => String::from(s),
    }
}

/// Whether `s` names SYSTEM, LOCAL SERVICE or NETWORK SERVICE.
pub fn is_service_account(s: &str) -> bool {
    known(s).is_some_and(|k| k.service)
}

/// Whether `s` names a well-known account at all.
pub fn is_known(s: &str) -> bool {
    known(s).is_some()
}
