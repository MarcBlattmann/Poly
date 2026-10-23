/* llistxattr permissions (all capabilities dropped first, so root gets the
 * same results): no permission on the file itself is needed (a 0000 file
 * and a 0000 directory are listed), only search permission on the
 * directories on the way: a file or a symbolic link in a 0644 or 0000
 * directory is EACCES, in a 0300 (search, no read) directory fine. A link
 * pointing into an unsearchable directory still lists itself (empty), its
 * trailing-slash form is EACCES when the walk has to search that
 * directory. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_llistxattr 195
#define D "/tmp/a_llistxattr_perm"

static char buf[256];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 llist(const char *p) { return syscall3(SYS_llistxattr, p, buf, sizeof buf); }

int test_main(void) {
    expect("setup_drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_inner", a_mkdir(D "/sub/inner", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0644), 0);
    expect("setup_set_file", syscall5(SYS_setxattr, D "/sub/f", "user.f", "x", 1, 0), 0);
    expect("setup_set_inner", syscall5(SYS_setxattr, D "/sub/inner", "user.in", "x", 1, 0), 0);
    expect("setup_link_in", syscall2(SYS_symlink, "f", D "/sub/l"), 0);
    expect("setup_link_out", syscall2(SYS_symlink, "sub/f", D "/out"), 0);
    expect("setup_link_out_dir", syscall2(SYS_symlink, "sub/inner", D "/outd"), 0);
    expect("setup_file_0000", syscall2(SYS_chmod, D "/sub/f", 0), 0);
    expect("setup_inner_0000", syscall2(SYS_chmod, D "/sub/inner", 0), 0);
    expect("file_0000", llist(D "/sub/f"), 7);
    expect("dir_0000", llist(D "/sub/inner"), 8);
    expect("setup_sub_0300", syscall2(SYS_chmod, D "/sub", 0300), 0);
    expect("search_only_file", llist(D "/sub/f"), 7);
    expect("search_only_link", llist(D "/sub/l"), 0);
    expect("setup_sub_0644", syscall2(SYS_chmod, D "/sub", 0644), 0);
    expect("no_search_file", llist(D "/sub/f"), -EACCES);
    expect("no_search_link", llist(D "/sub/l"), -EACCES);
    expect("no_search_missing", llist(D "/sub/nope"), -EACCES);
    expect("no_search_bad_buffer", syscall3(SYS_llistxattr, D "/sub/f", (void *)8, 100), -EACCES);
    expect("no_search_dir_itself", llist(D "/sub"), 0);
    expect("link_into_no_search", llist(D "/out"), 0);
    expect("link_into_no_search_slash", llist(D "/outd/"), -EACCES);
    expect("setup_sub_0000", syscall2(SYS_chmod, D "/sub", 0), 0);
    expect("mode_0000_file", llist(D "/sub/f"), -EACCES);
    expect("mode_0000_link_out", llist(D "/out"), 0);
    expect("setup_sub_0755", syscall2(SYS_chmod, D "/sub", 0755), 0);
    expect("setup_inner_0755", syscall2(SYS_chmod, D "/sub/inner", 0755), 0);
    expect("link_out_dir_slash", llist(D "/outd/"), 8);
    expect("cleanup_out", a_unlink(D "/out"), 0);
    expect("cleanup_outd", a_unlink(D "/outd"), 0);
    expect("cleanup_l", a_unlink(D "/sub/l"), 0);
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_inner", a_rmdir(D "/sub/inner"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
