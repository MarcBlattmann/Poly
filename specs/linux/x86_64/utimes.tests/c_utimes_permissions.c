/* utimess permission rules for a caller that does not own the file:
 * NULL times need write permission (else EACCES), explicit times are
 * refused (EPERM) even with write permission; a directory in the path
 * without search permission is EACCES; a bad times pointer is EFAULT before
 * any of these, and so is a microseconds value out of range (EINVAL).
 * Root may set explicit times on another user's file.
 * poly-diff: needs=root (only root can create files for another user and switch to it)
 */
#include "a_sys.h"

#define SYS_setuid 105
#define SYS_setgid 106
#define D "/tmp/c_utimes_permissions"
#define NOBODY 65534

/* sys/time.h: struct timeval { time_t tv_sec; suseconds_t tv_usec; } */
struct timeval_k {
    i64 tv_sec, tv_usec;
};

/* Runs in a child; its checks print their own lines, the parent reports
 * the exit status. */
static int as_nobody(void) {
    struct a_stat st;
    struct timeval_k t[2] = {{1, 0}, {2, 0}};
    if (syscall1(SYS_setgid, NOBODY) != 0 || syscall1(SYS_setuid, NOBODY) != 0) {
        check("child_setuid", 0, 0);
        return 1;
    }
    expect("ro_null", syscall2(SYS_utimes, D "/ro", 0), -EACCES);
    expect("ro_explicit", syscall2(SYS_utimes, D "/ro", t), -EPERM);
    expect("ro_bad_times", syscall2(SYS_utimes, D "/ro", (void *)8), -EFAULT);
    struct timeval_k bad[2] = {{1, 0}, {2, 1000000}};
    expect("ro_bad_usec", syscall2(SYS_utimes, D "/ro", bad), -EINVAL);
    expect("rw_bad_usec", syscall2(SYS_utimes, D "/rw", bad), -EINVAL);
    expect("no_search_bad_usec", syscall2(SYS_utimes, D "/closed/f", bad), -EINVAL);
    expect("rw_null", syscall2(SYS_utimes, D "/rw", 0), 0);
    a_stat_path(D "/rw", &st);
    check("rw_null_now", st.st_mtime_sec > 1000 && st.st_atime_sec == st.st_mtime_sec, (i64)st.st_mtime_sec);
    expect("rw_explicit", syscall2(SYS_utimes, D "/rw", t), -EPERM);
    expect("mine_explicit", syscall2(SYS_utimes, D "/mine", t), 0);
    a_stat_path(D "/mine", &st);
    check("mine_values", st.st_atime_sec == 1 && st.st_mtime_sec == 2, (i64)st.st_mtime_sec);
    expect("mine_null", syscall2(SYS_utimes, D "/mine", 0), 0);
    expect("no_search", syscall2(SYS_utimes, D "/closed/f", t), -EACCES);
    expect("no_search_null", syscall2(SYS_utimes, D "/closed/f", 0), -EACCES);
    expect("no_search_missing", syscall2(SYS_utimes, D "/closed/nope", t), -EACCES);
    expect("no_search_bad_times", syscall2(SYS_utimes, D "/closed/f", (void *)8), -EFAULT);
    return failed ? 1 : 0;
}

int test_main(void) {
    struct a_stat st;
    struct timeval_k old[2] = {{1000, 0}, {1000, 0}};
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_chmod_dir", syscall2(SYS_chmod, D, 0755), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0644), 0);
    expect("setup_rw", a_make_file(D "/rw", "", 0644), 0);
    expect("setup_rw_mode", syscall2(SYS_chmod, D "/rw", 0666), 0);
    expect("setup_mine", a_make_file(D "/mine", "", 0600), 0);
    expect("setup_chown", syscall3(SYS_chown, D "/mine", NOBODY, NOBODY), 0);
    expect("setup_closed", a_mkdir(D "/closed", 0700), 0);
    expect("setup_closed_file", a_make_file(D "/closed/f", "", 0666), 0);
    expect("setup_ro_times", syscall2(SYS_utimes, D "/ro", old), 0);
    expect("setup_rw_times", syscall2(SYS_utimes, D "/rw", old), 0);
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, as_nobody());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    check("child_passed", A_WIFEXITED(status) && A_WEXITSTATUS(status) == 0, status);
    a_stat_path(D "/ro", &st);
    check("ro_unchanged", st.st_atime_sec == 1000 && st.st_mtime_sec == 1000, (i64)st.st_mtime_sec);
    struct timeval_k t[2] = {{77, 7}, {88, 8}};
    expect("root_on_other_users_file", syscall2(SYS_utimes, D "/mine", t), 0);
    a_stat_path(D "/mine", &st);
    check("root_values",
          st.st_atime_sec == 77 && st.st_atime_nsec == 7000 && st.st_mtime_sec == 88 && st.st_mtime_nsec == 8000,
          (i64)st.st_mtime_sec);
    expect("cleanup_closed_file", a_unlink(D "/closed/f"), 0);
    expect("cleanup_closed", a_rmdir(D "/closed"), 0);
    expect("cleanup_ro", a_unlink(D "/ro"), 0);
    expect("cleanup_rw", a_unlink(D "/rw"), 0);
    expect("cleanup_mine", a_unlink(D "/mine"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
