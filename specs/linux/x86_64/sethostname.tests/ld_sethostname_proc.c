/* The node name set by sethostname is what /proc/sys/kernel/hostname
 * reads (the name and a newline) in the same UTS namespace, and writing
 * that file sets the name uname shows (a trailing newline is dropped).
 * Each UTS namespace has its own name: the test's namespace keeps its
 * own while a child's new namespace changes.
 * poly-diff: needs=root (a new UTS namespace needs CAP_SYS_ADMIN)
 */
#include "a_sys.h"

#define SYS_sethostname 170
#define SYS_unshare 272
#define CLONE_NEWUTS 0x04000000 /* linux/sched.h */

static struct a_utsname before, u;
static char buf[128];

static i64 read_proc(void) {
    memset(buf, 0, sizeof buf);
    i64 fd = a_open("/proc/sys/kernel/hostname", O_RDONLY, 0);
    if (fd < 0) return fd;
    i64 n = a_read(fd, buf, sizeof buf - 1);
    a_close(fd);
    return n;
}

static i64 write_proc(const char *s, size_t n) {
    i64 fd = a_open("/proc/sys/kernel/hostname", O_WRONLY, 0);
    if (fd < 0) return fd;
    i64 r = a_write(fd, s, n);
    a_close(fd);
    return r;
}

static int child(void) {
    expect("unshare_newuts", syscall1(SYS_unshare, CLONE_NEWUTS), 0);
    expect("sethostname", syscall2(SYS_sethostname, "proc-name", 9), 0);
    expect("proc_read_len", read_proc(), 10);
    check("proc_reads_name_newline", str_eq(buf, "proc-name\n"), 0);

    expect("proc_write", write_proc("from-proc\n", 10), 10);
    memset(&u, 0, sizeof u);
    syscall1(SYS_uname, &u);
    check("uname_after_proc_write", str_eq(u.nodename, "from-proc"), 0);

    expect("sethostname_empty", syscall2(SYS_sethostname, "", 0), 0);
    expect("proc_read_empty", read_proc(), 1);
    check("proc_reads_newline_only", str_eq(buf, "\n"), 0);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    syscall1(SYS_uname, &before);
    a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, child());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    memset(&u, 0, sizeof u);
    syscall1(SYS_uname, &u);
    check("own_name_unchanged", str_eq(u.nodename, before.nodename), 0);
    return finish();
}
