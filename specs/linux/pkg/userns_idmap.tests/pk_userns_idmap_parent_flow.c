/* The usual sandbox start: a child that runs as uid/gid 1000 makes a user
 * namespace (unshare(CLONE_NEWUSER)); the root parent writes the child's setgroups/uid_map/gid_map from
 * outside while the child waits, then lets it go. Before the maps are
 * written the child's ids read as 65534 and its uid_map is empty; the
 * parent reads the child's maps translated into its own ids. With the
 * maps "0 1000 1" the child is uid/gid 0 inside; a file made there is
 * owned by 1000 outside, a file owned by 1000 outside is owned by 0
 * inside (the child may open it), a root-owned file is owned by 65534
 * inside and a mode 0600 one cannot be opened (EACCES) although the
 * child is uid 0 (it has no capability over unmapped ids); chown to an
 * unmapped id is EINVAL. The child has no privilege over the parent's
 * processes: kill(parent, 0) is EPERM. A root parent needs no "deny" and
 * the child may call setgroups. /proc/<child>/status shows the child's
 * ids as the parent sees them (1000).
 * poly-diff: needs=root (CAP_SETUID in the parent namespace)
 * poly-diff: syscalls=off (several processes, pids in /proc paths)
 */
#include "p_ns.h"

#define PF "/tmp/pk_idmap_pf"
#define CF "/tmp/pk_idmap_cf"
#define RF "/tmp/pk_idmap_rf"

static char buf[1024];

static int pre_ids(void) {
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, 1000, 1000, 1000);
    syscall3(SYS_setresuid, 1000, 1000, 1000);
    syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
    return syscall1(SYS_unshare, CLONE_NEWUSER) == 0 ? 0 : 1;
}

static void parent_setup(i64 pid) {
    char path[48];
    p_read_file(p_proc_path(path, pid, "uid_map"), buf, sizeof buf);
    check("child_uid_map_empty_before", buf[0] == 0, 0);
    p_read_file(p_proc_path(path, pid, "setgroups"), buf, sizeof buf);
    check("child_setgroups_allow", str_eq(buf, "allow\n"), 0);
    expect("write_uid_map", p_write_file(p_proc_path(path, pid, "uid_map"), "0 1000 1\n"), 9);
    expect("write_gid_map", p_write_file(p_proc_path(path, pid, "gid_map"), "0 1000 1\n"), 9);
    p_read_file(p_proc_path(path, pid, "uid_map"), buf, sizeof buf);
    check("parent_reads_uid_map", str_eq(buf, "         0       1000          1\n"), 0);
    p_read_file(p_proc_path(path, pid, "status"), buf, sizeof buf);
    const char *u = p_status_field(buf, "Uid");
    check("parent_sees_uid_1000", u && p_starts(u, "1000\t1000\t1000\t1000\n"), 0);
    const char *g = p_status_field(buf, "Gid");
    check("parent_sees_gid_1000", g && p_starts(g, "1000\t1000\t1000\t1000\n"), 0);
    expect("second_uid_map_write", p_write_file(p_proc_path(path, pid, "uid_map"), "0 1000 1\n"), -EPERM);
}

static int child(void) {
    struct a_stat st;
    expect("uid_0", syscall0(SYS_getuid), 0);
    expect("euid_0", syscall0(SYS_geteuid), 0);
    expect("gid_0", syscall0(SYS_getgid), 0);
    p_read_file("/proc/self/uid_map", buf, sizeof buf);
    check("child_reads_uid_map", str_eq(buf, "         0       1000          1\n"), 0);
    p_read_file("/proc/self/status", buf, sizeof buf);
    const char *u = p_status_field(buf, "Uid");
    check("child_status_uid_0", u && p_starts(u, "0\t0\t0\t0\n"), 0);

    expect("stat_parent_file", a_stat_path(PF, &st), 0);
    expect("parent_file_uid_0", st.st_uid, 0);
    expect("parent_file_gid_0", st.st_gid, 0);
    i64 fd = a_open(PF, O_RDONLY, 0);
    check("open_parent_file_0600", fd >= 0, fd);
    a_close(fd);
    expect("stat_root_file", a_stat_path(RF, &st), 0);
    expect("root_file_uid_overflow", st.st_uid, 65534);
    expect("root_file_gid_overflow", st.st_gid, 65534);
    expect("open_root_file_0600", a_open(RF, O_RDONLY, 0), -EACCES);
    expect("chown_unmapped", syscall3(SYS_chown, PF, 0, 7), -EINVAL);

    expect("make_file", a_make_file(CF, "x", 0644), 0);
    expect("stat_own_file", a_stat_path(CF, &st), 0);
    expect("own_file_uid_0", st.st_uid, 0);
    expect("own_file_gid_0", st.st_gid, 0);
    expect("kill_parent", syscall2(SYS_kill, syscall0(SYS_getppid), 0), -EPERM);
    expect("setgroups_empty", syscall2(SYS_setgroups, 0, 0), 0);

    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    a_unlink(PF);
    a_unlink(CF);
    a_unlink(RF);
    a_make_file(PF, "p", 0600);
    syscall3(SYS_chown, PF, 1000, 1000);
    a_make_file(RF, "r", 0600);
    p_run_staged("child_exit_0", 0, pre_ids, child, parent_setup);
    struct a_stat st;
    expect("stat_child_file", a_stat_path(CF, &st), 0);
    expect("child_file_uid_1000", st.st_uid, 1000);
    expect("child_file_gid_1000", st.st_gid, 1000);
    a_unlink(PF);
    a_unlink(CF);
    a_unlink(RF);
    return finish();
}
