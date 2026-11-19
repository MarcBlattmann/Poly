//! Calling a function whose arguments are known only at run time (`ITypeInfo::Invoke`,
//! `DispCallFunc`, the marshaller's stubs), and the entries of proxy vtables, which receive a call
//! of any shape. x64 only (Learn: "x64 calling convention": the first four arguments in RCX, RDX,
//! R8, R9 or XMM0..3 by position, the rest on the stack above 32 bytes of shadow space, 16-byte
//! alignment at the call, RSI/RDI preserved). Other architectures have no generic call: the type
//! library marshaller is not available there.

/// Call `f` with `args` (each one 8-byte slot, in order; a `double` as its bits). The integer
/// result; the floating result's bits go to `*fp` when `fp` is not null.
///
/// # Safety
/// `f` is a function of the `system` convention taking `args.len()` slots.
#[cfg(target_arch = "x86_64")]
pub(crate) unsafe fn call(f: usize, args: &[u64], fp: *mut u64) -> u64 {
    // SAFETY: the caller's function and arguments; `call_x64` copies `n` slots from `args`.
    unsafe { call_x64(f, args.as_ptr(), args.len(), fp) }
}

#[cfg(not(target_arch = "x86_64"))]
pub(crate) unsafe fn call(_f: usize, _args: &[u64], _fp: *mut u64) -> u64 {
    crate::com::E_NOTIMPL as u32 as u64
}

/// Whether generic calls work here.
pub(crate) const AVAILABLE: bool = cfg!(target_arch = "x86_64");

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
unsafe extern "system" fn call_x64(f: usize, args: *const u64, n: usize, fp: *mut u64) -> u64 {
    core::arch::naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "push rsi",
        "push rdi",
        // home f and fp in our caller's shadow space
        "mov [rbp+16], rcx",
        "mov [rbp+40], r9",
        "mov rsi, rdx",
        "mov rcx, r8",
        "mov rax, r8",
        "cmp rax, 4",
        "jae 2f",
        "mov rax, 4",
        "2:",
        "shl rax, 3",
        "add rax, 15",
        "and rax, -16",
        "sub rsp, rax",
        "mov rdi, rsp",
        "cld",
        "rep movsq",
        "mov rcx, [rsp]",
        "mov rdx, [rsp+8]",
        "mov r8, [rsp+16]",
        "mov r9, [rsp+24]",
        "movq xmm0, rcx",
        "movq xmm1, rdx",
        "movq xmm2, r8",
        "movq xmm3, r9",
        "call qword ptr [rbp+16]",
        "mov r9, [rbp+40]",
        "test r9, r9",
        "jz 3f",
        "movq [r9], xmm0",
        "3:",
        "lea rsp, [rbp-16]",
        "pop rdi",
        "pop rsi",
        "pop rbp",
        "ret",
    )
}

/// What a proxy's vtable entry hands on: the slot, the arguments as they were passed (`args[0]` is
/// `this`, then every argument's 8-byte slot in order) and the bits of XMM1..XMM3 (where the
/// second to fourth arguments are when they are floating point).
#[cfg(target_arch = "x86_64")]
pub(crate) type Dispatch = unsafe extern "system" fn(args: *const u64, slot: u32, xmm: *const u64) -> u64;

/// The common part of every proxy vtable entry: spill the register arguments next to the stack
/// ones and call [`crate::marshal::proxy::dispatch`].
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
unsafe extern "system" fn entry_common() {
    core::arch::naked_asm!(
        "mov [rsp+8], rcx",
        "mov [rsp+16], rdx",
        "mov [rsp+24], r8",
        "mov [rsp+32], r9",
        "sub rsp, 72",
        "movsd [rsp+32], xmm1",
        "movsd [rsp+40], xmm2",
        "movsd [rsp+48], xmm3",
        "lea rcx, [rsp+80]",
        "mov edx, eax",
        "lea r8, [rsp+32]",
        "call {d}",
        "add rsp, 72",
        "ret",
        d = sym crate::marshal::proxy::dispatch,
    )
}

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
unsafe extern "system" fn entry<const K: u32>() {
    core::arch::naked_asm!("mov eax, {k}", "jmp {c}", k = const K, c = sym entry_common)
}

/// The most methods a marshalled interface may have (vtable slots).
pub(crate) const MAX_SLOTS: usize = 128;

macro_rules! entries {
    ($($k:literal)*) => {
        /// The proxy vtable entries, one per slot (slots 0..2 are replaced by the proxy's own
        /// `IUnknown` methods).
        #[cfg(target_arch = "x86_64")]
        pub(crate) static ENTRIES: [unsafe extern "system" fn(); MAX_SLOTS] = [$(entry::<$k>),*];
    };
}

entries!(
    0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31
    32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63
    64 65 66 67 68 69 70 71 72 73 74 75 76 77 78 79 80 81 82 83 84 85 86 87 88 89 90 91 92 93 94 95
    96 97 98 99 100 101 102 103 104 105 106 107 108 109 110 111 112 113 114 115 116 117 118 119 120 121 122 123 124 125 126 127
);
