/* listxattr: listing needs no permission on the file itself, only search
 * permission on the directories leading to it. The owner lists a 0000 file
 * and a 0000 directory; flistxattr on an O_WRONLY descriptor of a 0200 file
 * lists too. A file in a 0300 directory (search, no read) is listed. All
 * capabilities are dropped first, so a 0000 directory blocks the walk to its
 * children (EACCES) for root as well. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_listxattr 194
#define SYS_flistxattr 196
#define D "/tmp/a_listxattr_perm"

/* Drop every capability, so root fails the permission checks like any
 * other user (and the results do not depend on who runs the test). */
static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static char buf[256];

static i64 list(const char *p) { return syscall3(SYS_listxattr, p, buf, sizeof buf); }

int test_main(void) {
    expect("setup_drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/none", "", 0644), 0);
    expect("setup_wo", a_make_file(D "/wo", "", 0644), 0);
    expect("setup_dir", a_mkdir(D "/dir", 0755), 0);
    expect("setup_set_none", syscall5(SYS_setxattr, D "/none", "user.n", "x", 1, 0), 0);
    expect("setup_set_wo", syscall5(SYS_setxattr, D "/wo", "user.w", "x", 1, 0), 0);
    expect("setup_set_dir", syscall5(SYS_setxattr, D "/dir", "user.d", "x", 1, 0), 0);
    expect("setup_mode_none", syscall2(SYS_chmod, D "/none", 0), 0);
    expect("setup_mode_wo", syscall2(SYS_chmod, D "/wo", 0200), 0);
    expect("setup_mode_dir", syscall2(SYS_chmod, D "/dir", 0), 0);
    expect("mode_0000_file", list(D "/none"), 7);
    check("mode_0000_bytes", mem_eq(buf, "user.n\0", 7), 0);
    expect("mode_0000_size0", syscall3(SYS_listxattr, D "/none", 0, 0), 7);
    expect("mode_0200_file", list(D "/wo"), 7);
    expect("mode_0000_dir", list(D "/dir"), 7);
    expect("mode_0000_dir_slash", list(D "/dir/"), 7);
    expect("mode_0000_dir_child", list(D "/dir/x"), -EACCES);
    i64 fd = a_open(D "/wo", O_WRONLY, 0);
    check("setup_open_wo", fd >= 0, fd);
    expect("wronly_fd", syscall3(SYS_flistxattr, fd, buf, sizeof buf), 7);
    check("wronly_fd_bytes", mem_eq(buf, "user.w\0", 7), 0);
    a_close(fd);
    expect("setup_mode_d_0300", syscall2(SYS_chmod, D, 0300), 0);
    expect("search_only_parent", list(D "/none"), 7);
    expect("setup_mode_d_0755", syscall2(SYS_chmod, D, 0755), 0);
    expect("setup_mode_dir_back", syscall2(SYS_chmod, D "/dir", 0755), 0);
    expect("cleanup_none", a_unlink(D "/none"), 0);
    expect("cleanup_wo", a_unlink(D "/wo"), 0);
    expect("cleanup_dir", a_rmdir(D "/dir"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
