//! `NtCreateLowBoxToken` (ADR 0196): the argument rules of the sheets of `CreateAppContainerToken`
//! as the service enforces them, the number the system gives a container, and what the token
//! reports.

use super::*;
use poly_ntobj::status::*;
use poly_security::appcontainer::{capability_sids, package_sid};
use poly_security::token::class;

const TOKEN_ALL_ACCESS: u64 = 0xF_01FF;
const MAXIMUM_ALLOWED: u64 = 0x0200_0000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn put(f: &mut Fake, bytes: &[u8]) -> u64 {
    let at = f.alloc(bytes.len().max(1));
    f.mem.write_bytes(at, bytes).unwrap();
    at
}

/// A handle to the process's own token with every right.
fn own_token(f: &mut Fake) -> u64 {
    let out = f.put_u64(0);
    assert_eq!(f.call(Service::OpenProcessToken, &[SELF, TOKEN_ALL_ACCESS, out]), STATUS_SUCCESS);
    f.u64_at(out)
}

/// `NtCreateLowBoxToken(&new, source, MAXIMUM_ALLOWED, NULL, package, count, capabilities, 0, NULL)`:
/// the status and the new handle.
fn lowbox(f: &mut Fake, source: u64, package: &[u8], capabilities: &[&[u8]]) -> (NtStatus, u64) {
    let out = f.put_u64(0);
    let sid = put(f, package);
    // The list: `SID_AND_ATTRIBUTES` entries (a pointer and the attributes `SE_GROUP_ENABLED`).
    let mut list = Vec::new();
    for c in capabilities {
        let p = put(f, c);
        list.extend_from_slice(&p.to_le_bytes());
        list.extend_from_slice(&4u64.to_le_bytes());
    }
    let list_at = if capabilities.is_empty() { 0 } else { put(f, &list) };
    let st = f.call(
        Service::CreateLowBoxToken,
        &[out, source, MAXIMUM_ALLOWED, 0, sid, capabilities.len() as u64, list_at, 0, 0],
    );
    (st, f.u64_at(out))
}

fn query_u32(f: &mut Fake, token: u64, class: u32) -> u32 {
    let buf = f.alloc(64);
    let ret = f.put_u64(0);
    assert_eq!(f.call(Service::QueryInformationToken, &[token, u64::from(class), buf, 64, ret]), STATUS_SUCCESS);
    f.u32_at(buf)
}

#[test]
fn a_lowbox_token_reports_its_container_and_a_number_per_sid() {
    let mut f = Fake::new();
    let src = own_token(&mut f);
    let a = package_sid(&wide("polys2b.a")).to_bytes();
    let b = package_sid(&wide("polys2b.b")).to_bytes();
    let (st, t1) = lowbox(&mut f, src, &a, &[]);
    assert_eq!(st, STATUS_SUCCESS);
    assert_eq!(query_u32(&mut f, t1, class::TOKEN_IS_APP_CONTAINER), 1);
    assert_eq!(query_u32(&mut f, t1, class::TOKEN_IS_SANDBOXED), 1);
    assert_eq!(query_u32(&mut f, src, class::TOKEN_IS_APP_CONTAINER), 0);
    let n1 = query_u32(&mut f, t1, class::TOKEN_APP_CONTAINER_NUMBER);
    assert_ne!(n1, 0);
    // The same SID, the same number; another one, a larger one.
    let (_, t2) = lowbox(&mut f, src, &a, &[]);
    assert_eq!(query_u32(&mut f, t2, class::TOKEN_APP_CONTAINER_NUMBER), n1);
    let (_, t3) = lowbox(&mut f, src, &b, &[]);
    assert!(query_u32(&mut f, t3, class::TOKEN_APP_CONTAINER_NUMBER) > n1);
    // Low integrity, and a source's copy of it as a container's token.
    let buf = f.alloc(64);
    let ret = f.put_u64(0);
    assert_eq!(f.call(Service::QueryInformationToken, &[t1, 25, buf, 64, ret]), STATUS_SUCCESS);
    let label = f.u64_at(buf);
    assert_eq!(f.u32_at(label + 8), 0x1000, "the label's RID is Low");
    // Its SID, 8 + 40 bytes.
    assert_eq!(f.call(Service::QueryInformationToken, &[t1, 31, buf, 64, ret]), STATUS_SUCCESS);
    assert_eq!(f.u64_at(ret), 48);
}

#[test]
fn the_service_checks_its_arguments() {
    let mut f = Fake::new();
    let src = own_token(&mut f);
    let good = package_sid(&wide("polys2b.c")).to_bytes();
    let (_, cap) = capability_sids(&wide("internetClient"));
    let cap = cap.to_bytes();
    let (_, cap2) = capability_sids(&wide("registryRead"));
    let cap2 = cap2.to_bytes();
    // Capabilities: capability SIDs, each once.
    assert_eq!(lowbox(&mut f, src, &good, &[&cap, &cap2]).0, STATUS_SUCCESS);
    assert_eq!(lowbox(&mut f, src, &good, &[&cap, &cap]).0, STATUS_INVALID_PARAMETER);
    let everyone = poly_security::sid::world().to_bytes();
    assert_eq!(lowbox(&mut f, src, &good, &[&everyone]).0, STATUS_INVALID_PARAMETER);
    // The container is a package SID: 8 sub-authorities.
    assert_eq!(lowbox(&mut f, src, &cap, &[]).0, STATUS_INVALID_PARAMETER);
    let child = poly_security::sid::Sid::new(15, &[2, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]).to_bytes();
    assert_eq!(lowbox(&mut f, src, &child, &[]).0, STATUS_INVALID_PARAMETER);
    // A source token needs a handle with duplicate and query rights.
    let out = f.put_u64(0);
    assert_eq!(f.call(Service::OpenProcessToken, &[SELF, 0x8, out]), STATUS_SUCCESS);
    let query_only = f.u64_at(out);
    assert_eq!(lowbox(&mut f, query_only, &good, &[]).0, STATUS_ACCESS_DENIED);
    assert_eq!(lowbox(&mut f, 0x7777, &good, &[]).0, STATUS_INVALID_HANDLE);
    // A container's token is not the source of another container.
    let (_, container) = lowbox(&mut f, src, &good, &[]);
    assert_eq!(lowbox(&mut f, container, &good, &[]).0, STATUS_INVALID_PARAMETER);
}
