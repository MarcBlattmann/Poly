/* Only `len` bytes of the name are used and no terminating NUL is
 * needed: "abcdef" with len 3 sets "abc"; a name of 5 bytes that ends
 * right before an inaccessible page works with len 5 (nothing past len is
 * read).
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
    expect("len_3", setdomainname("abcdef", 3), 0);
    domainname(n);
    check("domainname_abc", str_eq(n, "abc"), (i64)str_len(n));
    expect("len_1", setdomainname("zyx", 1), 0);
    domainname(n);
    check("domainname_z", str_eq(n, "z"), (i64)str_len(n));

    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", (i64)m > 0, (i64)m);
    if ((i64)m <= 0) return finish();
    syscall3(SYS_mprotect, m + A_PAGE, A_PAGE, PROT_NONE);
    char *edge = m + A_PAGE - 5;
    memcpy(edge, "edged", 5);
    expect("unterminated_at_page_end", setdomainname(edge, 5), 0);
    domainname(n);
    check("domainname_edged", str_eq(n, "edged"), (i64)str_len(n));
    return finish();
}

int test_main(void) { return in_new_uts(body); }
