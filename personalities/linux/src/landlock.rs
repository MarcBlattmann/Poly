//! Landlock (the kernel's documentation `userspace-api/landlock.rst`,
//! UAPI `linux/landlock.h`): unprivileged sandboxing. A process builds a
//! [`Ruleset`] (the accesses it wants restricted, and rules that allow
//! some of them on file hierarchies or TCP ports) and enforces it on
//! itself with `landlock_restrict_self`: its credentials then carry a
//! [`Domain`], a stack of frozen rulesets ([`Layer`]s) that every handled
//! access must pass, layer by layer. Children inherit the domain with
//! their credentials; it can only grow.
//!
//! - File accesses are checked against the chain of objects from the
//!   file up to the root (the kernel gives it as [`ObjectKey`]s): a layer
//!   allows an access it handles if a rule on any object of the chain
//!   allows it. Accesses a layer does not handle pass it.
//! - TCP binds and connects are checked against the port rules.
//! - `LANDLOCK_SCOPE_SIGNAL`: a process may only signal processes in its
//!   own domain or one nested in it (a layer that scopes signals must be a
//!   layer of the target's domain too).
//!
//! Not enforced in Poly yet: `LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET`,
//! `REFER` across directories, `IOCTL_DEV`, and audit logging (the
//! `LANDLOCK_RESTRICT_SELF_LOG_*` flags are accepted and remembered only).

use crate::errno::Errno;
use crate::ns::lock::Mutex;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

/// Constants of UAPI `linux/landlock.h`.
pub mod abi {
    pub const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1 << 0;
    pub const LANDLOCK_CREATE_RULESET_ERRATA: u32 = 1 << 1;

    pub const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;
    pub const LANDLOCK_RULE_NET_PORT: u32 = 2;

    pub const LANDLOCK_ACCESS_FS_EXECUTE: u64 = 1 << 0;
    pub const LANDLOCK_ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
    pub const LANDLOCK_ACCESS_FS_READ_FILE: u64 = 1 << 2;
    pub const LANDLOCK_ACCESS_FS_READ_DIR: u64 = 1 << 3;
    pub const LANDLOCK_ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
    pub const LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
    pub const LANDLOCK_ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
    pub const LANDLOCK_ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
    pub const LANDLOCK_ACCESS_FS_MAKE_REG: u64 = 1 << 8;
    pub const LANDLOCK_ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
    pub const LANDLOCK_ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
    pub const LANDLOCK_ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
    pub const LANDLOCK_ACCESS_FS_MAKE_SYM: u64 = 1 << 12;
    pub const LANDLOCK_ACCESS_FS_REFER: u64 = 1 << 13;
    pub const LANDLOCK_ACCESS_FS_TRUNCATE: u64 = 1 << 14;
    pub const LANDLOCK_ACCESS_FS_IOCTL_DEV: u64 = 1 << 15;
    /// Every file system access bit of ABI 7.
    pub const ACCESS_FS_ALL: u64 = (1 << 16) - 1;
    /// The accesses a rule on a file that is not a directory may allow.
    pub const ACCESS_FILE: u64 = LANDLOCK_ACCESS_FS_EXECUTE
        | LANDLOCK_ACCESS_FS_WRITE_FILE
        | LANDLOCK_ACCESS_FS_READ_FILE
        | LANDLOCK_ACCESS_FS_TRUNCATE
        | LANDLOCK_ACCESS_FS_IOCTL_DEV;

    pub const LANDLOCK_ACCESS_NET_BIND_TCP: u64 = 1 << 0;
    pub const LANDLOCK_ACCESS_NET_CONNECT_TCP: u64 = 1 << 1;
    pub const ACCESS_NET_ALL: u64 = (1 << 2) - 1;

    pub const LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET: u64 = 1 << 0;
    pub const LANDLOCK_SCOPE_SIGNAL: u64 = 1 << 1;
    pub const SCOPE_ALL: u64 = (1 << 2) - 1;

    pub const LANDLOCK_RESTRICT_SELF_LOG_SAME_EXEC_OFF: u32 = 1 << 0;
    pub const LANDLOCK_RESTRICT_SELF_LOG_NEW_EXEC_ON: u32 = 1 << 1;
    pub const LANDLOCK_RESTRICT_SELF_LOG_SUBDOMAINS_OFF: u32 = 1 << 2;
    pub const RESTRICT_SELF_ALL: u32 = (1 << 3) - 1;

    /// `struct landlock_ruleset_attr` of ABI 6 and later (three `__u64`).
    pub const RULESET_ATTR_SIZE: usize = 24;
    /// The smallest size accepted (ABI 1: `handled_access_fs` only).
    pub const RULESET_ATTR_MIN: usize = 8;
    /// `struct landlock_path_beneath_attr` (packed: `__u64`, `__s32`).
    pub const PATH_BENEATH_ATTR_SIZE: usize = 12;
    /// `struct landlock_net_port_attr` (two `__u64`).
    pub const NET_PORT_ATTR_SIZE: usize = 16;
}

use abi::*;

/// The `open` flags the access checks look at (UAPI
/// `asm-generic/fcntl.h`).
pub mod oflags {
    pub const O_ACCMODE: u32 = 0o3;
    pub const O_RDONLY: u32 = 0o0;
    pub const O_WRONLY: u32 = 0o1;
    pub const O_RDWR: u32 = 0o2;
    pub const O_CREAT: u32 = 0o100;
    pub const O_TRUNC: u32 = 0o1000;
    pub const O_PATH: u32 = 0o10000000;
}

/// The Landlock ABI version Poly implements (scopes are ABI 6, the
/// restrict_self logging flags ABI 7).
pub const ABI_VERSION: u64 = 7;
/// The errata Poly's Landlock does not have (the bitmask
/// `LANDLOCK_CREATE_RULESET_ERRATA` returns; the spec sheet's oracle
/// reported 15).
pub const ERRATA: u64 = 15;
/// The most layers a domain may have (`E2BIG` beyond).
pub const MAX_LAYERS: usize = 16;
/// The largest attribute size `landlock_create_ruleset` looks at (a page).
pub const MAX_ATTR_SIZE: usize = 4096;

/// An object of a file system as the kernel names it: the file system's
/// identity and the inode number. A rule follows its inode.
pub type ObjectKey = (u64, u64);

/// What a ruleset restricts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Handled {
    pub fs: u64,
    pub net: u64,
    pub scoped: u64,
}

impl Handled {
    /// `struct landlock_ruleset_attr` from the first `size` bytes the caller
    /// gave (8, 16 or 24 or more; missing fields are 0): `EINVAL` for an
    /// unknown bit, `ENOMSG` if nothing is handled.
    pub fn parse(bytes: &[u8]) -> Result<Handled, Errno> {
        let field =
            |i: usize| bytes.get(i * 8..i * 8 + 8).map_or(0, |b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8])));
        let h = Handled { fs: field(0), net: field(1), scoped: field(2) };
        if h.fs & !ACCESS_FS_ALL != 0 || h.net & !ACCESS_NET_ALL != 0 || h.scoped & !SCOPE_ALL != 0 {
            return Err(Errno::EINVAL);
        }
        if h.fs == 0 && h.net == 0 && h.scoped == 0 {
            return Err(Errno::ENOMSG);
        }
        Ok(h)
    }
}

/// The rules of a ruleset or a layer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    pub handled: Handled,
    /// Accesses allowed on an object and (for a directory) beneath it.
    pub fs: BTreeMap<ObjectKey, u64>,
    /// Accesses allowed on a TCP port.
    pub net: BTreeMap<u16, u64>,
}

/// A rule to add.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    PathBeneath { allowed: u64, object: ObjectKey, is_dir: bool },
    NetPort { allowed: u64, port: u64 },
}

impl Rules {
    /// `landlock_add_rule` once the object is known: the accesses must be
    /// handled (`EINVAL`) and not empty (`ENOMSG`); a file that is no
    /// directory only takes file accesses (`EINVAL`); a port is at most
    /// 65535 (`EINVAL`). Rules on the same object or port accumulate.
    pub fn add(&mut self, rule: Rule) -> Result<(), Errno> {
        match rule {
            Rule::PathBeneath { allowed, object, is_dir } => {
                check_allowed(allowed, self.handled.fs)?;
                if !is_dir && allowed & !ACCESS_FILE != 0 {
                    return Err(Errno::EINVAL);
                }
                *self.fs.entry(object).or_default() |= allowed;
            }
            Rule::NetPort { allowed, port } => {
                check_allowed(allowed, self.handled.net)?;
                let port = u16::try_from(port).map_err(|_| Errno::EINVAL)?;
                *self.net.entry(port).or_default() |= allowed;
            }
        }
        Ok(())
    }
}

/// The checks of `allowed_access` against what the ruleset handles, in the
/// order the call reports them: bits not handled `EINVAL`, none `ENOMSG`.
pub fn check_allowed(allowed: u64, handled: u64) -> Result<(), Errno> {
    if allowed & !handled != 0 {
        return Err(Errno::EINVAL);
    }
    if allowed == 0 {
        return Err(Errno::ENOMSG);
    }
    Ok(())
}

/// What the ruleset part of a rule needs before the object is looked up:
/// [`check_allowed`] for a path rule with the bits known.
pub fn check_path_rule(rules: &Rules, allowed: u64) -> Result<(), Errno> {
    check_allowed(allowed, rules.handled.fs)
}

/// A ruleset as `landlock_create_ruleset` makes it: open (rules can be
/// added) until enforced; enforcing takes a copy.
#[derive(Debug)]
pub struct Ruleset {
    rules: Mutex<Rules>,
}

impl Ruleset {
    pub fn new(handled: Handled) -> Arc<Ruleset> {
        Arc::new(Ruleset { rules: Mutex::new(Rules { handled, ..Rules::default() }) })
    }

    pub fn handled(&self) -> Handled {
        self.rules.lock().handled
    }

    /// See [`Rules::add`].
    pub fn add(&self, rule: Rule) -> Result<(), Errno> {
        self.rules.lock().add(rule)
    }

    /// The rules as they are now.
    pub fn snapshot(&self) -> Rules {
        self.rules.lock().clone()
    }
}

/// One enforced ruleset.
pub type Layer = Rules;

/// The Landlock domain of a thread: its layers, oldest first. Layers are
/// shared with the domains stacked on it, so a domain nests in another
/// when that one's layers are its first ones.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain {
    layers: Vec<Arc<Layer>>,
    /// `LANDLOCK_RESTRICT_SELF_LOG_SUBDOMAINS_OFF` was set (logging is not
    /// done in Poly; remembered for the record).
    pub log_subdomains_off: bool,
}

impl Domain {
    /// `parent` (the thread's domain, if any) with `rules` stacked on top:
    /// `E2BIG` past [`MAX_LAYERS`].
    pub fn stack(parent: Option<&Domain>, rules: Rules) -> Result<Domain, Errno> {
        let mut d = parent.cloned().unwrap_or_default();
        if d.layers.len() >= MAX_LAYERS {
            return Err(Errno::E2BIG);
        }
        d.layers.push(Arc::new(rules));
        Ok(d)
    }

    pub fn layers(&self) -> usize {
        self.layers.len()
    }

    /// Whether some layer handles one of the file system accesses `access`.
    pub fn handles_fs(&self, access: u64) -> bool {
        self.layers.iter().any(|l| l.handled.fs & access != 0)
    }

    /// Whether some layer handles one of the network accesses `access`.
    pub fn handles_net(&self, access: u64) -> bool {
        self.layers.iter().any(|l| l.handled.net & access != 0)
    }

    /// Whether every layer allows `access` on the object whose chain is
    /// `chain` (the object first, the root last).
    pub fn allows_fs(&self, chain: &[ObjectKey], access: u64) -> bool {
        self.layers.iter().all(|l| {
            let needed = access & l.handled.fs;
            if needed == 0 {
                return true;
            }
            let allowed = chain.iter().filter_map(|k| l.fs.get(k)).fold(0, |a, b| a | b);
            needed & !allowed == 0
        })
    }

    /// Whether every layer that handles file accesses allows `REFER` on
    /// the chain: implicitly handled by every such layer (landlock.rst).
    pub fn allows_refer(&self, chain: &[ObjectKey]) -> bool {
        self.layers.iter().all(|l| {
            l.handled.fs == 0
                || chain.iter().filter_map(|k| l.fs.get(k)).fold(0, |a, b| a | b) & LANDLOCK_ACCESS_FS_REFER != 0
        })
    }

    /// Whether every layer allows `access` on TCP port `port`.
    pub fn allows_net(&self, port: u16, access: u64) -> bool {
        self.layers.iter().all(|l| {
            let needed = access & l.handled.net;
            needed == 0 || needed & !l.net.get(&port).copied().unwrap_or(0) == 0
        })
    }
}

/// `LANDLOCK_SCOPE_SIGNAL`: whether a thread in domain `sender` may
/// signal one in `target`. Every layer of the sender that scopes signals
/// must also be a layer of the target, at the same place.
pub fn may_signal(sender: Option<&Domain>, target: Option<&Domain>) -> bool {
    let Some(s) = sender else { return true };
    s.layers.iter().enumerate().all(|(i, l)| {
        l.handled.scoped & LANDLOCK_SCOPE_SIGNAL == 0
            || target.and_then(|t| t.layers.get(i)).is_some_and(|tl| Arc::ptr_eq(tl, l))
    })
}

/// The file system accesses opening a file asks for (landlock.rst):
/// reading a file `READ_FILE`, listing a directory `READ_DIR`, writing
/// `WRITE_FILE`, `O_TRUNC` `TRUNCATE`. `O_PATH` asks for nothing.
pub fn open_access(flags: u32, is_dir: bool) -> u64 {
    use oflags::*;
    if flags & O_PATH != 0 {
        return 0;
    }
    let mode = flags & O_ACCMODE;
    let mut a = 0;
    if mode == O_RDONLY || mode == O_RDWR {
        a |= if is_dir { LANDLOCK_ACCESS_FS_READ_DIR } else { LANDLOCK_ACCESS_FS_READ_FILE };
    }
    if (mode == O_WRONLY || mode == O_RDWR) && !is_dir {
        a |= LANDLOCK_ACCESS_FS_WRITE_FILE;
    }
    if flags & O_TRUNC != 0 && !is_dir {
        a |= LANDLOCK_ACCESS_FS_TRUNCATE;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: ObjectKey = (1, 10);
    const A_F: ObjectKey = (1, 11);
    const A_G: ObjectKey = (1, 12);
    const ROOT: ObjectKey = (1, 2);
    const ETC: ObjectKey = (1, 3);
    const HOSTNAME: ObjectKey = (1, 4);

    fn ruleset(fs: u64, net: u64, scoped: u64) -> Rules {
        Rules { handled: Handled { fs, net, scoped }, ..Rules::default() }
    }

    #[test]
    fn attr_parsing() {
        let mut b = [0u8; 24];
        assert_eq!(Handled::parse(&b), Err(Errno::ENOMSG));
        b[0] = LANDLOCK_ACCESS_FS_READ_FILE as u8;
        assert_eq!(Handled::parse(&b[..8]).unwrap().fs, LANDLOCK_ACCESS_FS_READ_FILE);
        b[2] = 1; // bit 16 of handled_access_fs
        assert_eq!(Handled::parse(&b), Err(Errno::EINVAL));
        b[2] = 0;
        b[8] = 4; // unknown net bit
        assert_eq!(Handled::parse(&b), Err(Errno::EINVAL));
        b[8] = 3;
        b[16] = 3;
        assert_eq!(Handled::parse(&b).unwrap(), Handled { fs: 4, net: 3, scoped: 3 });
    }

    #[test]
    fn rule_checks() {
        let mut r = ruleset(LANDLOCK_ACCESS_FS_READ_FILE, 0, 0);
        let pb = |allowed, is_dir| Rule::PathBeneath { allowed, object: A, is_dir };
        assert_eq!(r.add(pb(LANDLOCK_ACCESS_FS_WRITE_FILE, true)), Err(Errno::EINVAL));
        assert_eq!(r.add(pb(0, true)), Err(Errno::ENOMSG));
        assert_eq!(r.add(Rule::NetPort { allowed: 1, port: 80 }), Err(Errno::EINVAL));
        assert_eq!(r.add(pb(LANDLOCK_ACCESS_FS_READ_FILE, false)), Ok(()));
        let mut d = ruleset(LANDLOCK_ACCESS_FS_READ_DIR, 1, 0);
        let dir_only = Rule::PathBeneath { allowed: LANDLOCK_ACCESS_FS_READ_DIR, object: A_F, is_dir: false };
        assert_eq!(d.add(dir_only), Err(Errno::EINVAL));
        assert_eq!(d.add(Rule::NetPort { allowed: 1, port: 70000 }), Err(Errno::EINVAL));
        assert_eq!(d.add(Rule::NetPort { allowed: 1, port: 65535 }), Ok(()));
    }

    #[test]
    fn path_beneath_and_stacking() {
        let mut r = ruleset(LANDLOCK_ACCESS_FS_READ_FILE, 0, 0);
        r.add(Rule::PathBeneath { allowed: LANDLOCK_ACCESS_FS_READ_FILE, object: A, is_dir: true }).unwrap();
        let d1 = Domain::stack(None, r).unwrap();
        let read = LANDLOCK_ACCESS_FS_READ_FILE;
        assert!(d1.allows_fs(&[A_G, A, ROOT], read));
        assert!(!d1.allows_fs(&[HOSTNAME, ETC, ROOT], read));
        assert!(d1.allows_fs(&[HOSTNAME, ETC, ROOT], LANDLOCK_ACCESS_FS_WRITE_FILE), "not handled");
        let mut r2 = ruleset(read, 0, 0);
        r2.add(Rule::PathBeneath { allowed: read, object: A_F, is_dir: false }).unwrap();
        let d2 = Domain::stack(Some(&d1), r2).unwrap();
        assert!(d2.allows_fs(&[A_F, A, ROOT], read));
        assert!(!d2.allows_fs(&[A_G, A, ROOT], read));
        let mut d = Domain::default();
        for _ in 0..MAX_LAYERS {
            d = Domain::stack(Some(&d), ruleset(read, 0, 0)).unwrap();
        }
        assert_eq!(Domain::stack(Some(&d), ruleset(read, 0, 0)), Err(Errno::E2BIG));
    }

    #[test]
    fn ports() {
        let mut r = ruleset(0, LANDLOCK_ACCESS_NET_BIND_TCP, 0);
        r.add(Rule::NetPort { allowed: LANDLOCK_ACCESS_NET_BIND_TCP, port: 47011 }).unwrap();
        let d = Domain::stack(None, r).unwrap();
        assert!(d.allows_net(47011, LANDLOCK_ACCESS_NET_BIND_TCP));
        assert!(!d.allows_net(47012, LANDLOCK_ACCESS_NET_BIND_TCP));
        assert!(d.allows_net(47012, LANDLOCK_ACCESS_NET_CONNECT_TCP), "connect not handled");
        assert!(!d.handles_fs(ACCESS_FS_ALL));
    }

    #[test]
    fn signal_scope() {
        let scoped = Domain::stack(None, ruleset(0, 0, LANDLOCK_SCOPE_SIGNAL)).unwrap();
        assert!(!may_signal(Some(&scoped), None), "to the parent outside the domain");
        assert!(may_signal(Some(&scoped), Some(&scoped)), "itself, its children");
        let nested = Domain::stack(Some(&scoped), ruleset(LANDLOCK_ACCESS_FS_READ_FILE, 0, 0)).unwrap();
        assert!(may_signal(Some(&scoped), Some(&nested)), "a nested domain");
        assert!(!may_signal(Some(&nested), Some(&Domain::stack(None, ruleset(0, 0, 2)).unwrap())));
        assert!(may_signal(None, Some(&scoped)), "out of no domain");
        let unscoped = Domain::stack(None, ruleset(LANDLOCK_ACCESS_FS_READ_FILE, 0, 0)).unwrap();
        assert!(may_signal(Some(&unscoped), None));
    }

    #[test]
    fn open_accesses() {
        use super::oflags::*;
        assert_eq!(open_access(O_RDONLY, false), LANDLOCK_ACCESS_FS_READ_FILE);
        assert_eq!(open_access(O_RDONLY, true), LANDLOCK_ACCESS_FS_READ_DIR);
        assert_eq!(open_access(O_WRONLY | O_TRUNC, false), LANDLOCK_ACCESS_FS_WRITE_FILE | LANDLOCK_ACCESS_FS_TRUNCATE);
        assert_eq!(open_access(O_RDWR, false), LANDLOCK_ACCESS_FS_WRITE_FILE | LANDLOCK_ACCESS_FS_READ_FILE);
        assert_eq!(open_access(O_PATH | O_RDWR, false), 0);
    }
}
