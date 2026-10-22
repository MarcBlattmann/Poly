/* readlink: path errors: missing names, non-directories in the middle,
 * the empty path, unreadable pointers, an unwritable buffer. */
#include "a_sys.h"

#define D "/tmp/a_readlink_errors"

static i64 rl(const char *p, char *buf, i64 n) { return syscall3(SYS_readlink, p, buf, n); }

int test_main(void) {
    char buf[32];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall2(SYS_symlink, "target", D "/l"), 0);
    expect("missing", rl(D "/none", buf, sizeof buf), -ENOENT);
    expect("missing_dir", rl(D "/nodir/l", buf, sizeof buf), -ENOENT);
    expect("missing_then_dotdot", rl(D "/nodir/../l", buf, sizeof buf), -ENOENT);
    expect("file_as_dir", rl(D "/f/l", buf, sizeof buf), -ENOTDIR);
    expect("file_trailing_slash", rl(D "/f/", buf, sizeof buf), -ENOTDIR);
    expect("empty_path", rl("", buf, sizeof buf), -ENOENT);
    expect("null_path", rl((const char *)0, buf, sizeof buf), -EFAULT);
    expect("low_path", rl((const char *)16, buf, sizeof buf), -EFAULT);
    expect("kernel_path", rl((const char *)0xffff800000000000UL, buf, sizeof buf), -EFAULT);
    expect("null_buf", rl(D "/l", (char *)0, sizeof buf), -EFAULT);
    expect("low_buf", rl(D "/l", (char *)16, sizeof buf), -EFAULT);
    expect("kernel_buf", rl(D "/l", (char *)0xffff800000000000UL, sizeof buf), -EFAULT);
    /* The path is looked up before the buffer is touched. */
    expect("missing_before_bad_buf", rl(D "/none", (char *)16, sizeof buf), -ENOENT);
    expect("non_link_before_bad_buf", rl(D "/f", (char *)16, sizeof buf), -EINVAL);
    /* A read-only buffer cannot be written. */
    {
        char *ro = (char *)syscall6(SYS_mmap, 0, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        check("setup_ro_map", (i64)ro > 0, (i64)ro > 0);
        expect("readonly_buf", rl(D "/l", ro, 16), -EFAULT);
        check("readonly_buf_unchanged", ro[0] == 0, ro[0]);
        expect("cleanup_ro_map", syscall2(SYS_munmap, ro, 4096), 0);
    }
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
