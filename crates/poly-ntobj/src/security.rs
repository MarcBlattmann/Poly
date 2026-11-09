//! Security in the object manager (ADR 0171): access tokens and who holds
//! them, and the security descriptor of each object.
//!
//! - Every process has a primary token and a thread may have an
//!   impersonation token ([`ObjectSecurity::effective`] is the one a
//!   request is checked with: the thread's, else the process's).
//! - A token is an object of type `Token` whose cookie is the token's id;
//!   its contents ([`poly_security::token::Token`]) are kept here. A process
//!   or thread holds a pointer reference on its token's object, so the token
//!   lives as long as a handle, a process or a thread uses it.
//! - An object may have a security descriptor (self-relative bytes). Opening
//!   an object that has one is checked against the caller's effective token
//!   ([`poly_security::access::check`]); one without is not checked. A new
//!   named object gets a default descriptor from its creator's token, an
//!   explicit one from `OBJECT_ATTRIBUTES` is completed with the defaults.
//!   Kernel-mode callers are not checked (Learn: `ObOpenObjectByPointer`).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use poly_security::access::{self, GenericMapping as SecMapping};
use poly_security::acl;
use poly_security::sd::{self, Parts};
use poly_security::sdops;
use poly_security::token::{POLY_TOKEN_LUID, SE_PRIVILEGE_ENABLED, Token};

use crate::access::{AccessMask, Mode, Privileges};
use crate::object::{ObjectId, ObjectType};
use crate::status::*;
use crate::wait::ThreadRef;
use crate::{Caller, NewObject, ObjectManager, ProcessId};

/// A token and its object.
#[derive(Clone, Debug)]
pub struct TokenEntry {
    pub token: Token,
    pub object: ObjectId,
}

/// Tokens and descriptors.
#[derive(Debug)]
pub struct ObjectSecurity {
    tokens: BTreeMap<u64, TokenEntry>,
    process: BTreeMap<ProcessId, u64>,
    thread: BTreeMap<ThreadRef, u64>,
    sds: BTreeMap<ObjectId, Vec<u8>>,
    next_luid: u64,
    /// The descriptors of [`DefaultSd`], built once.
    file_sd: Vec<u8>,
    key_sd: Vec<u8>,
}

/// The descriptor a file system or the registry gives what it makes
/// (ADR 0171; the real ones are inherited from the parent, which Poly's
/// file systems do not store, so this is the common NTFS / registry shape):
/// owner Administrators, group SYSTEM, SYSTEM and Administrators full,
/// Users and RESTRICTED read (and execute for files). Files also let
/// Authenticated Users modify (a folder under the user profile or the temp
/// folder) and Everyone read and execute (what `KnownDlls` gives a
/// restricted token on Windows: the system DLLs load for any token).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultSd {
    File,
    Key,
}

const FILE_SDDL: &str = "O:BAG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1301bf;;;AU)(A;OICI;FRFX;;;BU)(A;OICI;FRFX;;;RC)(A;OICI;FRFX;;;WD)";
const KEY_SDDL: &str = "O:BAG:SYD:P(A;CI;KA;;;SY)(A;CI;KA;;;BA)(A;CI;KR;;;BU)(A;CI;KR;;;RC)";

fn sddl(s: &str) -> Vec<u8> {
    let w: Vec<u16> = s.encode_utf16().collect();
    poly_security::sdstr::parse(&w).expect("the built-in descriptor is valid SDDL")
}

impl Default for ObjectSecurity {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectSecurity {
    pub fn new() -> ObjectSecurity {
        ObjectSecurity {
            tokens: BTreeMap::new(),
            process: BTreeMap::new(),
            thread: BTreeMap::new(),
            sds: BTreeMap::new(),
            next_luid: POLY_TOKEN_LUID + 0x100,
            file_sd: sddl(FILE_SDDL),
            key_sd: sddl(KEY_SDDL),
        }
    }

    /// A new locally unique id (for a token).
    pub fn new_luid(&mut self) -> u64 {
        self.next_luid += 1;
        self.next_luid
    }

    pub fn token(&self, id: u64) -> Option<&Token> {
        self.tokens.get(&id).map(|e| &e.token)
    }

    pub fn token_mut(&mut self, id: u64) -> Option<&mut Token> {
        self.tokens.get_mut(&id).map(|e| &mut e.token)
    }

    pub fn token_object(&self, id: u64) -> Option<ObjectId> {
        self.tokens.get(&id).map(|e| e.object)
    }

    /// The id of `pid`'s primary token.
    pub fn process_token(&self, pid: ProcessId) -> Option<u64> {
        self.process.get(&pid).copied()
    }

    /// The id of `thread`'s impersonation token.
    pub fn thread_token(&self, thread: ThreadRef) -> Option<u64> {
        self.thread.get(&thread).copied()
    }

    /// The token a request by `thread` of `process` is checked with.
    pub fn effective(&self, process: ProcessId, thread: ThreadRef) -> Option<u64> {
        self.thread_token(thread).or_else(|| self.process_token(process))
    }

    /// The stored descriptor of `object` (self-relative).
    pub fn sd(&self, object: ObjectId) -> Option<&[u8]> {
        self.sds.get(&object).map(Vec::as_slice)
    }

    pub fn has_sd(&self, object: ObjectId) -> bool {
        self.sds.contains_key(&object)
    }

    /// Tokens held.
    pub fn token_count(&self) -> usize {
        self.tokens.len()
    }
}

/// `GENERIC_MAPPING` of an object type, as the security crate's.
pub fn mapping_of(kind: ObjectType) -> SecMapping {
    let m = kind.generic_mapping();
    SecMapping { read: m.read, write: m.write, execute: m.execute, all: m.all }
}

impl ObjectManager {
    /// A new token object (one pointer reference for the caller) holding
    /// `token` under a new id; its descriptor is derived from the token
    /// (owner, group, the default DACL it was made from).
    pub fn new_token(&mut self, mut token: Token, thread: ThreadRef) -> Result<(u64, ObjectId)> {
        let id = self.security.new_luid();
        token.id = id;
        let object = self.new_object(
            NewObject::Opaque { kind: ObjectType::Token, cookie: id, signaled: None, parse: false },
            thread,
        )?;
        let sd = token.object_sd().to_relative();
        self.security.sds.insert(object, sd);
        self.security.tokens.insert(id, TokenEntry { token, object });
        Ok((id, object))
    }

    /// Make token `id` the primary token of `pid` (it holds a pointer
    /// reference on the token's object; the old token's is dropped).
    pub fn set_process_token(&mut self, pid: ProcessId, id: u64) -> Result<()> {
        let object = self.security.token_object(id).ok_or(STATUS_INVALID_PARAMETER)?;
        self.reference(object)?;
        if let Some(old) = self.security.process.insert(pid, id)
            && let Some(o) = self.security.token_object(old)
        {
            let _ = self.dereference(o);
        }
        Ok(())
    }

    /// Make token `id` (or none) the impersonation token of `thread`.
    pub fn set_thread_token(&mut self, thread: ThreadRef, id: Option<u64>) -> Result<()> {
        if let Some(id) = id {
            let object = self.security.token_object(id).ok_or(STATUS_INVALID_PARAMETER)?;
            self.reference(object)?;
        }
        let old = match id {
            Some(id) => self.security.thread.insert(thread, id),
            None => self.security.thread.remove(&thread),
        };
        if let Some(old) = old
            && let Some(o) = self.security.token_object(old)
        {
            let _ = self.dereference(o);
        }
        Ok(())
    }

    /// A process ended: it no longer holds its token.
    pub fn forget_process_token(&mut self, pid: ProcessId) {
        if let Some(id) = self.security.process.remove(&pid)
            && let Some(o) = self.security.token_object(id)
        {
            let _ = self.dereference(o);
        }
    }

    /// A thread ended: it no longer holds an impersonation token.
    pub fn forget_thread_token(&mut self, thread: ThreadRef) {
        let _ = self.set_thread_token(thread, None);
    }

    /// An opaque object was deleted: forget its descriptor, and its token
    /// if it was one.
    pub fn security_object_deleted(&mut self, d: &crate::manager::Deleted) {
        self.security.sds.remove(&d.id);
        if d.kind == ObjectType::Token {
            self.security.tokens.remove(&d.cookie);
        }
    }

    /// The privileges of `caller`'s effective token (or the ones the caller
    /// says it has, without a token).
    pub fn effective_privileges(&self, caller: &Caller) -> Privileges {
        let Some(t) = self.security.effective(caller.process, caller.thread).and_then(|i| self.security.token(i))
        else {
            return caller.privileges;
        };
        let enabled = |luid: u32| t.privileges.iter().any(|&(l, a)| l == luid && a & SE_PRIVILEGE_ENABLED != 0);
        Privileges { create_permanent: enabled(16), security: enabled(8) }
    }

    /// What `desired` access to `id` the descriptor leaves `caller`:
    /// `None` when nothing is to be checked (no descriptor, no token, a
    /// kernel-mode caller), else the access granted or why not.
    pub(crate) fn dacl_grant(
        &self,
        caller: &Caller,
        id: ObjectId,
        desired: AccessMask,
        force: bool,
    ) -> Option<Result<AccessMask>> {
        if caller.mode == Mode::Kernel && !force {
            return None;
        }
        let sd = self.security.sds.get(&id)?;
        let tid = self.security.effective(caller.process, caller.thread)?;
        let token = self.security.token(tid)?;
        let parts = Parts::from_relative(sd)?;
        let kind = self.obj(id).header.kind;
        let mapping = mapping_of(kind);
        let want = mapping.map(desired);
        Some(match access::check(&parts, &token.check_view(), want, &mapping) {
            access::Outcome { status: Ok(()), granted, .. } => Ok(granted & !access::MAXIMUM_ALLOWED),
            access::Outcome { status: Err(poly_security::err::SecErr::PrivilegeNotHeld), .. } => {
                Err(STATUS_PRIVILEGE_NOT_HELD)
            }
            _ => Err(STATUS_ACCESS_DENIED),
        })
    }

    /// Give `id` (of type `kind`) its security descriptor: `explicit`
    /// (self-relative) completed with the defaults of `creator`'s token, its
    /// generic rights mapped. No token, no descriptor.
    pub fn assign_security(&mut self, id: ObjectId, kind: ObjectType, creator: Option<u64>, explicit: Option<&[u8]>) {
        let Some(token) = creator.and_then(|t| self.security.token(t)) else { return };
        let mapping = mapping_of(kind);
        let map = |m: u32| mapping.map(m);
        let mut parts = sdops::default_descriptor(token, &map);
        if matches!(kind, ObjectType::Process | ObjectType::Thread | ObjectType::Token) {
            // Processes, threads and tokens carry their creator's integrity
            // as a mandatory label (the sheets: `S:AI(ML;;NW;;;HI)`).
            // Tokens: no write up; processes and threads: no read up either
            // (the sheets: 0x1 on the token, 0x3 on the process).
            let policy = if kind == ObjectType::Token { 1 } else { 3 };
            parts.sacl = Some(sdops::label_sacl(token.integrity_rid(), policy));
            parts.control |= sd::SE_SACL_PRESENT | sd::SE_SACL_AUTO_INHERITED;
        }
        if let Some(e) = explicit.and_then(Parts::from_relative) {
            if let Some(o) = e.owner {
                parts.owner = Some(o);
            }
            if let Some(g) = e.group {
                parts.group = Some(g);
            }
            if e.control & sd::SE_DACL_PRESENT != 0 {
                parts.dacl = e.dacl.map(|d| sdops::trimmed_acl(&map_acl(&d, &map)));
                parts.control |=
                    e.control & (sd::SE_DACL_PROTECTED | sd::SE_DACL_AUTO_INHERITED | sd::SE_DACL_AUTO_INHERIT_REQ);
            }
            if e.control & sd::SE_SACL_PRESENT != 0 {
                parts.sacl = e.sacl.map(|a| sdops::trimmed_acl(&a));
                parts.control |= sd::SE_SACL_PRESENT;
            }
        }
        self.security.sds.insert(id, parts.to_relative());
    }

    /// Give `id` (a file or key object the file system or the registry just
    /// made) its [`DefaultSd`], so opening it is checked against the
    /// caller's token.
    pub fn assign_default_security(&mut self, id: ObjectId, which: DefaultSd) {
        let bytes = match which {
            DefaultSd::File => self.security.file_sd.clone(),
            DefaultSd::Key => self.security.key_sd.clone(),
        };
        self.security.sds.insert(id, bytes);
    }

    /// Give a new object made by `create_object` its descriptor: an
    /// explicit one from `OBJECT_ATTRIBUTES`, or the default for the kinds
    /// that have one when they are named.
    pub(crate) fn assign_new_security(
        &mut self,
        caller: &Caller,
        id: ObjectId,
        kind: ObjectType,
        explicit: Option<&[u8]>,
        named: bool,
    ) {
        let default_kind = matches!(
            kind,
            ObjectType::Event | ObjectType::Mutant | ObjectType::Semaphore | ObjectType::Timer | ObjectType::Section
        );
        // A job object has its descriptor named or not (the sheets).
        let always = kind == ObjectType::Job;
        if explicit.is_none() && !always && !(named && default_kind) {
            return;
        }
        let creator = self.security.effective(caller.process, caller.thread);
        self.assign_security(id, kind, creator, explicit);
    }

    /// Replace the stored descriptor of `id` (generic rights mapped).
    pub fn replace_security(&mut self, id: ObjectId, kind: ObjectType, mut parts: Parts) {
        let mapping = mapping_of(kind);
        let map = |m: u32| mapping.map(m);
        parts.dacl = parts.dacl.map(|d| sdops::trimmed_acl(&map_acl(&d, &map)));
        parts.sacl = parts.sacl.map(|a| sdops::trimmed_acl(&a));
        self.security.sds.insert(id, parts.to_relative());
    }
}

/// An ACL with the generic rights of each allow and deny ACE mapped.
fn map_acl(acl_bytes: &[u8], map: &dyn Fn(u32) -> u32) -> Vec<u8> {
    let mut out = acl_bytes.to_vec();
    let mut at = acl::ACL_HEADER;
    let count = usize::from(u16::from_le_bytes([out[4], out[5]]));
    for _ in 0..count {
        if at + acl::ACE_FIXED > out.len() {
            break;
        }
        let kind = out[at];
        let len = usize::from(u16::from_le_bytes([out[at + 2], out[at + 3]]));
        if len < acl::ACE_FIXED {
            break;
        }
        if kind == acl::ACCESS_ALLOWED_ACE_TYPE || kind == acl::ACCESS_DENIED_ACE_TYPE {
            let m = u32::from_le_bytes([out[at + 4], out[at + 5], out[at + 6], out[at + 7]]);
            out[at + 4..at + 8].copy_from_slice(&map(m).to_le_bytes());
        }
        at += len;
    }
    out
}
