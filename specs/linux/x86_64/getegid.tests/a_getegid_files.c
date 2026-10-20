/* getegid: new files and directories get the caller's ids as owner and
 * group (st_gid here); after execve the id is unchanged. */
#include "a_sys.h"

#define D "/tmp/a_getegid_files"

int test_main(void) {
    if (a_argc() > 2 && str_eq(a_argv()[1], "again")) {
        passed = (int)a_atou(a_argv()[2]);
        expect("same_after_execve", syscall0(SYS_getegid), 0);
        return finish();
    }
    struct a_stat st;
    i64 id = syscall0(SYS_getegid);
    expect("mkdir", a_mkdir(D, 0700), 0);
    expect("file", a_make_file(D "/f", "", 0644), 0);
    a_stat_path(D "/f", &st);
    expect("file_owned_by_id", st.st_gid, id);
    a_stat_path(D, &st);
    expect("dir_owned_by_id", st.st_gid, id);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_d", a_rmdir(D), 0);
    if (failed) return finish();
    char count[24];
    a_utoa(count, (u64)passed);
    char *argv[] = {"x", "again", count, 0};
    char *envp[] = {0};
    syscall3(SYS_execve, a_self_path(), argv, envp);
    expect("execve_returned", 1, 0);
    return finish();
}
