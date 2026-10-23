/* llistxattr (195): like listxattr but a final symbolic link is not
 * followed. A normal user cannot give a link attributes, so a live,
 * dangling or looping link lists as empty (0, also with size 0 and with an
 * unmapped buffer). Links in earlier components are followed, a trailing
 * slash follows the link (directory: its list; file: ENOTDIR); on files and
 * directories it gives what listxattr gives. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_lsetxattr 189
#define SYS_listxattr 194
#define SYS_llistxattr 195
#define D "/tmp/a_listxattr_llistxattr"
#define BAD ((void *)8)

static char buf[256];

static i64 llist(const char *p) { return syscall3(SYS_llistxattr, p, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0644), 0);
    expect("setup_set_file", syscall5(SYS_setxattr, D "/sub/f", "user.ff", "x", 1, 0), 0);
    expect("setup_set_dir", syscall5(SYS_setxattr, D "/sub", "user.dd", "x", 1, 0), 0);
    expect("setup_link", syscall2(SYS_symlink, "sub/f", D "/lf"), 0);
    expect("setup_link_dir", syscall2(SYS_symlink, "sub", D "/ld"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nothing", D "/dang"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);
    expect("link_user_set_eperm", syscall5(SYS_lsetxattr, D "/lf", "user.l", "x", 1, 0), -EPERM);
    expect("file", llist(D "/sub/f"), 8);
    check("file_bytes", mem_eq(buf, "user.ff\0", 8), 0);
    expect("dir", llist(D "/sub"), 8);
    expect("link_not_followed", llist(D "/lf"), 0);
    expect("link_size0", syscall3(SYS_llistxattr, D "/lf", 0, 0), 0);
    expect("link_bad_buffer", syscall3(SYS_llistxattr, D "/lf", BAD, 100), 0);
    expect("listxattr_follows", syscall3(SYS_listxattr, D "/lf", buf, sizeof buf), 8);
    expect("dangling", llist(D "/dang"), 0);
    expect("loop", llist(D "/loop"), 0);
    expect("link_dir", llist(D "/ld"), 0);
    expect("link_dir_slash", llist(D "/ld/"), 8);
    check("link_dir_slash_bytes", mem_eq(buf, "user.dd\0", 8), 0);
    expect("link_file_slash", llist(D "/lf/"), -ENOTDIR);
    expect("through_link", llist(D "/ld/f"), 8);
    check("through_link_bytes", mem_eq(buf, "user.ff\0", 8), 0);
    expect("dangling_slash", llist(D "/dang/"), -ENOENT);
    expect("missing", llist(D "/nope"), -ENOENT);
    expect("cleanup_lf", a_unlink(D "/lf"), 0);
    expect("cleanup_ld", a_unlink(D "/ld"), 0);
    expect("cleanup_dang", a_unlink(D "/dang"), 0);
    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
