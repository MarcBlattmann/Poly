/* len 0 is allowed and sets the empty domain name: uname domainname is then
 * 65 NUL bytes. With len 0 the name is not read at all, so NULL or an
 * unmapped address also succeed.
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
    expect("len_0", setdomainname("ignored", 0), 0);
    domainname(n);
    int zero = 1;
    for (int i = 0; i < 65; i++)
        if (n[i]) zero = 0;
    check("domainname_all_nul", zero, (i64)str_len(n));
    expect("set_known_again", setdomainname("known", 5), 0);
    expect("null_len_0", setdomainname(0, 0), 0);
    domainname(n);
    check("domainname_empty_after_null", n[0] == 0, (i64)str_len(n));
    i64 gone = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, gone, A_PAGE);
    expect("set_known_third", setdomainname("known", 5), 0);
    expect("unmapped_len_0", setdomainname((void *)gone, 0), 0);
    domainname(n);
    check("domainname_empty_after_unmapped", n[0] == 0, (i64)str_len(n));
    return finish();
}

int test_main(void) { return in_new_uts(body); }
