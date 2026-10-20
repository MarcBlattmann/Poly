/* readlinkat: returns the link text without a NUL, truncated to the
 * buffer; EINVAL for things that are not links or a bad size. */
#include "a_sys.h"

#define D "/tmp/a_readlinkat_basic"

static i64 rl(i64 dfd, const char *p, char *buf, i64 n) { return syscall4(SYS_readlinkat, dfd, p, buf, n); }

int test_main(void) {
    char buf[64];
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "some/target", AT_FDCWD, D "/l"), 0);
    memset(buf, 'z', sizeof buf);
    expect("full_length", rl(AT_FDCWD, D "/l", buf, sizeof buf), 11);
    check("text", mem_eq(buf, "some/target", 11), buf[0]);
    check("no_nul_written", buf[11] == 'z', buf[11]);
    memset(buf, 'z', sizeof buf);
    expect("truncated_to_4", rl(AT_FDCWD, D "/l", buf, 4), 4);
    check("truncated_text", mem_eq(buf, "some", 4) && buf[4] == 'z', buf[4]);
    expect("size_1", rl(AT_FDCWD, D "/l", buf, 1), 1);
    expect("size_0_einval", rl(AT_FDCWD, D "/l", buf, 0), -EINVAL);
    expect("negative_size_einval", rl(AT_FDCWD, D "/l", buf, -1), -EINVAL);
    expect("regular_file_einval", rl(AT_FDCWD, D "/f", buf, sizeof buf), -EINVAL);
    expect("directory_einval", rl(AT_FDCWD, D, buf, sizeof buf), -EINVAL);
    expect("missing_enoent", rl(AT_FDCWD, D "/none", buf, sizeof buf), -ENOENT);
    expect("through_file_enotdir", rl(AT_FDCWD, D "/f/x", buf, sizeof buf), -ENOTDIR);
    expect("empty_path_enoent", rl(AT_FDCWD, "", buf, sizeof buf), -ENOENT);
    expect("bad_buffer_efault", rl(AT_FDCWD, D "/l", (char *)16, sizeof buf), -EFAULT);
    expect("bad_path_efault", rl(AT_FDCWD, (const char *)16, buf, sizeof buf), -EFAULT);
    /* The last component is not followed; earlier ones are. */
    expect("link_to_link", syscall3(SYS_symlinkat, "l", AT_FDCWD, D "/l2"), 0);
    expect("link_to_link_text", rl(AT_FDCWD, D "/l2", buf, sizeof buf), 1);
    check("link_to_link_not_followed", buf[0] == 'l', buf[0]);
    expect("old_readlink_syscall", syscall3(SYS_readlink, D "/l", buf, sizeof buf), 11);
    expect("cleanup_l2", a_unlink(D "/l2"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
