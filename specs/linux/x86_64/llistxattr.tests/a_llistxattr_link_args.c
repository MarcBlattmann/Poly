/* llistxattr on a symbolic link (whose list is empty for a normal user):
 * the size and the buffer are never checked, so size 0, size 1, a NULL or
 * unmapped buffer with any size, and SIZE_MAX with a NULL buffer all
 * return 0. The same arguments on the target file give the needed length,
 * ERANGE and EFAULT. All capabilities are dropped first. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_llistxattr 195
#define D "/tmp/a_llistxattr_link_args"
#define BAD ((void *)8)

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 ll(const char *p, void *b, i64 n) { return syscall3(SYS_llistxattr, p, b, n); }

int test_main(void) {
    expect("setup_drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.abc", "x", 1, 0), 0);
    expect("setup_link", syscall2(SYS_symlink, "f", D "/l"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "none", D "/dang"), 0);
    expect("link_size0", ll(D "/l", 0, 0), 0);
    expect("link_size1", ll(D "/l", buf, 1), 0);
    expect("link_null_buffer", ll(D "/l", 0, 100), 0);
    expect("link_bad_buffer", ll(D "/l", BAD, 100), 0);
    expect("link_null_size_max", ll(D "/l", 0, -1L), 0);
    expect("link_size_2_32", ll(D "/l", buf, 1L << 32), 0);
    expect("dangling_size0", ll(D "/dang", 0, 0), 0);
    expect("dangling_bad_buffer", ll(D "/dang", BAD, 100), 0);
    expect("file_size0", ll(D "/f", 0, 0), 9);
    expect("file_size1", ll(D "/f", buf, 1), -ERANGE);
    expect("file_null_buffer", ll(D "/f", 0, 100), -EFAULT);
    expect("file_bad_buffer", ll(D "/f", BAD, 100), -EFAULT);
    expect("file_null_size_max", ll(D "/f", 0, -1L), -EFAULT);
    expect("file_size_2_32", ll(D "/f", buf, 1L << 32), 9);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_dang", a_unlink(D "/dang"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
