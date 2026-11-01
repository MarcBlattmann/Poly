/* SECCOMP_SET_MODE_FILTER argument checks (with no_new_privs set): an
 * unknown flag bit is EINVAL (checked first, even with a NULL program);
 * an unreadable struct sock_fprog or filter array is EFAULT; a program
 * of 0 or more than 4096 instructions, one whose last instruction is not
 * a return, a load outside struct seccomp_data (offset 64) or not 4-byte
 * aligned, an unknown opcode or a jump past the end is EINVAL. Nothing
 * is installed by a failed call. SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV
 * without NEW_LISTENER, and TSYNC together with NEW_LISTENER (without
 * TSYNC_ESRCH), are EINVAL. NEW_LISTENER returns a new descriptor (the
 * lowest free, O_RDWR, close-on-exec; /proc/self/fd names it
 * "anon_inode:seccomp notify"); a second listener anywhere in the
 * process's filter chain is EBUSY. LOG, SPEC_ALLOW, TSYNC and
 * TSYNC_ESRCH alone are accepted. SECCOMP_SET_MODE_STRICT once filters
 * are installed is EINVAL.
 */
#include "a_sys.h"

#define SYS_prctl 157
#define SYS_seccomp 317
#define PR_SET_NO_NEW_PRIVS 38 /* linux/prctl.h */
/* linux/seccomp.h, linux/filter.h, linux/bpf_common.h */
#define SECCOMP_SET_MODE_STRICT 0
#define SECCOMP_SET_MODE_FILTER 1
#define SECCOMP_FILTER_FLAG_TSYNC 1
#define SECCOMP_FILTER_FLAG_LOG 2
#define SECCOMP_FILTER_FLAG_SPEC_ALLOW 4
#define SECCOMP_FILTER_FLAG_NEW_LISTENER 8
#define SECCOMP_FILTER_FLAG_TSYNC_ESRCH 16
#define SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV 32
#define SECCOMP_RET_ALLOW 0x7fff0000U
#define BPF_LD_W_ABS 0x20
#define BPF_JEQ_K 0x15
#define BPF_RET_K 0x06

struct sock_filter {
    u16 code;
    u8 jt, jf;
    u32 k;
};
struct sock_fprog {
    u16 len;
    struct sock_filter *filter;
};

static struct sock_filter insns[8];
static struct sock_fprog prog;
static const struct sock_filter RET_ALLOW = {BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW};

static i64 load(u16 len, u64 flags) {
    prog.len = len;
    prog.filter = insns;
    return syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, flags, &prog);
}

static i64 allow_all(u64 flags) {
    insns[0] = RET_ALLOW;
    return load(1, flags);
}

static int child(void) {
    expect("set_nnp", syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    insns[0] = RET_ALLOW;
    prog.len = 1;
    prog.filter = insns;
    expect("flag_0x40_einval", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0x40, &prog), -EINVAL);
    expect("flag_0x40_null_einval", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0x40, 0), -EINVAL);
    expect("null_prog_efault", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, 0), -EFAULT);
    expect("low_prog_efault", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, 8), -EFAULT);
    prog.filter = (struct sock_filter *)8;
    expect("bad_filter_array_efault", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog), -EFAULT);

    expect("len0_einval", load(0, 0), -EINVAL);
    expect("len4097_einval", load(4097, 0), -EINVAL);
    insns[0] = (struct sock_filter){BPF_LD_W_ABS, 0, 0, 0};
    expect("no_return_einval", load(1, 0), -EINVAL);
    insns[1] = RET_ALLOW;
    insns[0] = (struct sock_filter){BPF_LD_W_ABS, 0, 0, 64};
    expect("load_past_data_einval", load(2, 0), -EINVAL);
    insns[0] = (struct sock_filter){BPF_LD_W_ABS, 0, 0, 2};
    expect("load_unaligned_einval", load(2, 0), -EINVAL);
    insns[0] = (struct sock_filter){BPF_LD_W_ABS, 0, 0, 60};
    expect("load_last_word_ok", load(2, 0), 0);
    insns[0] = (struct sock_filter){0xff, 0, 0, 0};
    expect("bad_opcode_einval", load(2, 0), -EINVAL);
    insns[0] = (struct sock_filter){BPF_JEQ_K, 5, 5, 0};
    expect("jump_past_end_einval", load(2, 0), -EINVAL);

    expect("killable_recv_alone_einval", allow_all(SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV), -EINVAL);
    expect("tsync_listener_einval", allow_all(SECCOMP_FILTER_FLAG_TSYNC | SECCOMP_FILTER_FLAG_NEW_LISTENER), -EINVAL);
    expect("flag_log_ok", allow_all(SECCOMP_FILTER_FLAG_LOG), 0);
    expect("flag_spec_allow_ok", allow_all(SECCOMP_FILTER_FLAG_SPEC_ALLOW), 0);
    expect("flag_tsync_ok", allow_all(SECCOMP_FILTER_FLAG_TSYNC), 0);
    expect("flag_tsync_esrch_ok", allow_all(SECCOMP_FILTER_FLAG_TSYNC_ESRCH), 0);

    i64 l = allow_all(SECCOMP_FILTER_FLAG_NEW_LISTENER);
    expect("listener_fd_3", l, 3);
    expect("listener_cloexec", syscall2(SYS_fcntl, l, F_GETFD), FD_CLOEXEC);
    expect("listener_rdwr", syscall2(SYS_fcntl, l, F_GETFL) & O_ACCMODE, O_RDWR);
    char link[64];
    memset(link, 0, sizeof link);
    syscall3(SYS_readlink, "/proc/self/fd/3", link, sizeof link - 1);
    check("listener_link", str_eq(link, "anon_inode:seccomp notify"), 0);
    expect("second_listener_ebusy", allow_all(SECCOMP_FILTER_FLAG_NEW_LISTENER), -EBUSY);
    expect("listener_killable_ebusy",
           allow_all(SECCOMP_FILTER_FLAG_NEW_LISTENER | SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV), -EBUSY);
    expect("strict_after_filter_einval", syscall3(SYS_seccomp, SECCOMP_SET_MODE_STRICT, 0, 0), -EINVAL);
    check("still_running", syscall0(SYS_getpid) > 0, 0);
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
