/* Each namespace a process is in is a file under /proc/self/ns: its
 * inode number (fstat st_ino of the opened file) identifies the
 * namespace, and readlink gives "<type>:[<inode>]". unshare of one
 * namespace type changes the inode of exactly that file and leaves the
 * others. For CLONE_NEWPID the caller's own pid namespace (ns/pid) stays
 * and ns/pid_for_children cannot be opened (ENOENT) until the first
 * child is created, which is then in the new pid namespace (the child's
 * ns/pid is the caller's ns/pid_for_children). CLONE_NEWTIME changes
 * ns/time_for_children only (at once), and the next child is in that
 * time namespace.
 * poly-diff: needs=root (new namespaces need CAP_SYS_ADMIN)
 * poly-diff: syscalls=names (readlink's length depends on the inode numbers)
 */
#include "a_sys.h"

#define SYS_unshare 272
/* linux/sched.h */
#define CLONE_NEWTIME 0x80
#define CLONE_NEWNS 0x20000
#define CLONE_NEWCGROUP 0x02000000
#define CLONE_NEWUTS 0x04000000
#define CLONE_NEWIPC 0x08000000
#define CLONE_NEWPID 0x20000000
#define CLONE_NEWNET 0x40000000

#define N 9
static const char *names[N] = {"uts", "ipc", "net", "mnt", "cgroup", "pid", "pid_for_children", "time",
                               "time_for_children"};
static u64 ino0[N], ino1[N];

static u64 ns_ino(const char *name) {
    char path[64];
    str_cat2(path, "/proc/self/ns/", name);
    struct a_stat st;
    if (a_stat_path(path, &st) != 0) return 0;
    return st.st_ino;
}

static void snap(u64 *out) {
    for (int i = 0; i < N; i++) out[i] = ns_ino(names[i]);
}

/* bit i set when names[i] changed between the snapshots */
static i64 changed(void) {
    i64 m = 0;
    for (int i = 0; i < N; i++)
        if (ino0[i] != ino1[i]) m |= 1 << i;
    return m;
}

static int one(const char *label, u64 flag, i64 want_mask) {
    snap(ino0);
    i64 r = syscall1(SYS_unshare, flag);
    snap(ino1);
    char nm[64];
    str_cat2(nm, label, "_ok");
    expect(nm, r, 0);
    str_cat2(nm, label, "_changes");
    expect(nm, changed(), want_mask);
    return 0;
}

static int child(void) {
    snap(ino0);
    int all_found = 1;
    for (int i = 0; i < N; i++) all_found &= ino0[i] != 0;
    check("all_ns_files_present", all_found, 0);

    /* readlink names the type and the inode. */
    char link[64], want[64], num[24];
    memset(link, 0, sizeof link);
    i64 n = syscall3(SYS_readlink, "/proc/self/ns/uts", link, sizeof link - 1);
    str_cpy(str_cpy(str_cpy(want, "uts:["), a_utoa(num, ino0[0])), "]");
    check("readlink_uts_type_inode", n > 0 && str_eq(link, want), n);
    check("pid_and_children_same_at_start", ino0[5] == ino0[6], 0);

    one("uts", CLONE_NEWUTS, 1 << 0);
    one("ipc", CLONE_NEWIPC, 1 << 1);
    one("net", CLONE_NEWNET, 1 << 2);
    one("mnt", CLONE_NEWNS, 1 << 3);
    one("cgroup", CLONE_NEWCGROUP, 1 << 4);
    one("pid", CLONE_NEWPID, 1 << 6);
    struct a_stat st0;
    expect("pid_for_children_enoent_before_child", a_stat_path("/proc/self/ns/pid_for_children", &st0), -ENOENT);
    one("time", CLONE_NEWTIME, 1 << 8);

    /* A child is in the pid and time namespaces made for children. */
    u64 tfc = ns_ino("time_for_children");
    volatile i32 *sh = a_shared();
    i64 c = a_fork_sync();
    if (c == 0) {
        sh[2] = (i32)(ns_ino("pid") & 0x7fffffff);
        sh[3] = ns_ino("time") == tfc;
        syscall1(SYS_exit_group, 0);
    }
    i32 st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    u64 pfc = ns_ino("pid_for_children");
    sh[2] = pfc != 0 && sh[2] == (i32)(pfc & 0x7fffffff) && pfc != ino0[5];
    expect("child_in_new_pid_ns", sh[2], 1);
    expect("child_in_new_time_ns", sh[3], 1);
    sh[0] = passed;
    sh[1] = failed;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, child());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_exit_0", st, 0);
    passed += sh[0];
    failed += sh[1];
    return finish();
}
