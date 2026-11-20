# Poly's dyld start code for x86_64 (see src/start.rs). Assembled once with
# `clang -arch x86_64 -c` (black box) by gen.sh; the bytes are embedded in
# start.rs. Written for Poly; no Apple or third-party source was read.
#
# Entry: rsp -> [main header address, argc, argv..., 0, envp..., 0,
# apple..., 0]. The data block is the page after this code page:
#   0  main (LC_MAIN) or the program counter of LC_UNIXTHREAD
#   8  mode: 0 = call main(argc, argv, envp, apple) and exit with its result;
#            1 = jump to the program counter with rsp at argc
#  16  exit function (0: the raw exit system call)
#  24  number of initializers
#  32  the initializers, called with (argc, argv, envp, apple, data block)
#      (the image table libdyld reads follows them: src/start.rs)
    .text
    .globl _poly_dyld_start
_poly_dyld_start:
    leaq -7(%rip), %rbx          # rbx = this code page
    addq $0x1000, %rbx           # rbx = the data block
    movq %rsp, %r12              # r12 -> main header address
    movq 8(%r12), %r13           # argc
    leaq 16(%r12), %r14          # argv
    leaq 8(%r14,%r13,8), %r15    # envp
    movq %r15, %rbp
1:  cmpq $0, (%rbp)
    je 2f
    addq $8, %rbp
    jmp 1b
2:  addq $8, %rbp                # apple
    andq $-16, %rsp
    pushq $0
    pushq $0                     # (%rsp): index of the next initializer
3:  movq (%rsp), %rax
    cmpq 24(%rbx), %rax
    jae 4f
    movq 32(%rbx,%rax,8), %rax
    incq (%rsp)
    movq %r13, %rdi
    movq %r14, %rsi
    movq %r15, %rdx
    movq %rbp, %rcx
    movq %rbx, %r8
    callq *%rax
    jmp 3b
4:  cmpq $1, 8(%rbx)
    jne 5f
    leaq 8(%r12), %rsp           # rsp -> argc
    jmpq *(%rbx)
5:  movq %r13, %rdi
    movq %r14, %rsi
    movq %r15, %rdx
    movq %rbp, %rcx
    callq *(%rbx)
    movl %eax, %edi
    movq 16(%rbx), %rax
    testq %rax, %rax
    je 6f
    callq *%rax
6:  movl $0x2000001, %eax
    syscall
    ud2

# The thread-local variable thunk every descriptor's first word points to:
# rdi -> descriptor {thunk, key, offset}; the key is the address of the
# image's thread-local block. Returns key + offset in rax; nothing else
# changes.
    .p2align 4
    .globl _poly_tlv_get
_poly_tlv_get:
    movq 8(%rdi), %rax
    addq 16(%rdi), %rax
    retq
