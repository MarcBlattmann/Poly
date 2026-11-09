.text
.globl PolyUserCall
PolyUserCall:
    mov r10, rcx
    mov eax, 4096
    syscall
    ret

.globl PolyUserCommitSurface
PolyUserCommitSurface:
    mov r10, rcx
    mov eax, 4097
    syscall
    ret

