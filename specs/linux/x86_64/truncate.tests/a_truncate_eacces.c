/* truncate needs write permission on the file and search permission on
 * every directory on the way: without them it is EACCES and nothing
 * changes. Read permission is not needed. The file type is checked
 * before permission: a directory is still EISDIR and a FIFO EINVAL, and
 * a negative length still EINVAL. A missing name in a directory that may
 * not be searched is EACCES, not ENOENT.
 * The checks run in a child; if the test runs as root, the child first
 * becomes uid/gid 65534 so that permission checks apply (the files are
 * then another user's, and only their "other" bits count: the modes are
 * chosen so the result is the same either way).
 */
#include "a_sys.h"

#define D "/tmp/a_truncate_eacces"

/* asm/unistd_64.h */
#define SYS_mknodat 259
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119

static int checks(void) {
    struct a_stat st;
    if (syscall0(SYS_geteuid) == 0) {
        check("drop_groups", syscall2(SYS_setgroups, 0, 0) == 0, 0);
        check("drop_gid", syscall3(SYS_setresgid, 65534, 65534, 65534) == 0, 0);
        check("drop_uid", syscall3(SYS_setresuid, 65534, 65534, 65534) == 0, 0);
    } else {
        check("drop_groups", 1, 0);
        check("drop_gid", 1, 0);
        check("drop_uid", 1, 0);
    }
    expect("readonly_file", syscall2(SYS_truncate, D "/ro", 0), -EACCES);
    expect("readonly_file_grow", syscall2(SYS_truncate, D "/ro", 100), -EACCES);
    expect("readonly_same_size", syscall2(SYS_truncate, D "/ro", 5), -EACCES);
    a_stat_path(D "/ro", &st);
    expect("readonly_size_kept", st.st_size, 5);
    expect("readonly_negative", syscall2(SYS_truncate, D "/ro", -1), -EINVAL);
    expect("no_bits_file", syscall2(SYS_truncate, D "/none_bits", 0), -EACCES);
    expect("writeonly_file", syscall2(SYS_truncate, D "/wo", 1), 0);
    a_stat_path(D "/wo", &st);
    expect("writeonly_size", st.st_size, 1);
    expect("no_search_file", syscall2(SYS_truncate, D "/nx/f", 0), -EACCES);
    expect("no_search_missing", syscall2(SYS_truncate, D "/nx/missing", 0), -EACCES);
    expect("no_search_negative", syscall2(SYS_truncate, D "/nx/f", -1), -EINVAL);
    expect("readonly_dir_file", syscall2(SYS_truncate, D "/rodir/f", 2), 0);
    expect("dir_without_write", syscall2(SYS_truncate, D "/rodir", 0), -EISDIR);
    expect("fifo_without_write", syscall2(SYS_truncate, D "/p", 0), -EINVAL);
    return finish();
}

int test_main(void) {
    struct a_stat st;
    a_block_sigchld();
    check("setup_mkdir", a_mkdir(D, 0755) == 0, 0);
    check("setup_ro", a_make_file(D "/ro", "hello", 0444) == 0, 0);
    check("setup_none_bits", a_make_file(D "/none_bits", "hello", 0) == 0, 0);
    check("setup_wo", a_make_file(D "/wo", "hello", 0222) == 0, 0);
    check("setup_nx", a_mkdir(D "/nx", 0755) == 0 && a_make_file(D "/nx/f", "hello", 0666) == 0, 0);
    check("setup_nx_mode", syscall2(SYS_chmod, D "/nx", 0666) == 0, 0);
    check("setup_rodir", a_mkdir(D "/rodir", 0755) == 0 && a_make_file(D "/rodir/f", "hello", 0666) == 0, 0);
    check("setup_rodir_mode", syscall2(SYS_chmod, D "/rodir", 0555) == 0, 0);
    check("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/p", S_IFIFO | 0444, 0) == 0, 0);
    if (failed) return finish();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, checks());
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    /* Cleanup; the child printed the results. */
    syscall2(SYS_chmod, D "/nx", 0755);
    syscall2(SYS_chmod, D "/rodir", 0755);
    a_stat_path(D "/nx/f", &st);
    i64 nx_size = st.st_size;
    a_unlink(D "/nx/f");
    a_unlink(D "/rodir/f");
    a_rmdir(D "/nx");
    a_rmdir(D "/rodir");
    a_unlink(D "/ro");
    a_unlink(D "/none_bits");
    a_unlink(D "/wo");
    a_unlink(D "/p");
    a_rmdir(D);
    if (nx_size != 5) return 1;
    return A_WIFEXITED(status) ? A_WEXITSTATUS(status) : 1;
}
