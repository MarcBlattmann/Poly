/* The longest name is 64 bytes (__NEW_UTS_LEN): len 64 succeeds and
 * nodename is those 64 bytes followed by a NUL; len 65, 66, 255 and 4096
 * fail with EINVAL and leave the name as it was.
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

static char big[4096];

int body(void) {
    char n[65];
    memset(big, 0x71, sizeof big);
    expect("len_64", sethostname(big, 64), 0);
    nodename(n);
    int all_q = 1;
    for (int i = 0; i < 64; i++)
        if (n[i] != (char)0x71) all_q = 0;
    check("nodename_64_bytes", all_q && n[64] == 0, (i64)str_len(n));
    expect("set_known", sethostname("known", 5), 0);
    expect("len_65_einval", sethostname(big, 65), -EINVAL);
    expect("len_66_einval", sethostname(big, 66), -EINVAL);
    expect("len_255_einval", sethostname(big, 255), -EINVAL);
    expect("len_4096_einval", sethostname(big, 4096), -EINVAL);
    nodename(n);
    check("unchanged_after_einval", str_eq(n, "known"), (i64)str_len(n));
    return finish();
}

int test_main(void) { return in_new_uts(body); }
