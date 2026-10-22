/* readlink: anything that is not a symbolic link is EINVAL: regular
 * files, directories, devices, FIFOs, "." and "..". */
#include "a_sys.h"

#define D "/tmp/a_readlink_not_link"
#define SYS_mknod 133

static i64 rl(const char *p, char *buf, i64 n) { return syscall3(SYS_readlink, p, buf, n); }

int test_main(void) {
    char buf[32];
    memset(buf, 'z', sizeof buf);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("setup_empty_file", a_make_file(D "/e", "", 0000), 0);
    expect("setup_fifo", syscall3(SYS_mknod, D "/p", S_IFIFO | 0644, 0), 0);
    expect("setup_subdir", a_mkdir(D "/sub", 0755), 0);
    expect("regular_file", rl(D "/f", buf, sizeof buf), -EINVAL);
    expect("mode_0000_file", rl(D "/e", buf, sizeof buf), -EINVAL);
    expect("fifo", rl(D "/p", buf, sizeof buf), -EINVAL);
    expect("directory", rl(D "/sub", buf, sizeof buf), -EINVAL);
    expect("root_dir", rl("/", buf, sizeof buf), -EINVAL);
    expect("dot", rl(D "/.", buf, sizeof buf), -EINVAL);
    expect("dotdot", rl(D "/sub/..", buf, sizeof buf), -EINVAL);
    expect("trailing_slash_dir", rl(D "/sub/", buf, sizeof buf), -EINVAL);
    expect("dev_null", rl("/dev/null", buf, sizeof buf), -EINVAL);
    expect("tmp", rl("/tmp", buf, sizeof buf), -EINVAL);
    check("buffer_untouched", buf[0] == 'z' && buf[31] == 'z', buf[0]);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_e", a_unlink(D "/e"), 0);
    expect("cleanup_p", a_unlink(D "/p"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
