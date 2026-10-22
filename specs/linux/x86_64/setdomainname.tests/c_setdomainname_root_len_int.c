/* len is a 32-bit signed int: the upper half of the register is
 * ignored. -1, 0x80000000 and 0xffffffff are negative and fail with
 * EINVAL; (1<<32)|4 is 4 and sets 4 bytes; (1<<32)|65 is 65 (EINVAL);
 * 1<<32 is 0 and sets the empty name.
 * poly-diff: needs=root (only root may set the domain name)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_setdomainname 171
#define SYS_unshare 272
/* linux/sched.h */
#define CLONE_NEWUTS 0x04000000

static i64 setdomainname(const void *name, i64 len) { return syscall2(SYS_setdomainname, name, len); }

static A_UNUSED void domainname(char out[65]) {
    struct a_utsname u;
    memset(&u, 0x55, sizeof u);
    syscall1(SYS_uname, &u);
    memcpy(out, u.domainname, 65);
}

/* Run `body` in a child that has its own UTS namespace, so the real domain
 * name is never touched. The child's output is the test's output. If the
 * namespace cannot be made, the child reports that and never calls
 * setdomainname. */
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
    expect("set_known", setdomainname("known", 5), 0);
    expect("len_minus_1_einval", setdomainname("abcdef", -1), -EINVAL);
    expect("len_int_min_einval", setdomainname("abcdef", 0x80000000L), -EINVAL);
    expect("len_u32_max_einval", setdomainname("abcdef", 0xffffffffL), -EINVAL);
    expect("len_hi_65_einval", setdomainname("abcdef", (1L << 32) | 65), -EINVAL);
    domainname(n);
    check("unchanged_after_einval", str_eq(n, "known"), (i64)str_len(n));
    expect("len_hi_4", setdomainname("abcdef", (1L << 32) | 4), 0);
    domainname(n);
    check("domainname_abcd", str_eq(n, "abcd"), (i64)str_len(n));
    expect("len_hi_0", setdomainname("abcdef", 1L << 32), 0);
    domainname(n);
    check("domainname_empty", n[0] == 0, (i64)str_len(n));
    return finish();
}

int test_main(void) { return in_new_uts(body); }
