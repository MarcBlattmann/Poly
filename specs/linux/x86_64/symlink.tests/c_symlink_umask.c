/* symlink: the umask does not apply to a symbolic link; its mode is 0777
 * under umask 0, 022, 077 and 0777, and the umask itself is unchanged by
 * the call. */
#include "a_sys.h"

#define D "/tmp/c_symlink_umask"

static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    syscall1(SYS_umask, 0);
    expect("umask_0", syscall2(SYS_symlink, "x", D "/u0"), 0);
    expect("umask_0_mode", mode_of(D "/u0"), S_IFLNK | 0777);
    syscall1(SYS_umask, 022);
    expect("umask_022", syscall2(SYS_symlink, "x", D "/u22"), 0);
    expect("umask_022_mode", mode_of(D "/u22"), S_IFLNK | 0777);
    syscall1(SYS_umask, 077);
    expect("umask_077", syscall2(SYS_symlink, "x", D "/u77"), 0);
    expect("umask_077_mode", mode_of(D "/u77"), S_IFLNK | 0777);
    syscall1(SYS_umask, 0777);
    expect("umask_0777", syscall2(SYS_symlink, "x", D "/u777"), 0);
    expect("umask_0777_mode", mode_of(D "/u777"), S_IFLNK | 0777);
    expect("umask_unchanged", syscall1(SYS_umask, 022), 0777);
    int ok = a_unlink(D "/u0") == 0;
    ok &= a_unlink(D "/u22") == 0;
    ok &= a_unlink(D "/u77") == 0;
    ok &= a_unlink(D "/u777") == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
