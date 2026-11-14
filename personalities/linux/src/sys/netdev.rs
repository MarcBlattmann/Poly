//! The interface requests of ioctl(2) on a socket (netdevice(7)):
//! `SIOCGIFCONF` and the `SIOCGIF*` / `SIOCSIF*` requests on a `struct
//! ifreq`. They act on the devices of the network namespace the socket
//! was made in (ADR 0178), whatever its family: sandboxes use an
//! `AF_INET` datagram socket to bring the loopback device of their new
//! namespace up (`SIOCSIFFLAGS`).
//!
//! The answers are [`crate::net::dev`]'s; this file copies the structures
//! and checks the privilege: a request that changes a device needs
//! `CAP_NET_ADMIN` in the user namespace that owns the network namespace
//! (`EPERM`, checked before the device is looked up), while the getters
//! work for anybody.

use crate::cred::cap;
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::net::dev::{self, IFCONF_SIZE, IFREQ_SIZE, IfAnswer};
use crate::signal::deliver::with_own;

/// Answer `request` on `fd` if it is an interface request and `fd` a
/// socket; `None` otherwise (the caller goes on as for any other ioctl).
pub fn ioctl(k: &mut dyn LinuxKernel, fd: i32, request: u32, arg: UserPtr) -> Option<SysResult> {
    if request != dev::SIOCGIFCONF && !dev::is_ifreq_request(request) {
        return None;
    }
    let ns = match k.net()?.netns(fd) {
        Err(Errno::ENOTSOCK) => return None,
        Err(e) => return Some(Err(e)),
        Ok(ns) => ns,
    };
    Some(run(k, &ns, request, arg))
}

fn run(
    k: &mut dyn LinuxKernel,
    ns: &Option<alloc::sync::Arc<crate::ns::NetNs>>,
    request: u32,
    arg: UserPtr,
) -> SysResult {
    if request == dev::SIOCGIFCONF {
        // struct ifconf: int ifc_len, padding, char *ifc_buf.
        let mut c = [0u8; IFCONF_SIZE];
        k.copy_from_user(arg, &mut c)?;
        let len = i32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        let buf = u64::from_le_bytes([c[8], c[9], c[10], c[11], c[12], c[13], c[14], c[15]]);
        if len < 0 {
            return Err(Errno::EINVAL);
        }
        let ifs = k.net().ok_or(Errno::ENOSYS)?.interfaces(ns);
        let out = dev::ifconf(&ifs, (buf != 0).then_some(len as usize));
        if buf != 0 {
            k.copy_to_user(UserPtr(buf), &out)?;
        }
        c[0..4].copy_from_slice(&(out.len() as i32).to_le_bytes());
        k.copy_to_user(arg, &c)?;
        return Ok(0);
    }
    let mut req = [0u8; IFREQ_SIZE];
    k.copy_from_user(arg, &mut req)?;
    if dev::is_set_request(request) {
        let tid = k.tid();
        let creds = with_own(k, |p| p.thread(tid).creds.clone());
        if !crate::ns::user::ns_capable(&creds, &crate::ns::net_owner(ns), cap::NET_ADMIN) {
            return Err(Errno::EPERM);
        }
    }
    let net = k.net().ok_or(Errno::ENOSYS)?;
    let ifs = net.interfaces(ns);
    match dev::ifreq(request, &mut req, &ifs)? {
        IfAnswer::Filled => k.copy_to_user(arg, &req)?,
        IfAnswer::SetUp { index, up } => net.set_link_up(ns, index, up)?,
    }
    Ok(0)
}
