# mach_msg(msg, option, send_size, rcv_size, rcv_name, timeout, notify):
# Mach trap 31 (mach_msg_trap). Arguments in rdi, rsi, rdx, rcx -> r10,
# r8, r9; the seventh stays on the stack above the return address, where
# the kernel reads it. Written for Poly's tests (ADR 0238).
    .text
    .globl _mach_msg
    .p2align 4
_mach_msg:
    movq %rcx, %r10
    movl $0x100001f, %eax
    syscall
    ret
