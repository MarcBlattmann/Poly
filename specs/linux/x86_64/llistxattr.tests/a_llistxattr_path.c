/* llistxattr path errors: missing file or component and the empty path
 * ENOENT, a file used as a directory ENOTDIR, a 256-byte component or a
 * 4096-byte path ENAMETOOLONG (4095 bytes resolve), a NULL or unmapped
 * path pointer EFAULT. Path errors come before the size and the buffer
 * are looked at (a missing path with an unmapped buffer or size 0 is
 * ENOENT). ".", ".." and absolute paths work. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_llistxattr 195
#define ENAMETOOLONG 36
#define D "/tmp/a_llistxattr_path"
#define BAD ((void *)8)

static char buf[256];
static char path[4200];

static i64 llist(const char *p) { return syscall3(SYS_llistxattr, p, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.p", "", 0, 0), 0);
    expect("absolute", llist(D "/f"), 7);
    expect("dotdot", llist(D "/../a_llistxattr_path/f"), 7);
    expect("dot", llist(D "/./f"), 7);
    expect("missing", llist(D "/nope"), -ENOENT);
    expect("missing_component", llist(D "/nope/f"), -ENOENT);
    expect("missing_bad_buffer", syscall3(SYS_llistxattr, D "/nope", BAD, 100), -ENOENT);
    expect("missing_size0", syscall3(SYS_llistxattr, D "/nope", 0, 0), -ENOENT);
    expect("missing_size1", syscall3(SYS_llistxattr, D "/nope", buf, 1), -ENOENT);
    expect("empty_path", llist(""), -ENOENT);
    expect("null_path", llist(0), -EFAULT);
    expect("bad_path", llist(BAD), -EFAULT);
    expect("file_as_dir", llist(D "/f/x"), -ENOTDIR);
    expect("file_slash", llist(D "/f/"), -ENOTDIR);
    expect("file_slash_bad_buffer", syscall3(SYS_llistxattr, D "/f/", BAD, 100), -ENOTDIR);
    char *q = str_cpy(path, D "/");
    for (int i = 0; i < 256; i++) *q++ = 'n';
    *q = 0;
    expect("component_256", llist(path), -ENAMETOOLONG);
    path[str_len(D "/") + 255] = 0;
    expect("component_255_missing", llist(path), -ENOENT);
    for (int i = 0; i < 4095; i++) path[i] = (i % 2) ? '.' : '/';
    path[4095] = 0;
    expect("path_4095", llist(path), 0);
    path[4095] = '/';
    path[4096] = 0;
    expect("path_4096", llist(path), -ENAMETOOLONG);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
