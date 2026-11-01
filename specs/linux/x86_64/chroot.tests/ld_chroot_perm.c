/* chroot needs CAP_SYS_CHROOT: a caller without it (here a child that
 * dropped to uid/gid 65534) gets EPERM for any existing directory it
 * may search (also "/" and "."). The lookup and permission checks come
 * first: ENOENT for a missing path, ENOTDIR for a file, EACCES for a
 * directory without search (x) permission for the caller or reached
 * through one. Root passes the permission checks (CAP_DAC_OVERRIDE):
 * it may chroot to a directory of mode 0000.
 * poly-diff: needs=root (drops to another uid; root chroots)
 */
#include "a_sys.h"

#define SYS_chroot 161
#define SYS_setresuid 117
#define SYS_setgroups 116
#define SYS_setresgid 119
#define DIR "/tmp/ld_chroot_perm"

static int run_child(int (*fn)(void)) {
    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(fn() ? 1 : 0);
    i32 status = -1;
    if (syscall4(SYS_wait4, pid, &status, 0, 0) != pid) return -1;
    return status;
}

static int as_user(void) {
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, 65534, 65534, 65534);
    expect("user_setresuid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
    expect("user_dir_eperm", syscall1(SYS_chroot, DIR "/open"), -EPERM);
    expect("user_slash_eperm", syscall1(SYS_chroot, "/"), -EPERM);
    expect("user_dot_eperm", syscall1(SYS_chroot, "."), -EPERM);
    expect("user_missing_enoent", syscall1(SYS_chroot, DIR "/none"), -ENOENT);
    expect("user_file_enotdir", syscall1(SYS_chroot, DIR "/f"), -ENOTDIR);
    expect("user_nosearch_eacces", syscall1(SYS_chroot, DIR "/closed"), -EACCES);
    expect("user_through_nosearch_eacces", syscall1(SYS_chroot, DIR "/closed/in"), -EACCES);
    struct a_stat st;
    expect("user_root_unchanged", a_stat_path(DIR "/f", &st), 0);
    return failed;
}

static int as_root(void) {
    expect("root_mode_0000_dir", syscall1(SYS_chroot, DIR "/closed"), 0);
    struct a_stat st;
    expect("root_sees_in", a_stat_path("/in", &st), 0);
    return failed;
}

static void cleanup(void) {
    syscall2(SYS_chmod, DIR "/closed", 0755);
    a_rmdir(DIR "/closed/in");
    a_rmdir(DIR "/closed");
    a_rmdir(DIR "/open");
    a_unlink(DIR "/f");
    a_rmdir(DIR);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    a_mkdir(DIR "/open", 0755);
    a_mkdir(DIR "/closed", 0755);
    a_mkdir(DIR "/closed/in", 0755);
    expect("chmod_closed_0000", syscall2(SYS_chmod, DIR "/closed", 0), 0);
    expect("file", a_make_file(DIR "/f", "x", 0644), 0);
    a_block_sigchld();
    expect("child_user", run_child(as_user), 0);
    expect("child_root", run_child(as_root), 0);
    cleanup();
    return finish();
}
