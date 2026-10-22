/* readlink: relative paths start at the working directory; the text is
 * returned as stored, never resolved against anything. */
#include "a_sys.h"

#define D "/tmp/a_readlink_relative"

static i64 rl(const char *p, char *buf, i64 n) { return syscall3(SYS_readlink, p, buf, n); }

int test_main(void) {
    char buf[64];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_l", syscall2(SYS_symlink, "../up/./x//y", D "/sub/l"), 0);
    expect("chdir", syscall1(SYS_chdir, D), 0);
    memset(buf, 'z', sizeof buf);
    expect("relative", rl("sub/l", buf, sizeof buf), 12);
    check("relative_text_as_stored", mem_eq(buf, "../up/./x//y", 12), buf[0]);
    expect("dot_relative", rl("./sub/l", buf, sizeof buf), 12);
    expect("dotdot_relative", rl("../a_readlink_relative/sub/l", buf, sizeof buf), 12);
    expect("double_slash", rl("sub//l", buf, sizeof buf), 12);
    expect("chdir_sub", syscall1(SYS_chdir, "sub"), 0);
    expect("plain_name", rl("l", buf, sizeof buf), 12);
    expect("plain_missing", rl("m", buf, sizeof buf), -ENOENT);
    expect("cwd_dot", rl(".", buf, sizeof buf), -EINVAL);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("from_root_relative", rl("tmp/a_readlink_relative/sub/l", buf, sizeof buf), 12);
    expect("old_cwd_gone", rl("l", buf, sizeof buf), -ENOENT);
    expect("cleanup_l", a_unlink(D "/sub/l"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
