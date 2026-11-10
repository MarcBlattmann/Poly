/* chown by the root of a user namespace. Its CAP_CHOWN and CAP_FOWNER
 * reach exactly the files whose owner uid and group are both mapped in its
 * namespace: such a file can be given to any mapped uid and gid (the file
 * need not be owned by the caller: after it was given away it can be
 * taken back), and chmod works on it; -1 leaves that id as it is. The ids
 * are translated through the maps: with "0 3000 1" and "1 300000 400" the
 * inside id 5 is the kernel id 300004, which is what the parent namespace
 * sees. An id that is not mapped is EINVAL as a target, and a file whose
 * owner is not mapped (it shows as 65534) cannot be changed at all (chown
 * and chmod EPERM), not even to the ids it already shows.
 * The same holds one level down. In a namespace made inside the first one
 * and mapped onto it ("0 0 1" and "1 100 50": inside 1..50 are the outer
 * namespace's 100..149), its root gives a file to its ids 3 and 4; the
 * outer namespace sees 102 and 103 and the initial namespace 300101 and
 * 300102. A file the inner root made is owned by the inner 0 = outer 0 =
 * kernel 3000.
 * poly-diff: needs=root (CAP_SETUID in the parent namespace; creating user namespaces is a system setting)
 * poly-diff: syscalls=off (several processes, pids in /proc paths)
 */
#include "p_ns.h"

#define F1 "/tmp/pk_idmap_chown_a"
#define F2 "/tmp/pk_idmap_chown_b"
#define FR "/tmp/pk_idmap_chown_root"

static i64 owner(const char *path, int gid) {
    struct a_stat st;
    i64 r = a_stat_path(path, &st);
    if (r < 0) return r;
    return gid ? (i64)st.st_gid : (i64)st.st_uid;
}

static i64 mode_of(const char *path) {
    struct a_stat st;
    i64 r = a_stat_path(path, &st);
    return r < 0 ? r : (i64)(st.st_mode & 07777);
}

static int become_3000(void) {
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, 3000, 3000, 3000);
    syscall3(SYS_setresuid, 3000, 3000, 3000);
    syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
    return syscall1(SYS_unshare, CLONE_NEWUSER) == 0 ? 0 : 1;
}

static void map_outer(i64 pid) {
    char path[48];
    expect("outer_uid_map", p_write_file(p_proc_path(path, pid, "uid_map"), "0 3000 1\n1 300000 400\n"), 22);
    expect("outer_gid_map", p_write_file(p_proc_path(path, pid, "gid_map"), "0 3000 1\n1 300000 400\n"), 22);
}

static int inner(void) {
    expect("inner_uid_0", syscall0(SYS_getuid), 0);
    expect("inner_make_file", a_make_file(F2, "b", 0644), 0);
    expect("inner_file_uid", owner(F2, 0), 0);
    expect("inner_chown_3_4", syscall3(SYS_chown, F2, 3, 4), 0);
    expect("inner_sees_uid_3", owner(F2, 0), 3);
    expect("inner_sees_gid_4", owner(F2, 1), 4);
    expect("inner_chown_last_mapped", syscall3(SYS_chown, F2, 50, 50), 0);
    expect("inner_chown_first_unmapped", syscall3(SYS_chown, F2, 51, 50), -EINVAL);
    expect("inner_take_back_and_give", syscall3(SYS_chown, F2, 3, 4), 0);
    /* F1 belongs to the outer namespace's 5:9, which the inner one does not map */
    expect("inner_sees_outer_file_as_overflow", owner(F1, 0), 65534);
    expect("inner_chown_unmapped_owner", syscall3(SYS_chown, F1, 0, 0), -EPERM);
    expect("inner_chmod_unmapped_owner", syscall2(SYS_chmod, F1, 0600), -EPERM);
    return failed ? 1 : 0;
}

static int outer(void) {
    expect("uid_0_inside", syscall0(SYS_getuid), 0);
    expect("make_file", a_make_file(F1, "a", 0644), 0);
    expect("new_file_uid_0", owner(F1, 0), 0);
    expect("new_file_gid_0", owner(F1, 1), 0);
    expect("chown_5_9", syscall3(SYS_chown, F1, 5, 9), 0);
    expect("uid_now_5", owner(F1, 0), 5);
    expect("gid_now_9", owner(F1, 1), 9);
    expect("chmod_file_of_another_mapped_owner", syscall2(SYS_chmod, F1, 0640), 0);
    expect("chown_gid_only", syscall3(SYS_chown, F1, -1, 20), 0);
    expect("uid_kept", owner(F1, 0), 5);
    expect("gid_now_20", owner(F1, 1), 20);
    expect("chown_uid_only", syscall3(SYS_chown, F1, 400, -1), 0);
    expect("uid_now_400_last_mapped", owner(F1, 0), 400);
    expect("chown_401_unmapped", syscall3(SYS_chown, F1, 401, -1), -EINVAL);
    expect("chown_gid_3000_unmapped", syscall3(SYS_chown, F1, -1, 3000), -EINVAL);
    expect("take_back", syscall3(SYS_chown, F1, 0, 0), 0);
    expect("chown_5_9_again", syscall3(SYS_chown, F1, 5, 9), 0);
    /* a file of the initial namespace's root: its owner is not mapped here */
    expect("root_file_uid_shows_overflow", owner(FR, 0), 65534);
    expect("root_file_gid_shows_overflow", owner(FR, 1), 65534);
    expect("chown_unmapped_owner_eperm", syscall3(SYS_chown, FR, 0, 0), -EPERM);
    expect("chown_unmapped_owner_to_shown_ids", syscall3(SYS_chown, FR, 65534, 65534), -EINVAL);
    expect("chown_unmapped_owner_no_change", syscall3(SYS_chown, FR, -1, -1), 0);
    expect("chmod_unmapped_owner_eperm", syscall2(SYS_chmod, FR, 0600), -EPERM);

    /* one level down */
    i32 hold[2];
    p_pipe(hold);
    i64 pid = p_clone(CLONE_NEWUSER);
    if (pid == 0) {
        a_close(hold[1]);
        p_wait_byte(hold[0]);
        passed = failed = 0;
        int rc = inner();
        a_shared()[4] = passed;
        a_shared()[5] = failed;
        p_exit(rc);
    }
    a_close(hold[0]);
    char path[48];
    expect("inner_uid_map", p_write_file(p_proc_path(path, pid, "uid_map"), "0 0 1\n1 100 50\n"), 15);
    expect("inner_gid_map", p_write_file(p_proc_path(path, pid, "gid_map"), "0 0 1\n1 100 50\n"), 15);
    p_send_byte(hold[1], 'g');
    a_close(hold[1]);
    expect("inner_exit_0", p_wait(pid), 0);
    passed += a_shared()[4];
    failed += a_shared()[5];
    expect("outer_sees_inner_3_as_102", owner(F2, 0), 102);
    expect("outer_sees_inner_4_as_103", owner(F2, 1), 103);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    a_unlink(F1);
    a_unlink(F2);
    a_unlink(FR);
    expect("root_file", a_make_file(FR, "r", 0644), 0);
    p_run_staged("outer_child_exit_0", 0, become_3000, outer, map_outer);
    expect("kernel_uid_of_5", owner(F1, 0), 300004);
    expect("kernel_gid_of_9", owner(F1, 1), 300008);
    expect("kernel_uid_of_inner_3", owner(F2, 0), 300101);
    expect("kernel_gid_of_inner_4", owner(F2, 1), 300102);
    expect("root_file_untouched_uid", owner(FR, 0), 0);
    expect("root_file_mode_untouched", mode_of(FR), 0644);
    a_unlink(F1);
    a_unlink(F2);
    a_unlink(FR);
    return finish();
}
