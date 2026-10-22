/* setdomainname(name, len) as root, in a private UTS namespace: returns 0
 * and uname then reports exactly that name as domainname; a second call
 * replaces it; the name may live in read-only memory; the other uname
 * fields (sysname, release, version, machine, nodename) do not change.
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
    struct a_utsname before, after;
    memset(&before, 0, sizeof before);
    memset(&after, 0, sizeof after);
    syscall1(SYS_uname, &before);
    char n[65];
    expect("set_first", setdomainname("poly-uts-test", 13), 0);
    domainname(n);
    check("domainname_first", str_eq(n, "poly-uts-test"), (i64)str_len(n));
    expect("set_second", setdomainname("second.example", 14), 0);
    domainname(n);
    check("domainname_second", str_eq(n, "second.example"), (i64)str_len(n));
    expect("set_shorter", setdomainname("s", 1), 0);
    domainname(n);
    check("domainname_shorter", str_eq(n, "s"), (i64)str_len(n));
    syscall1(SYS_uname, &after);
    check("sysname_same", mem_eq(before.sysname, after.sysname, 65), 0);
    check("release_same", mem_eq(before.release, after.release, 65), 0);
    check("version_same", mem_eq(before.version, after.version, 65), 0);
    check("machine_same", mem_eq(before.machine, after.machine, 65), 0);
    check("nodename_same", mem_eq(before.nodename, after.nodename, 65), 0);
    return finish();
}

int test_main(void) { return in_new_uts(body); }
