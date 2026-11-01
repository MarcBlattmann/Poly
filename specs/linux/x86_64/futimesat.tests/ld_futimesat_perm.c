/* futimesat permissions. Setting explicit times needs the caller to own
 * the file (or CAP_FOWNER): otherwise EPERM, even with write permission.
 * Setting the current time (times NULL) needs ownership or write
 * permission: a file the caller may write (mode 0666) works, one it may
 * not (0644) is EACCES. Through a descriptor (NULL path) the same rules
 * apply. Root may do all of it.
 * poly-diff: needs=root (files of other owners, dropping to uid 65534)
 */
#include "a_sys.h"

#define SYS_futimesat 261
#define SYS_setresuid 117
#define SYS_setgroups 116
#define SYS_setresgid 119
#define DIR "/tmp/ld_futimesat_perm"

static struct a_timeval tv[2] = {{1234, 0}, {5678, 0}};
static struct a_stat st;

static i64 fut(i64 d, const char *p, const void *t) { return syscall3(SYS_futimesat, d, p, t); }

static int as_user(void) {
    i64 wfd = a_open(DIR "/open", O_RDONLY, 0);
    i64 rfd = a_open(DIR "/closed", O_RDONLY, 0);
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, 65534, 65534, 65534);
    expect("user_setresuid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
    expect("user_writable_now_ok", fut(AT_FDCWD, DIR "/open", 0), 0);
    expect("user_writable_explicit_eperm", fut(AT_FDCWD, DIR "/open", tv), -EPERM);
    expect("user_readonly_now_eacces", fut(AT_FDCWD, DIR "/closed", 0), -EACCES);
    expect("user_readonly_explicit_eperm", fut(AT_FDCWD, DIR "/closed", tv), -EPERM);
    expect("user_fd_writable_now_ok", fut(wfd, 0, 0), 0);
    expect("user_fd_explicit_eperm", fut(wfd, 0, tv), -EPERM);
    expect("user_fd_readonly_now_eacces", fut(rfd, 0, 0), -EACCES);
    expect("user_own_explicit_ok", fut(AT_FDCWD, DIR "/mine", tv), 0);
    a_stat_path(DIR "/mine", &st);
    expect("user_own_mtime", st.st_mtime_sec, 5678);
    a_stat_path(DIR "/closed", &st);
    expect("user_closed_unchanged", st.st_mtime_sec, 1);
    return failed;
}

static void cleanup(void) {
    a_unlink(DIR "/open");
    a_unlink(DIR "/closed");
    a_unlink(DIR "/mine");
    a_rmdir(DIR);
}

int test_main(void) {
    static struct a_timeval one[2] = {{1, 0}, {1, 0}};
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    a_make_file(DIR "/open", "o", 0644);
    syscall2(SYS_chmod, DIR "/open", 0666);
    a_make_file(DIR "/closed", "c", 0644);
    a_make_file(DIR "/mine", "m", 0644);
    expect("chown_mine", syscall3(SYS_chown, DIR "/mine", 65534, 65534), 0);
    expect("root_set_closed", fut(AT_FDCWD, DIR "/closed", one), 0);
    expect("root_set_mine", fut(AT_FDCWD, DIR "/mine", one), 0);
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(as_user() ? 1 : 0);
    i32 status = -1;
    expect("wait_child", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);
    expect("root_other_owner_explicit", fut(AT_FDCWD, DIR "/mine", tv), 0);
    cleanup();
    return finish();
}
