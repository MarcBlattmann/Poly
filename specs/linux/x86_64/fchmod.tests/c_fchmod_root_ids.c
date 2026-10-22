/* fchmod and ownership. Root may change the mode of a file it does not
 * own, and keeps set-group-ID even when the file's group is not one of
 * its groups. Another user who owns the file may change its mode too,
 * but when the file's group is neither its effective gid nor one of its
 * supplementary groups, a requested set-group-ID bit is silently dropped
 * (the call still returns 0; set-user-ID and sticky stay). A user that
 * does not own the file gets EPERM, even with write access.
 * poly-diff: needs=root (making files of other owners and switching users needs root)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119

#define D "/tmp/c_fchmod_root_ids"
#define USER 65534
#define GROUP 65534
#define OTHER_GROUP 12345

static i64 fmode(i64 fd) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

static i64 foreign, own_group, roots;

/* The child only records what it saw (the parent prints). */
static void child(volatile i32 *r) {
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, GROUP, GROUP, GROUP);
    syscall3(SYS_setresuid, USER, USER, USER);
    r[0] = (i32)syscall0(SYS_geteuid);
    if (r[0] != USER) return;
    r[1] = (i32)syscall2(SYS_fchmod, foreign, 02755);
    r[2] = (i32)fmode(foreign);
    r[3] = (i32)syscall2(SYS_fchmod, foreign, 07777);
    r[4] = (i32)fmode(foreign);
    r[5] = (i32)syscall2(SYS_fchmod, own_group, 02755);
    r[6] = (i32)fmode(own_group);
    r[7] = (i32)syscall2(SYS_fchmod, roots, 0600);
    r[8] = (i32)fmode(roots);
}

int test_main(void) {
    a_unlink(D "/foreign");
    a_unlink(D "/own");
    a_unlink(D "/roots");
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    foreign = a_open(D "/foreign", O_RDWR | O_CREAT | O_EXCL, 0644);
    own_group = a_open(D "/own", O_RDWR | O_CREAT | O_EXCL, 0644);
    roots = a_open(D "/roots", O_RDWR | O_CREAT | O_EXCL, 0644);
    check("setup_files", foreign >= 0 && own_group >= 0 && roots >= 0, foreign);
    expect("chown_foreign", syscall3(SYS_fchown, foreign, USER, OTHER_GROUP), 0);
    expect("chown_own", syscall3(SYS_fchown, own_group, USER, GROUP), 0);
    expect("root_fchmod_not_its_file", syscall2(SYS_fchmod, foreign, 02750), 0);
    expect("root_keeps_sgid", fmode(foreign), S_IFREG | 02750);
    expect("root_fchmod_world_writable", syscall2(SYS_fchmod, roots, 0666), 0);
    expect("root_mode", fmode(roots), S_IFREG | 0666);

    volatile i32 *r = a_shared();
    for (int i = 0; i < 9; i++) r[i] = -1;
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        child(r);
        syscall1(SYS_exit_group, 0);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("child_not_root", r[0], USER);
    expect("owner_other_group_02755", r[1], 0);
    expect("sgid_dropped", r[2], S_IFREG | 0755);
    expect("owner_other_group_07777", r[3], 0);
    expect("only_sgid_dropped", r[4], S_IFREG | 05777);
    expect("owner_own_group_02755", r[5], 0);
    expect("sgid_kept_own_group", r[6], S_IFREG | 02755);
    expect("not_owner_eperm", r[7], -EPERM);
    expect("not_owner_mode_kept", r[8], S_IFREG | 0666);
    expect("parent_sees_sgid_dropped", fmode(foreign), S_IFREG | 05777);
    a_close(foreign);
    a_close(own_group);
    a_close(roots);
    expect("cleanup_foreign", a_unlink(D "/foreign"), 0);
    expect("cleanup_own", a_unlink(D "/own"), 0);
    expect("cleanup_roots", a_unlink(D "/roots"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
