/* llistxattr does not follow a final symbolic link: it lists the link's
 * own names, and a normal user cannot give a link any (lsetxattr of user.*
 * on a link is EPERM), so every link lists as empty: relative and absolute
 * targets, links to files and directories (both with names), dangling and
 * self-looping links, a link to a link. listxattr on the same paths lists
 * the target's names (or fails on the dangling and looping links). All
 * capabilities are dropped first, so root sees the same. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_lsetxattr 189
#define SYS_listxattr 194
#define SYS_llistxattr 195
#define ELOOP 40
#define D "/tmp/a_llistxattr_link"

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
static i64 list(const char *p) { return syscall3(SYS_listxattr, p, buf, sizeof buf); }

int test_main(void) {
    expect("setup_drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0644), 0);
    expect("setup_set_file", syscall5(SYS_setxattr, D "/sub/f", "user.ff", "x", 1, 0), 0);
    expect("setup_set_dir", syscall5(SYS_setxattr, D "/sub", "user.ddd", "x", 1, 0), 0);
    expect("setup_rel", syscall2(SYS_symlink, "sub/f", D "/rel"), 0);
    expect("setup_abs", syscall2(SYS_symlink, D "/sub/f", D "/abs"), 0);
    expect("setup_dir", syscall2(SYS_symlink, "sub", D "/dir"), 0);
    expect("setup_dir_abs", syscall2(SYS_symlink, D "/sub", D "/dirabs"), 0);
    expect("setup_chain", syscall2(SYS_symlink, "rel", D "/chain"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nothing", D "/dang"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);
    expect("lsetxattr_user_link_eperm", syscall5(SYS_lsetxattr, D "/rel", "user.l", "x", 1, 0), -EPERM);
    expect("rel", llist(D "/rel"), 0);
    expect("abs", llist(D "/abs"), 0);
    expect("dir", llist(D "/dir"), 0);
    expect("dir_abs", llist(D "/dirabs"), 0);
    expect("chain", llist(D "/chain"), 0);
    expect("dangling", llist(D "/dang"), 0);
    expect("loop", llist(D "/loop"), 0);
    expect("target_file", llist(D "/sub/f"), 8);
    check("target_file_bytes", mem_eq(buf, "user.ff\0", 8), 0);
    expect("target_dir", llist(D "/sub"), 9);
    check("target_dir_bytes", mem_eq(buf, "user.ddd\0", 9), 0);
    expect("listxattr_rel", list(D "/rel"), 8);
    expect("listxattr_abs", list(D "/abs"), 8);
    expect("listxattr_dir", list(D "/dir"), 9);
    expect("listxattr_chain", list(D "/chain"), 8);
    expect("listxattr_dangling", list(D "/dang"), -ENOENT);
    expect("listxattr_loop", list(D "/loop"), -ELOOP);
    expect("setup_unlink_target", a_unlink(D "/sub/f"), 0);
    expect("now_dangling", llist(D "/rel"), 0);
    expect("cleanup_rel", a_unlink(D "/rel"), 0);
    expect("cleanup_abs", a_unlink(D "/abs"), 0);
    expect("cleanup_dir", a_unlink(D "/dir"), 0);
    expect("cleanup_dirabs", a_unlink(D "/dirabs"), 0);
    expect("cleanup_chain", a_unlink(D "/chain"), 0);
    expect("cleanup_dang", a_unlink(D "/dang"), 0);
    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
