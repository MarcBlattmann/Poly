/* An unprivileged user (uid/gid 65534, no capabilities) may create a user
 * namespace and map exactly its own ids, as bubblewrap does without
 * set-user-ID: only a single line with outside id equal to its own
 * effective id and count 1 is accepted from inside (anything else EPERM);
 * gid_map needs "deny" written to setgroups first (EPERM before). Ids
 * that are not mapped read as 65534 (stat of a root-owned directory, the
 * caller's ids before the map); mapped ids read as the inside ids. With
 * the map "0 65534 1" the process is uid 0 inside: setresuid to an
 * unmapped id is EINVAL, chown to one is EINVAL, setgroups(0) is EPERM
 * after "deny", creating a file makes one that is owned by 0 inside; it
 * may signal a process of its own kernel uid outside (kill(.., 0) is 0)
 * but not init (EPERM).
 * (A process whose ids changed must prctl(PR_SET_DUMPABLE, 1), or its /proc/self files belong to root and cannot be written.)
 * poly-diff: needs=root (the test starts as root and becomes uid 65534; creating user namespaces is a system setting)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"

#define DIR "/tmp/pk_idmap_unpriv"

static char buf[512];
static i64 outside_pid;

static int body(void) {
    expect("drop_to_nobody", p_drop_to_nobody(), 0);
    expect("dumpable", syscall2(SYS_prctl, PR_SET_DUMPABLE, 1), 0); /* the ids changed: /proc/self files would be root-owned */
    expect("uid_before", syscall0(SYS_getuid), 65534);
    expect("unshare_newuser", syscall1(SYS_unshare, CLONE_NEWUSER), 0);
    expect("uid_overflow", syscall0(SYS_getuid), 65534);
    expect("gid_overflow", syscall0(SYS_getgid), 65534);

    expect("map_outer_root", p_write_file("/proc/self/uid_map", "0 0 1\n"), -EPERM);
    expect("map_count_2", p_write_file("/proc/self/uid_map", "0 65534 2\n"), -EPERM);
    expect("map_two_lines", p_write_file("/proc/self/uid_map", "0 65534 1\n5 7 1\n"), -EPERM);
    expect("map_outer_65533", p_write_file("/proc/self/uid_map", "0 65533 1\n"), -EPERM);
    expect("gid_map_without_deny", p_write_file("/proc/self/gid_map", "0 65534 1\n"), -EPERM);
    expect("setgroups_deny", p_write_file("/proc/self/setgroups", "deny\n"), 5);
    expect("gid_map_outer_root", p_write_file("/proc/self/gid_map", "0 0 1\n"), -EPERM);
    expect("gid_map_own", p_write_file("/proc/self/gid_map", "0 65534 1\n"), 10);
    expect("uid_map_own", p_write_file("/proc/self/uid_map", "0 65534 1\n"), 10);
    p_read_file("/proc/self/uid_map", buf, sizeof buf);
    check("uid_map_text", str_eq(buf, "         0      65534          1\n"), 0);
    expect("uid_inside_0", syscall0(SYS_getuid), 0);
    expect("gid_inside_0", syscall0(SYS_getgid), 0);

    struct a_stat st;
    expect("stat_tmp", a_stat_path("/tmp", &st), 0);
    expect("tmp_uid_unmapped", st.st_uid, 65534);
    expect("tmp_gid_unmapped", st.st_gid, 65534);

    a_rmdir(DIR);
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    expect("stat_dir", a_stat_path(DIR, &st), 0);
    expect("dir_uid_inside", st.st_uid, 0);
    expect("dir_gid_inside", st.st_gid, 0);
    expect("chown_unmapped_uid", syscall3(SYS_chown, DIR, 5, -1), -EINVAL);
    expect("chown_unmapped_gid", syscall3(SYS_chown, DIR, -1, 5), -EINVAL);
    expect("chown_to_self", syscall3(SYS_chown, DIR, 0, 0), 0);
    expect("setgroups_denied", syscall2(SYS_setgroups, 0, 0), -EPERM);
    expect("setresuid_unmapped", syscall3(SYS_setresuid, 5, 5, 5), -EINVAL);
    expect("setresgid_unmapped", syscall3(SYS_setresgid, 5, 5, 5), -EINVAL);
    expect("setresuid_zero", syscall3(SYS_setresuid, 0, 0, 0), 0);
    expect("kill_same_kernel_uid", syscall2(SYS_kill, outside_pid, 0), 0);
    expect("kill_init", syscall2(SYS_kill, 1, 0), -EPERM);
    expect("unshare_uts_with_caps", syscall1(SYS_unshare, CLONE_NEWUTS), 0);
    expect("sethostname_own_uts", syscall2(SYS_sethostname, "pk", 2), 0);
    expect("rmdir", a_rmdir(DIR), 0);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    i32 hold[2], ready[2];
    p_pipe(hold);
    p_pipe(ready);
    outside_pid = p_fork();
    if (outside_pid == 0) {
        a_close(hold[1]);
        p_drop_to_nobody();
        p_send_byte(ready[1], (char)114);
        char c;
        a_read(hold[0], &c, 1);
        p_exit(0);
    }
    a_close(hold[0]);
    /* Wait until the helper has dropped its ids: kill(.., 0) checks them. */
    expect("helper_ready", p_wait_byte(ready[0]), (i64)(unsigned char)(char)114);
    p_run("userns_child_exit_0", 0, body);
    a_close(hold[1]);
    expect("helper_exit_0", p_wait(outside_pid), 0);
    return finish();
}
