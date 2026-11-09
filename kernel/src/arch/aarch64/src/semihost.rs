//! Arm semihosting (Arm's published "Semihosting for AArch32 and AArch64"
//! specification): the guest executes `HLT 0xF000`
//! with the operation number in `w0` and a pointer to a parameter block in
//! `x1`; the debugger or emulator acts on it. The stub uses one operation,
//! `SYS_EXIT`, to end QEMU with a status.

/// `SYS_EXIT`: report an exception to the debugger; the stub's way of
/// stopping with a status.
pub const SYS_EXIT: u64 = 0x18;

/// `ADP_Stopped_ApplicationExit`: the reason code of a normal exit. In
/// AArch64 `SYS_EXIT`'s parameter block is two 64-bit words, the reason and
/// the exit status ("subcode") that the host sees.
pub const ADP_STOPPED_APPLICATION_EXIT: u64 = 0x2_0026;

/// The `SYS_EXIT` parameter block (AArch64 layout) for exit status `code`.
pub const fn exit_block(code: u32) -> [u64; 2] {
    [ADP_STOPPED_APPLICATION_EXIT, code as u64]
}

/// The stub's exit statuses; `cargo xtask aarch64 test` checks them.
pub mod status {
    /// The self-test passed.
    pub const PASS: u32 = 0;
    /// A self-test check failed.
    pub const FAIL: u32 = 1;
    /// The kernel panicked.
    pub const PANIC: u32 = 2;
    /// An exception nothing expected.
    pub const EXCEPTION: u32 = 3;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_block_is_reason_then_status() {
        assert_eq!(exit_block(0), [0x20026, 0]);
        assert_eq!(exit_block(2), [0x20026, 2]);
    }
}
