/* A procfs mounted by the first process of a PID namespace shows only that
 * namespace. In a child made with clone(CLONE_NEWPID | CLONE_NEWNS) whose
 * mounts were made private, mounting "proc" on a new directory works (0)
 * and: the numeric entries of its root are only the processes of the
 * namespace (one at first: "1"; two once the init has a child, "1" and
 * "2"); "self" reads as the link "1" and "thread-self" as "1/task/1"; its
 * /1/status has "Pid:" 1, "PPid:" 0 and "NSpid:" with the one number 1;
 * the creator's number in the initial namespace has no entry (ENOENT).
 * The /proc that was mounted before (the initial namespace's) keeps
 * showing the initial namespace: its "self" link is the process's number
 * out there (not 1, and different from what the mounted one shows) and
 * it lists the creator. /proc/sys/kernel/ns_last_pid of the new mount is
 * the last number handed out in the namespace (2 after the first child),
 * writing 100 into it makes the next process 101, and the value read back
 * is then 101. The mount can be unmounted again and its directory
 * removed.
 * poly-diff: needs=root (a new PID namespace and a mount need CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (several processes, pids in the output paths)
 */
#include "p_ns.h"

static char buf[4096];
static i64 creator;

static int init_body(void) {
    char link[64];
    expect("private", p_mount("", "/", P_NULL, MS_REC | MS_PRIVATE, P_NULL), 0);
    a_mkdir("/tmp/pk_pidns_proc", 0755);
    expect("mount_proc", p_mount("proc", "/tmp/pk_pidns_proc", "proc", 0, P_NULL), 0);

    int num = -1, all = -1;
    expect("count_dir", p_count_dir("/tmp/pk_pidns_proc", &num, &all), 0);
    expect("only_init_listed", num, 1);
    check("has_self", p_dir_has("/tmp/pk_pidns_proc", "self"), 0);
    check("has_thread_self", p_dir_has("/tmp/pk_pidns_proc", "thread-self"), 0);
    check("has_1", p_dir_has("/tmp/pk_pidns_proc", "1"), 0);
    expect("self_link", p_readlink("/tmp/pk_pidns_proc/self", link, sizeof link) > 0 ? str_eq(link, "1") : -1, 1);
    expect("thread_self_link",
           p_readlink("/tmp/pk_pidns_proc/thread-self", link, sizeof link) > 0 ? str_eq(link, "1/task/1") : -1, 1);

    expect("status_read", p_read_file("/tmp/pk_pidns_proc/1/status", buf, sizeof buf) > 0, 1);
    const char *pid = p_status_field(buf, "Pid");
    const char *ppid = p_status_field(buf, "PPid");
    const char *nspid = p_status_field(buf, "NSpid");
    expect("status_pid_1", pid ? (i64)p_num(pid) : -1, 1);
    expect("status_ppid_0", ppid ? (i64)p_num(ppid) : -1, 0);
    expect("status_nspid_fields", nspid ? p_fields(nspid) : -1, 1);

    struct a_stat st;
    char path[64];
    p_proc_path(path, creator, "stat");
    str_cpy(str_cpy(buf, "/tmp/pk_pidns_proc"), path + 5);
    expect("creator_not_visible", a_stat_path(buf, &st), -ENOENT);

    /* The old mount still shows the initial namespace. */
    expect("old_self_readable", p_readlink("/proc/self", link, sizeof link) > 0, 1);
    check("old_self_not_1", !str_eq(link, "1"), 0);
    expect("old_self_is_outer_number", (i64)a_atou(link) != 1, 1);
    check("old_lists_creator", p_dir_has("/proc", a_utoa(buf, (u64)creator)), 0);

    i32 hold[2];
    p_pipe(hold);
    i64 kid = p_fork();
    if (kid == 0) {
        char c;
        a_read(hold[0], &c, 1);
        p_exit(0);
    }
    expect("child_is_2", kid, 2);
    expect("count_dir_2", p_count_dir("/tmp/pk_pidns_proc", &num, &all), 0);
    expect("two_listed", num, 2);
    check("has_2", p_dir_has("/tmp/pk_pidns_proc", "2"), 0);

    expect("last_pid_read", p_read_file("/tmp/pk_pidns_proc/sys/kernel/ns_last_pid", buf, sizeof buf) > 0, 1);
    expect("last_pid_is_2", (i64)p_num(buf), 2);
    expect("last_pid_write", p_write_file("/tmp/pk_pidns_proc/sys/kernel/ns_last_pid", "100"), 3);
    i64 kid2 = p_fork();
    if (kid2 == 0) p_exit(0);
    expect("next_is_101", kid2, 101);
    int wst = p_wait(kid2);
    expect("kid2_status", wst, 0);
    p_read_file("/tmp/pk_pidns_proc/sys/kernel/ns_last_pid", buf, sizeof buf);
    expect("last_pid_is_101", (i64)p_num(buf), 101);

    p_send_byte(hold[1], 'x');
    expect("kid_status", p_wait(kid), 0);
    expect("umount", syscall2(SYS_umount2, "/tmp/pk_pidns_proc", 0), 0);
    expect("rmdir", a_rmdir("/tmp/pk_pidns_proc"), 0);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    creator = syscall0(SYS_getpid);
    p_run("pidns_proc", CLONE_NEWPID | CLONE_NEWNS, init_body);
    return finish();
}
