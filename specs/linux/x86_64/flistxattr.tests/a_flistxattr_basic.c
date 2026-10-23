/* flistxattr (196) on an open regular file: the names come back as
 * NUL-terminated strings back to back, the return value is the list's
 * length (names plus one NUL each). Size 0 returns the length without
 * copying; a fresh file lists nothing (0). Zero-length values are listed,
 * replacing a value does not duplicate its name, a removed name is gone.
 * The order is unspecified, so only membership is checked. */
#include "a_sys.h"

#define SYS_fsetxattr 190
#define SYS_flistxattr 196
#define SYS_fremovexattr 199
#define D "/tmp/a_flistxattr_basic"

static char buf[256];
static i64 len;

static i64 flist(i64 fd) { return len = syscall3(SYS_flistxattr, fd, buf, sizeof buf); }

static int has(const char *name) {
    int n = 0;
    for (i64 i = 0; i < len; i += str_len(buf + i) + 1)
        if (str_len(buf + i) == str_len(name) && mem_eq(buf + i, name, str_len(name))) n++;
    return n;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    expect("fresh_empty", flist(fd), 0);
    expect("fresh_size0", syscall3(SYS_flistxattr, fd, 0, 0), 0);
    expect("setup_one", syscall5(SYS_fsetxattr, fd, "user.one", "1", 1, 0), 0);
    expect("one", flist(fd), 9);
    check("one_bytes", mem_eq(buf, "user.one\0", 9), 0);
    expect("setup_empty_value", syscall5(SYS_fsetxattr, fd, "user.e", "", 0, 0), 0);
    expect("setup_long", syscall5(SYS_fsetxattr, fd, "user.longer_name", "xyz", 3, 0), 0);
    expect("three", flist(fd), 9 + 7 + 17);
    expect("three_one", has("user.one"), 1);
    expect("three_e", has("user.e"), 1);
    expect("three_long", has("user.longer_name"), 1);
    check("three_last_nul", buf[len - 1] == 0, buf[len - 1]);
    expect("three_size0", syscall3(SYS_flistxattr, fd, 0, 0), 33);
    expect("setup_replace", syscall5(SYS_fsetxattr, fd, "user.one", "2222", 4, 0), 0);
    expect("replace_same_length", flist(fd), 33);
    expect("replace_no_dup", has("user.one"), 1);
    expect("setup_remove", syscall2(SYS_fremovexattr, fd, "user.e"), 0);
    expect("removed", flist(fd), 26);
    expect("removed_gone", has("user.e"), 0);
    expect("removed_others", has("user.one") + has("user.longer_name"), 2);
    expect("cleanup_close", a_close(fd), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
