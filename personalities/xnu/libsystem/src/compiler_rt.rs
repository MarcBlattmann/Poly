//! libcompiler_rt (ADR 0257 addendum): `___chkstk_darwin`, which the
//! compiler calls before a large stack frame to touch its pages in order.
//! Poly maps a thread's whole stack when it makes it, so there is nothing
//! to touch: it returns at once, every register as it was.

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(".globl ___chkstk_darwin", ".p2align 4", "___chkstk_darwin:", "    ret");

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(".globl ___chkstk_darwin", ".p2align 2", "___chkstk_darwin:", "    ret");
