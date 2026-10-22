/* The domain name and the host name are separate: setdomainname leaves
 * uname's nodename and /proc/sys/kernel/hostname alone, sethostname
 * leaves domainname and /proc/sys/kernel/domainname alone, and a
 * 64-byte domain name reads back from /proc as those 64 bytes and "\n".
 * poly-diff: needs=root (only root may set the domain name)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sethostname 170
#define SYS_setdomainname 171
#define SYS_unshare 272
/* linux/sched.h */
#define CLONE_NEWUTS 0x04000000

static i64 setdomainname(const void *name, i64 len) { return syscall2(SYS_setdomainname, name, len); }

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

static i64 read_proc(const char *path, char *buf, size_t n) {
    memset(buf, 0, n);
    i64 fd = a_open(path, O_RDONLY, 0);
    if (fd < 0) return fd;
    i64 r = a_read(fd, buf, n - 1);
    a_close(fd);
    return r;
}

int body(void) {
    struct a_utsname u;
    char p[128];
    expect("set_host", syscall2(SYS_sethostname, "the-host", 8), 0);
    expect("set_domain", setdomainname("the.domain", 10), 0);
    memset(&u, 0x55, sizeof u);
    syscall1(SYS_uname, &u);
    check("nodename_is_host", str_eq(u.nodename, "the-host"), (i64)str_len(u.nodename));
    check("domainname_is_domain", str_eq(u.domainname, "the.domain"), (i64)str_len(u.domainname));

    expect("set_domain_again", setdomainname("other.domain", 12), 0);
    memset(&u, 0x55, sizeof u);
    syscall1(SYS_uname, &u);
    check("nodename_kept", str_eq(u.nodename, "the-host"), (i64)str_len(u.nodename));
    i64 r = read_proc("/proc/sys/kernel/hostname", p, sizeof p);
    check("proc_hostname_kept", r == 9 && str_eq(p, "the-host\n"), r);

    expect("set_host_again", syscall2(SYS_sethostname, "new-host", 8), 0);
    memset(&u, 0x55, sizeof u);
    syscall1(SYS_uname, &u);
    check("domainname_kept", str_eq(u.domainname, "other.domain"), (i64)str_len(u.domainname));
    r = read_proc("/proc/sys/kernel/domainname", p, sizeof p);
    check("proc_domainname_kept", r == 13 && str_eq(p, "other.domain\n"), r);

    char big[64];
    memset(big, 0x64, sizeof big);
    expect("set_domain_64", setdomainname(big, 64), 0);
    r = read_proc("/proc/sys/kernel/domainname", p, sizeof p);
    check("proc_domainname_64", r == 65 && mem_eq(p, big, 64) && p[64] == 0x0a, r);
    return finish();
}

int test_main(void) { return in_new_uts(body); }
