//! Who a test program runs as, the same on both sides of a comparison.
//!
//! On the reference side a test program inherits the user and group ids
//! and supplementary groups of whoever runs the pipeline tool: root in
//! some environments (a container), a normal user in others (GitHub's CI
//! runner). Poly's test init starts the
//! program as root unless told otherwise, so without care a test that
//! calls `getuid()` or meets a permission check diverges before any
//! behaviour is compared. So the reference's ids are recorded
//! ([`Identity::current`]) and given to Poly as kernel arguments
//! ([`Identity::kernel_args`]), which the kernel's test init applies to the
//! program before it runs (`kernel/src/testinit.rs`).
//!
//! The file mode creation mask is not inherited at all: both sides set
//! [`REFERENCE_UMASK`] (022, Linux's usual default) right before the
//! program starts, whatever the tool's caller had (a systemd unit's
//! `UMask=0027`, the role wrapper's `umask 0027`).
//!
//! A test that only makes sense as root says so with `poly-diff:
//! needs=root` (`corpus::directives`); where the reference is not root it
//! is not compared but checked on Poly as root (`main.rs`, `diff_cmd`).

use serde::{Deserialize, Serialize};

/// The umask every test program starts with, on the reference and on Poly.
pub const REFERENCE_UMASK: u32 = 0o022;

/// Kernel arguments of the test-run protocol (ADR 0027, ADR 0081) that set
/// the program's credentials and umask: `poly.test.uid=R[,E,S]`,
/// `poly.test.gid=R[,E,S]`, `poly.test.groups=G1,G2,...`,
/// `poly.test.umask=OCTAL`.
pub const UID_ARG: &str = "poly.test.uid";
pub const GID_ARG: &str = "poly.test.gid";
pub const GROUPS_ARG: &str = "poly.test.groups";
pub const UMASK_ARG: &str = "poly.test.umask";

/// User and group ids (real, effective, saved), supplementary groups and
/// umask of a test program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// Real, effective and saved set-user-ID.
    pub uid: [u32; 3],
    /// Real, effective and saved set-group-ID.
    pub gid: [u32; 3],
    /// Supplementary groups, as getgroups(2) reports them.
    pub groups: Vec<u32>,
    pub umask: u32,
}

impl Identity {
    /// What a test program started by this process runs as: this
    /// process's ids and groups, with [`REFERENCE_UMASK`].
    pub fn current() -> Identity {
        let (mut r, mut e, mut s) = (0, 0, 0);
        // SAFETY: getresuid writes one uid_t through each pointer, which
        // point at the locals above.
        let uid_ok = unsafe { libc::getresuid(&mut r, &mut e, &mut s) } == 0;
        // SAFETY: geteuid cannot fail and touches no memory.
        let uid = if uid_ok { [r, e, s] } else { [unsafe { libc::geteuid() }; 3] };
        let (mut r, mut e, mut s) = (0, 0, 0);
        // SAFETY: as above, for gid_t.
        let gid_ok = unsafe { libc::getresgid(&mut r, &mut e, &mut s) } == 0;
        // SAFETY: getegid cannot fail and touches no memory.
        let gid = if gid_ok { [r, e, s] } else { [unsafe { libc::getegid() }; 3] };
        Identity { uid, gid, groups: current_groups(), umask: REFERENCE_UMASK }
    }

    /// Root with no supplementary groups: what Poly runs a program as
    /// without identity arguments.
    pub fn root() -> Identity {
        Identity { uid: [0; 3], gid: [0; 3], groups: Vec::new(), umask: REFERENCE_UMASK }
    }

    /// Whether the program runs with root's privileges (effective uid 0;
    /// Linux gives such a process every capability).
    pub fn is_root(&self) -> bool {
        self.uid[1] == 0
    }

    /// The kernel arguments that make Poly's test init start the program
    /// as this identity (each word separated by a space, no leading one).
    pub fn kernel_args(&self) -> String {
        let ids = |v: &[u32; 3]| {
            if v[0] == v[1] && v[1] == v[2] { v[0].to_string() } else { format!("{},{},{}", v[0], v[1], v[2]) }
        };
        let mut out = format!("{UID_ARG}={} {GID_ARG}={}", ids(&self.uid), ids(&self.gid));
        if !self.groups.is_empty() {
            let list: Vec<String> = self.groups.iter().map(u32::to_string).collect();
            out.push_str(&format!(" {GROUPS_ARG}={}", list.join(",")));
        }
        out.push_str(&format!(" {UMASK_ARG}={:04o}", self.umask));
        out
    }

    /// For reports: `uid 1000 gid 1000 groups 1000,27 umask 0022`.
    pub fn describe(&self) -> String {
        let ids = |v: &[u32; 3]| {
            if v[0] == v[1] && v[1] == v[2] { v[0].to_string() } else { format!("{}/{}/{}", v[0], v[1], v[2]) }
        };
        let groups = if self.groups.is_empty() {
            "none".to_string()
        } else {
            self.groups.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
        };
        format!("uid {} gid {} groups {groups} umask {:04o}", ids(&self.uid), ids(&self.gid), self.umask)
    }
}

/// This process's supplementary groups.
fn current_groups() -> Vec<u32> {
    // SAFETY: getgroups(0, NULL) only returns the count.
    let n = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    if n <= 0 {
        return Vec::new();
    }
    let mut groups = vec![0 as libc::gid_t; n as usize];
    // SAFETY: the buffer holds `n` gid_t values, the size passed.
    let got = unsafe { libc::getgroups(n, groups.as_mut_ptr()) };
    groups.truncate(got.max(0) as usize);
    groups
}

/// In a child about to exec a test program: the fixed umask. Only
/// async-signal-safe calls. Never call it in the tool's own process.
pub(crate) fn set_reference_umask() {
    // SAFETY: umask(2) only sets the process's mask; it cannot fail.
    unsafe { libc::umask(REFERENCE_UMASK as libc::mode_t) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_args_name_every_id() {
        let id = Identity { uid: [1000; 3], gid: [1000; 3], groups: vec![4, 27, 1000], umask: 0o022 };
        assert_eq!(
            id.kernel_args(),
            "poly.test.uid=1000 poly.test.gid=1000 poly.test.groups=4,27,1000 poly.test.umask=0022"
        );
        assert_eq!(id.describe(), "uid 1000 gid 1000 groups 4,27,1000 umask 0022");
        assert!(!id.is_root());
        // Real, effective and saved ids that differ are given separately;
        // no supplementary groups, no groups argument.
        let id = Identity { uid: [1000, 0, 0], gid: [5, 5, 5], groups: vec![], umask: 0o027 };
        assert_eq!(id.kernel_args(), "poly.test.uid=1000,0,0 poly.test.gid=5 poly.test.umask=0027");
        assert!(id.is_root(), "effective uid 0 is root");
        assert_eq!(Identity::root().kernel_args(), "poly.test.uid=0 poly.test.gid=0 poly.test.umask=0022");
    }

    /// The current identity is this process's, with the fixed umask.
    #[test]
    fn the_current_identity_is_this_process() {
        let id = Identity::current();
        // SAFETY: geteuid and getegid cannot fail and touch no memory.
        let (euid, egid) = unsafe { (libc::geteuid(), libc::getegid()) };
        assert_eq!((id.uid[1], id.gid[1]), (euid, egid));
        assert_eq!(id.umask, REFERENCE_UMASK);
        assert_eq!(id.is_root(), euid == 0);
    }
}
