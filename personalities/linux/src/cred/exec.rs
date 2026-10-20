//! What execve(2) does to credentials: set-user-ID and set-group-ID
//! programs, and the capability sets recalculated per capabilities(7),
//! "Transformation of capabilities during execve()", with the rules for
//! root ("Capabilities and execution of programs by root"). Poly has no
//! file capabilities, so a program's file sets are empty unless the root
//! rules make them full.

use super::{CapSet, Caps, Creds};

/// Mode bits (UAPI `linux/stat.h`).
pub const S_ISUID: u16 = 0o4000;
pub const S_ISGID: u16 = 0o2000;
pub const S_IXGRP: u16 = 0o010;

/// What execve(2) needs to know about the program file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecFile {
    /// The file's owner and group.
    pub uid: u32,
    pub gid: u32,
    /// Its permission bits, set-user-ID and set-group-ID included.
    pub perm: u16,
    /// It is on a mount with `nosuid` (mount(2) `MS_NOSUID`: "Do not
    /// honor set-user-ID and set-group-ID bits").
    pub nosuid: bool,
}

impl Creds {
    /// The credentials of the thread after it execs `file`, and whether the
    /// program must be treated securely (getauxval(3) `AT_SECURE`: "the
    /// process is executing a set-user-ID or set-group-ID binary (so that
    /// its real and effective UIDs or GIDs differ from one another)").
    ///
    /// execve(2): "If the set-user-ID bit is set on the program file ...,
    /// then the effective user ID of the calling process is changed to
    /// that of the owner of the program file. Similarly, if the
    /// set-group-ID bit is set on the program file, then the effective
    /// group ID of the calling process is set to the group of the program
    /// file." A set-group-ID bit without group execute marks mandatory
    /// locking (inode(7)) and is ignored here. Neither applies on a
    /// `nosuid` mount or with `no_new_privs` (prctl(2)). "The effective
    /// user ID of the process is copied to the saved set-user-ID;
    /// similarly, the effective group ID is copied to the saved
    /// set-group-ID"; the file system ids follow the effective ones.
    /// `SECBIT_KEEP_CAPS` "is always cleared on an execve(2)".
    pub fn exec(&self, file: &ExecFile) -> (Creds, bool) {
        let mut new = self.clone();
        let honor = !file.nosuid && !self.no_new_privs;
        let mut privileged_file = false;
        if honor && file.perm & S_ISUID != 0 {
            new.euid = file.uid;
            privileged_file = true;
        }
        if honor && file.perm & S_ISGID != 0 && file.perm & S_IXGRP != 0 {
            new.egid = file.gid;
            privileged_file = true;
        }
        new.suid = new.euid;
        new.fsuid = new.euid;
        new.sgid = new.egid;
        new.fsgid = new.egid;
        new.keep_caps = false;
        new.caps = exec_caps(&self.caps, new.uid, new.euid, privileged_file);
        let secure = new.uid != new.euid || new.gid != new.egid;
        (new, secure)
    }
}

/// capabilities(7):
///
/// ```text
/// P'(ambient)     = (file is privileged) ? 0 : P(ambient)
/// P'(permitted)   = (P(inheritable) & F(inheritable)) |
///                   (F(permitted) & P(bounding)) | P'(ambient)
/// P'(effective)   = F(effective) ? P'(permitted) : P'(ambient)
/// P'(inheritable) = P(inheritable)    [i.e., unchanged]
/// P'(bounding)    = P(bounding)       [i.e., unchanged]
/// ```
///
/// Without file capabilities F is empty, except: "If the real or effective
/// user ID of the process is 0 ..., then the file inheritable and
/// permitted sets are ignored; instead they are notionally considered to
/// be all ones", and "If the effective user ID of the process is 0 ...,
/// then the file effective bit is notionally defined to be one". The ids
/// are those after the set-user-ID change, so a set-user-ID-root program
/// gains its capabilities.
fn exec_caps(old: &Caps, uid: u32, euid: u32, privileged_file: bool) -> Caps {
    let root = uid == 0 || euid == 0;
    let file_sets = if root { CapSet::ALL } else { CapSet::EMPTY };
    let ambient = if privileged_file { CapSet::EMPTY } else { old.ambient };
    let permitted = old.inheritable.intersection(file_sets).union(file_sets.intersection(old.bounding)).union(ambient);
    let effective = if euid == 0 { permitted } else { ambient };
    let mut caps = Caps { effective, permitted, inheritable: old.inheritable, bounding: old.bounding, ambient };
    caps.fix_ambient();
    caps
}
