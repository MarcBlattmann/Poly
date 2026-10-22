/* A name that cannot be read in full fails with EFAULT and changes
 * nothing (no partial name): NULL, an unmapped page, a PROT_NONE page, a
 * kernel or non-canonical address, and a name whose last bytes run into
 * an inaccessible page. The length check comes first: NULL with len 65
 * is EINVAL.
 * poly-diff: needs=root (only root may set the host name)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sethostname 170
#define SYS_unshare 272
/* linux/sched.h */
#define CLONE_NEWUTS 0x04000000

static i64 sethostname(const void *name, i64 len) { return syscall2(SYS_sethostname, name, len); }

static A_UNUSED void nodename(char out[65]) {
    struct a_utsname u;
    memset(&u, 0x55, sizeof u);
    syscall1(SYS_uname, &u);
    memcpy(out, u.nodename, 65);
}

/* Run `body` in a child that has its own UTS namespace, so the real host
 * name is never touched. The child's output is the test's output. If the
 * namespace cannot be made, the child reports that and never calls
 * sethostname. */
static int in_new_uts(int (*body)(void)) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        i64 r = syscall1(SYS_unshare, CLONE_NEWUTS);
        expect("unshare_newuts", r, 0);
        if (r != 0) syscall1(SYS_exit_group, finish());
        syscall1(SYS_exit_group, body());
    }
    i32 status = -1;
    if (pid < 0 || syscall4(SYS_wait4, pid, &status, 0, 0) != pid) return 1;
    return A_WIFEXITED(status) ? A_WEXITSTATUS(status) : 1;
}

int body(void) {
    char n[65];
    expect("set_known", sethostname("known", 5), 0);
    char *m = (char *)syscall6(SYS_mmap, 0, 3 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", (i64)m > 0, (i64)m);
    if ((i64)m <= 0) return finish();
    syscall2(SYS_munmap, m + 2 * A_PAGE, A_PAGE);
    syscall3(SYS_mprotect, m + A_PAGE, A_PAGE, PROT_NONE);
    memcpy(m + A_PAGE - 3, "abc", 3);

    expect("null_efault", sethostname(0, 5), -EFAULT);
    expect("low_address_efault", sethostname((void *)16, 1), -EFAULT);
    expect("unmapped_efault", sethostname(m + 2 * A_PAGE, 8), -EFAULT);
    expect("prot_none_efault", sethostname(m + A_PAGE, 8), -EFAULT);
    expect("kernel_address_efault", sethostname((void *)0xffffffff81000000UL, 4), -EFAULT);
    expect("noncanonical_efault", sethostname((void *)0x0000800000000000UL, 4), -EFAULT);
    expect("runs_into_guard_efault", sethostname(m + A_PAGE - 3, 5), -EFAULT);
    expect("runs_into_guard_64_efault", sethostname(m + A_PAGE - 3, 64), -EFAULT);
    nodename(n);
    check("unchanged_after_efault", str_eq(n, "known"), (i64)str_len(n));
    expect("null_len_65_einval", sethostname(0, 65), -EINVAL);
    expect("bad_len_negative_einval", sethostname(0, -1), -EINVAL);
    expect("fits_before_guard", sethostname(m + A_PAGE - 3, 3), 0);
    nodename(n);
    check("nodename_abc", str_eq(n, "abc"), (i64)str_len(n));
    return finish();
}

int test_main(void) { return in_new_uts(body); }
