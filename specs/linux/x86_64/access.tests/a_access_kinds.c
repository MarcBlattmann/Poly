/* access on other kinds of files: a FIFO is checked by its mode and not
 * opened (no blocking); /dev/null (0666) passes R_OK and W_OK and fails
 * X_OK for everyone, root included; a directory without write bits fails
 * W_OK for a normal user. Nothing about the file changes. */
#include "a_sys.h"

#define D "/tmp/a_access_kinds"
#define SYS_mknodat 259

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/p", S_IFIFO | 0644, 0), 0);
    expect("fifo_f", syscall2(SYS_access, D "/p", F_OK), 0);
    expect("fifo_rw", syscall2(SYS_access, D "/p", R_OK | W_OK), 0);
    expect("fifo_x", syscall2(SYS_access, D "/p", X_OK), -EACCES);
    expect("devnull_rw", syscall2(SYS_access, "/dev/null", R_OK | W_OK), 0);
    expect("devnull_x", syscall2(SYS_access, "/dev/null", X_OK), -EACCES);
    expect("setup_rodir", a_mkdir(D "/ro", 0555), 0);
    expect("rodir_rx", syscall2(SYS_access, D "/ro", R_OK | X_OK), 0);
    expect("rodir_w", syscall2(SYS_access, D "/ro", W_OK), root ? 0 : -EACCES);

    expect("setup_file", a_make_file(D "/f", "abc", 0644), 0);
    struct a_stat a, b;
    expect("stat_before", a_stat_path(D "/f", &a), 0);
    for (int i = 0; i < 20; i++) syscall2(SYS_access, D "/f", R_OK | W_OK);
    expect("stat_after", a_stat_path(D "/f", &b), 0);
    check("no_change", mem_eq(&a, &b, sizeof a), 0);

    expect("cleanup_fifo", a_unlink(D "/p"), 0);
    expect("cleanup_ro", a_rmdir(D "/ro"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
