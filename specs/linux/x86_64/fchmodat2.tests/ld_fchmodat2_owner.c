/* Who may change a mode with fchmodat2: the file's owner, or a caller
 * with CAP_FOWNER (root). Any other caller gets EPERM, for every form
 * (path, AT_SYMLINK_NOFOLLOW, AT_EMPTY_PATH on a descriptor it holds),
 * whatever the file's mode, after the lookup (a missing name is ENOENT,
 * a directory it may not search EACCES). An owner that is not in the
 * file's group may still chmod, but S_ISGID is silently dropped (the
 * other bits are set); root keeps it.
 * poly-diff: needs=root (files of other owners, dropping to uid 65534)
 */
#include "a_sys.h"

#define SYS_fchmodat2 452
#define SYS_setresuid 117
#define SYS_setgroups 116
#define SYS_setresgid 119
#define DIR "/tmp/ld_fchmodat2_owner"

static struct a_stat st;

static i64 chm(i64 d, const char *p, u64 mode, u64 flags) { return syscall4(SYS_fchmodat2, d, p, mode, flags); }

static u64 mode_of(const char *p) {
    memset(&st, 0, sizeof st);
    a_lstat_path(p, &st);
    return st.st_mode;
}

static int as_user(void) {
    i64 fd = a_open(DIR "/rootfile", O_RDONLY, 0);
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, 65534, 65534, 65534);
    expect("user_setresuid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
    expect("user_other_file_eperm", chm(AT_FDCWD, DIR "/rootfile", 0600, 0), -EPERM);
    expect("user_other_world_writable_eperm", chm(AT_FDCWD, DIR "/rootopen", 0600, 0), -EPERM);
    expect("user_nofollow_eperm", chm(AT_FDCWD, DIR "/rootfile", 0600, AT_SYMLINK_NOFOLLOW), -EPERM);
    expect("user_fd_eperm", chm(fd, "", 0600, AT_EMPTY_PATH), -EPERM);
    expect("user_missing_enoent", chm(AT_FDCWD, DIR "/none", 0600, 0), -ENOENT);
    expect("user_nosearch_eacces", chm(AT_FDCWD, DIR "/closed/x", 0600, 0), -EACCES);
    expect("user_rootfile_unchanged", mode_of(DIR "/rootfile"), S_IFREG | 0644);
    /* Owned by 65534, group 0 (not a member): S_ISGID dropped. */
    expect("user_own_file", chm(AT_FDCWD, DIR "/userfile", 02755, 0), 0);
    expect("user_sgid_dropped", mode_of(DIR "/userfile"), S_IFREG | 0755);
    expect("user_suid_sticky_kept", chm(AT_FDCWD, DIR "/userfile", 05700, 0), 0);
    expect("user_suid_sticky_mode", mode_of(DIR "/userfile"), S_IFREG | 05700);
    return failed;
}

static void cleanup(void) {
    a_unlink(DIR "/rootfile");
    a_unlink(DIR "/rootopen");
    a_unlink(DIR "/userfile");
    a_rmdir(DIR "/closed");
    a_rmdir(DIR);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    a_mkdir(DIR "/closed", 0700);
    expect("rootfile", a_make_file(DIR "/rootfile", "r", 0644), 0);
    a_make_file(DIR "/rootopen", "o", 0644);
    syscall2(SYS_chmod, DIR "/rootopen", 0666);
    a_make_file(DIR "/userfile", "u", 0644);
    expect("chown_userfile", syscall3(SYS_chown, DIR "/userfile", 65534, 0), 0);
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(as_user() ? 1 : 0);
    i32 status = -1;
    expect("wait_child", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);

    /* Root: anyone's file, S_ISGID kept for a group it is not in. */
    expect("root_other_owner", chm(AT_FDCWD, DIR "/userfile", 02755, 0), 0);
    expect("root_sgid_kept", mode_of(DIR "/userfile"), S_IFREG | 02755);
    cleanup();
    return finish();
}
