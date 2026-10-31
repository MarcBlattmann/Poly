/* prctl(PR_SET_NAME 15, name) sets the calling thread's name (comm) to
 * the first 15 bytes of name; PR_GET_NAME (16) stores it into a 16-byte
 * buffer, NUL terminated and padded with NULs to 16 bytes (bytes after
 * the 16th are not touched); both return 0. The name shows in
 * /proc/self/comm (with a newline) and /proc/self/task/<tid>/comm. The
 * empty name is allowed. An unreadable name or unwritable buffer is
 * EFAULT. The name is per thread: a thread renaming itself leaves the
 * leader's name alone. Unused arguments are ignored.
 * poly-diff: syscalls=names (the /proc/self/task path holds the tid, which differs between runs)
 */
#include "a_sys.h"

#define SYS_prctl 157
#define PR_SET_NAME 15
#define PR_GET_NAME 16

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM | CLONE_CHILD_CLEARTID)

static i64 pr(u64 op, u64 a) { return syscall5(SYS_prctl, op, a, 0, 0, 0); }

static i64 read_file(const char *path, char *buf, size_t n) {
    i64 fd = a_open(path, O_RDONLY, 0);
    if (fd < 0) return fd;
    i64 r = a_read(fd, buf, n - 1);
    a_close(fd);
    buf[r > 0 ? r : 0] = 0;
    return r;
}

static volatile i32 done, renamed, go;
static char thread_name[32];

static int thread_fn(void *arg) {
    (void)arg;
    pr(PR_SET_NAME, (u64) "worker");
    pr(PR_GET_NAME, (u64)thread_name);
    renamed = 1;
    while (!go) a_relax();
    return 0;
}

int test_main(void) {
    char buf[32];
    memset(buf, 'X', sizeof buf);
    expect("get_ok", pr(PR_GET_NAME, (u64)buf), 0);
    check("get_terminated_within_16", buf[15] == 0, buf[15]);
    check("get_leaves_byte_16", buf[16] == 'X', buf[16]);

    expect("set_long", pr(PR_SET_NAME, (u64) "0123456789abcdefghij"), 0);
    memset(buf, 'X', sizeof buf);
    pr(PR_GET_NAME, (u64)buf);
    check("truncated_to_15", str_eq(buf, "0123456789abcde"), 0);
    check("byte_16_untouched", buf[16] == 'X', buf[16]);

    expect("set_short", pr(PR_SET_NAME, (u64) "abc"), 0);
    memset(buf, 'X', sizeof buf);
    pr(PR_GET_NAME, (u64)buf);
    check("short_name", str_eq(buf, "abc"), 0);
    check("nul_padded", buf[3] == 0 && buf[10] == 0 && buf[15] == 0, buf[10]);

    char comm[64];
    read_file("/proc/self/comm", comm, sizeof comm);
    check("proc_comm", str_eq(comm, "abc\n"), 0);
    expect("extra_args_ignored", syscall5(SYS_prctl, PR_SET_NAME, (u64) "named", 1, 2, 3), 0);
    char path[64];
    char *p = str_cpy(path, "/proc/self/task/");
    p = a_utoa(p, (u64)syscall0(SYS_gettid));
    str_cpy(p + str_len(p), "/comm");
    read_file(path, comm, sizeof comm);
    check("proc_task_comm", str_eq(comm, "named\n"), 0);

    expect("set_empty", pr(PR_SET_NAME, (u64) ""), 0);
    memset(buf, 'X', sizeof buf);
    pr(PR_GET_NAME, (u64)buf);
    check("empty_name", buf[0] == 0 && buf[15] == 0, buf[0]);

    expect("set_null_efault", pr(PR_SET_NAME, 0), -EFAULT);
    expect("get_null_efault", pr(PR_GET_NAME, 0), -EFAULT);
    char *ro = (char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("get_read_only_efault", pr(PR_GET_NAME, (u64)ro), -EFAULT);

    /* Per thread. */
    pr(PR_SET_NAME, (u64) "leader");
    done = -1;
    a_clone(THREAD_FLAGS, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&done, 0, thread_fn, 0);
    a_spin_until(&renamed, 1, A_SPIN_LIMIT);
    check("thread_own_name", str_eq(thread_name, "worker"), 0);
    memset(buf, 0, sizeof buf);
    pr(PR_GET_NAME, (u64)buf);
    check("leader_name_kept", str_eq(buf, "leader"), 0);
    read_file("/proc/self/comm", comm, sizeof comm);
    check("proc_comm_is_leader", str_eq(comm, "leader\n"), 0);
    go = 1;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    return finish();
}
