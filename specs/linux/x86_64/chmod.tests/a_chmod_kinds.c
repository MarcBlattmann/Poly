/* chmod on a FIFO and a socket node: the permission bits change and the
 * file type stays; type bits in the mode argument are ignored. Opening a FIFO
 * is not needed (chmod never opens the file). */
#include "a_sys.h"

#define D "/tmp/a_chmod_kinds"
#define SYS_mknodat 259

static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/p", S_IFIFO | 0644, 0), 0);
    expect("setup_sock", syscall4(SYS_mknodat, AT_FDCWD, D "/s", S_IFSOCK | 0644, 0), 0);
    expect("fifo_chmod", syscall2(SYS_chmod, D "/p", 0600), 0);
    expect("fifo_mode", mode_of(D "/p"), S_IFIFO | 0600);
    expect("fifo_chmod_reg_type", syscall2(SYS_chmod, D "/p", S_IFREG | 0666), 0);
    expect("fifo_still_fifo", mode_of(D "/p"), S_IFIFO | 0666);
    expect("fifo_chmod_sticky", syscall2(SYS_chmod, D "/p", 01640), 0);
    expect("fifo_sticky", mode_of(D "/p"), S_IFIFO | 01640);
    expect("sock_chmod", syscall2(SYS_chmod, D "/s", 0700), 0);
    expect("sock_mode", mode_of(D "/s"), S_IFSOCK | 0700);
    expect("sock_chmod_0", syscall2(SYS_chmod, D "/s", S_IFDIR), 0);
    expect("sock_mode_0", mode_of(D "/s"), S_IFSOCK);
    expect("fifo_link_setup", syscall3(SYS_symlinkat, "p", AT_FDCWD, D "/lp"), 0);
    expect("fifo_via_link", syscall2(SYS_chmod, D "/lp", 0604), 0);
    expect("fifo_via_link_mode", mode_of(D "/p"), S_IFIFO | 0604);
    int ok = a_unlink(D "/p") == 0 && a_unlink(D "/s") == 0 && a_unlink(D "/lp") == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
