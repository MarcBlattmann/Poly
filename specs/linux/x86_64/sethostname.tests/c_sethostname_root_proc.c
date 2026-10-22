/* /proc/sys/kernel/hostname shows the name set by sethostname, up to
 * its first NUL, followed by a newline: "proc-host\n"; after the empty
 * name just "\n"; after "ab\0cd" it reads "ab\n".
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

static i64 read_proc(char *buf, size_t n) {
    memset(buf, 0, n);
    i64 fd = a_open("/proc/sys/kernel/hostname", O_RDONLY, 0);
    if (fd < 0) return fd;
    i64 r = a_read(fd, buf, n - 1);
    a_close(fd);
    return r;
}

int body(void) {
    char p[128];
    expect("set_name", sethostname("proc-host", 9), 0);
    i64 r = read_proc(p, sizeof p);
    check("proc_name", r == 10 && str_eq(p, "proc-host\n"), r);
    expect("set_empty", sethostname("", 0), 0);
    r = read_proc(p, sizeof p);
    check("proc_empty", r == 1 && str_eq(p, "\n"), r);
    expect("set_embedded_nul", sethostname("ab\0cd", 5), 0);
    r = read_proc(p, sizeof p);
    check("proc_up_to_nul", r == 3 && str_eq(p, "ab\n"), r);
    return finish();
}

int test_main(void) { return in_new_uts(body); }
