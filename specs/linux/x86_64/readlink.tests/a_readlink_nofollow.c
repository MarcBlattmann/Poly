/* readlink: the last component is never followed (a link to a link gives
 * the first link's text, a dangling link still has text), but links
 * earlier in the path are; a trailing slash makes the last one followed. */
#include "a_sys.h"

#define D "/tmp/a_readlink_nofollow"

static i64 rl(const char *p, char *buf, i64 n) { return syscall3(SYS_readlink, p, buf, n); }

int test_main(void) {
    char buf[64];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_inner", syscall2(SYS_symlink, "inner-text", D "/sub/in"), 0);
    expect("setup_l1", syscall2(SYS_symlink, "l2", D "/l1"), 0);
    expect("setup_l2", syscall2(SYS_symlink, "sub", D "/l2"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "no/such/thing", D "/dang"), 0);
    expect("setup_tofile_target", a_make_file(D "/f", "", 0644), 0);
    expect("setup_tofile", syscall2(SYS_symlink, "f", D "/tofile"), 0);
    memset(buf, 'z', sizeof buf);
    expect("link_to_link", rl(D "/l1", buf, sizeof buf), 2);
    check("link_to_link_text", mem_eq(buf, "l2", 2), buf[0]);
    expect("link_to_dir", rl(D "/l2", buf, sizeof buf), 3);
    check("link_to_dir_text", mem_eq(buf, "sub", 3), buf[0]);
    expect("dangling", rl(D "/dang", buf, sizeof buf), 13);
    check("dangling_text", mem_eq(buf, "no/such/thing", 13), buf[0]);
    /* Links before the last component are followed (two levels here). */
    memset(buf, 'z', sizeof buf);
    expect("through_two_links", rl(D "/l1/in", buf, sizeof buf), 10);
    check("through_two_links_text", mem_eq(buf, "inner-text", 10), buf[0]);
    expect("through_dotdot", rl(D "/sub/../l1", buf, sizeof buf), 2);
    expect("through_link_dotdot", rl(D "/l1/../l2", buf, sizeof buf), 3);
    /* A trailing slash (or "/.") follows the last link. */
    expect("slash_on_link_to_dir", rl(D "/l2/", buf, sizeof buf), -EINVAL);
    expect("slash_on_chain_to_dir", rl(D "/l1/", buf, sizeof buf), -EINVAL);
    expect("dot_on_link_to_dir", rl(D "/l2/.", buf, sizeof buf), -EINVAL);
    expect("slash_on_dangling", rl(D "/dang/", buf, sizeof buf), -ENOENT);
    expect("slash_on_link_to_file", rl(D "/tofile/", buf, sizeof buf), -ENOTDIR);
    expect("through_link_to_file", rl(D "/tofile/x", buf, sizeof buf), -ENOTDIR);
    expect("through_dangling", rl(D "/dang/x", buf, sizeof buf), -ENOENT);
    expect("cleanup_inner", a_unlink(D "/sub/in"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_l1", a_unlink(D "/l1"), 0);
    expect("cleanup_l2", a_unlink(D "/l2"), 0);
    expect("cleanup_dang", a_unlink(D "/dang"), 0);
    expect("cleanup_tofile", a_unlink(D "/tofile"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
