/* fchmodat: the new mode is what access checks see; an open descriptor
 * keeps working after its file's mode drops to 0. */
#include "a_sys.h"

#define D "/tmp/a_fchmodat_effect"

int test_main(void) {
    char c;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "z", 0755), 0);
    expect("exec_allowed", syscall3(SYS_faccessat, AT_FDCWD, D "/f", X_OK), 0);
    expect("chmod_0644", syscall3(SYS_fchmodat, AT_FDCWD, D "/f", 0644), 0);
    expect("exec_now_refused", syscall3(SYS_faccessat, AT_FDCWD, D "/f", X_OK), -EACCES);
    expect("chmod_0001", syscall3(SYS_fchmodat, AT_FDCWD, D "/f", 0001), 0);
    expect("other_exec_counts", syscall3(SYS_faccessat, AT_FDCWD, D "/f", X_OK), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    expect("chmod_0_while_open", syscall3(SYS_fchmodat, AT_FDCWD, D "/f", 0), 0);
    expect("open_fd_still_reads", a_read(fd, &c, 1), 1);
    expect("open_fd_still_writes", a_write(fd, "y", 1), 1);
    a_close(fd);
    /* Each mode in turn round-trips through stat. */
    int ok = 1;
    for (i64 m = 0; m < 01000; m += 0111) {
        struct a_stat st;
        ok &= syscall3(SYS_fchmodat, AT_FDCWD, D "/f", m) == 0;
        ok &= a_stat_path(D "/f", &st) == 0 && st.st_mode == (S_IFREG | m);
    }
    check("modes_round_trip", ok, 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
