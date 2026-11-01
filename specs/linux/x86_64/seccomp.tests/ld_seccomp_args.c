/* A seccomp filter sees struct seccomp_data: nr (offset 0), arch
 * (offset 4, AUDIT_ARCH_X86_64 = 0xc000003e for native calls) and the
 * six arguments as 64-bit values from offset 16 (low half first). A
 * filter failing dup(1) with EPERM leaves dup(0) alone; it loads only the
 * low 32 bits of args[0], so dup((1 << 32) | 1) matches too. A filter
 * that fails every call whose arch is not x86-64 with EACCES does not
 * affect native calls.
 */
#include "a_sys.h"

#define SYS_prctl 157
#define SYS_seccomp 317
#define PR_SET_NO_NEW_PRIVS 38 /* linux/prctl.h */
/* linux/seccomp.h, linux/filter.h, linux/bpf_common.h, linux/audit.h */
#define SECCOMP_SET_MODE_FILTER 1
#define SECCOMP_RET_ERRNO 0x00050000U
#define SECCOMP_RET_ALLOW 0x7fff0000U
#define BPF_LD_W_ABS 0x20
#define BPF_JEQ_K 0x15
#define BPF_RET_K 0x06
#define AUDIT_ARCH_X86_64 0xc000003eU

struct sock_filter {
    u16 code;
    u8 jt, jf;
    u32 k;
};
struct sock_fprog {
    u16 len;
    struct sock_filter *filter;
};

static struct sock_filter dup_filter[6] = {
    {BPF_LD_W_ABS, 0, 0, 0},          /* nr */
    {BPF_JEQ_K, 0, 3, SYS_dup},
    {BPF_LD_W_ABS, 0, 0, 16},         /* args[0], low 32 bits */
    {BPF_JEQ_K, 0, 1, 1},
    {BPF_RET_K, 0, 0, SECCOMP_RET_ERRNO | EPERM},
    {BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW},
};
static struct sock_filter arch_filter[4] = {
    {BPF_LD_W_ABS, 0, 0, 4}, /* arch */
    {BPF_JEQ_K, 1, 0, AUDIT_ARCH_X86_64},
    {BPF_RET_K, 0, 0, SECCOMP_RET_ERRNO | EACCES},
    {BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW},
};
static struct sock_fprog p1 = {6, dup_filter}, p2 = {4, arch_filter};

static int child(void) {
    expect("set_nnp", syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    expect("install_arch_filter", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &p2), 0);
    check("native_calls_pass", syscall0(SYS_getuid) >= 0, 0);
    expect("install_dup_filter", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &p1), 0);
    expect("dup1_eperm", syscall1(SYS_dup, 1), -EPERM);
    expect("dup0_works", syscall1(SYS_dup, 0), 3);
    expect("dup_high_bits_eperm", syscall1(SYS_dup, (1L << 32) | 1), -EPERM);
    expect("dup2_1_unfiltered", syscall2(SYS_dup2, 1, 7), 7);
    expect("dup_of_7_works", syscall1(SYS_dup, 7), 4);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, child());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    return finish();
}
