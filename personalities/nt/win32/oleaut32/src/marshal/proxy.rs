//! Interface proxies made from type information.

/// Every proxy vtable entry of slot 3 and above comes here.
#[cfg(target_arch = "x86_64")]
pub(crate) unsafe extern "system" fn dispatch(_args: *const u64, _slot: u32, _xmm: *const u64) -> u64 {
    crate::com::E_NOTIMPL as u32 as u64
}
