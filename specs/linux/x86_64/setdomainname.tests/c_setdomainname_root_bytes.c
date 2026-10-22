/* The name is stored byte for byte, with no validation: bytes after len
 * up to 65 become NUL (a shorter name after a longer one leaves no
 * trace), an embedded NUL is kept with the bytes after it (uname shows
 * them past the NUL), and any byte values (0xff, 0x01, "/", " ", "\n")
 * are accepted.
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
    char n[65], big[64];
    memset(big, 0x61, sizeof big);
    expect("set_64", setdomainname(big, 64), 0);
    expect("set_xy", setdomainname("xy", 2), 0);
    domainname(n);
    int tail_zero = n[0] == 0x78 && n[1] == 0x79;
    for (int i = 2; i < 65; i++)
        if (n[i]) tail_zero = 0;
    check("tail_cleared", tail_zero, (i64)str_len(n));

    expect("set_embedded_nul", setdomainname("ab\0cd", 5), 0);
    domainname(n);
    int kept = n[0] == 0x61 && n[1] == 0x62 && n[2] == 0 && n[3] == 0x63 && n[4] == 0x64;
    for (int i = 5; i < 65; i++)
        if (n[i]) kept = 0;
    check("embedded_nul_kept", kept, (i64)str_len(n));

    const char odd[8] = { (char)0xff, 0x01, 0x2f, 0x20, 0x0a, 0x2e, 0x7f, 0x2d };
    expect("set_odd_bytes", setdomainname(odd, 8), 0);
    domainname(n);
    check("odd_bytes_verbatim", mem_eq(n, odd, 8) && n[8] == 0, (i64)str_len(n));
    expect("set_leading_nul", setdomainname("\0zz", 3), 0);
    domainname(n);
    check("leading_nul_kept", n[0] == 0 && n[1] == 0x7a && n[2] == 0x7a && n[3] == 0, (i64)n[1]);
    return finish();
}

int test_main(void) { return in_new_uts(body); }
