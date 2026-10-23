/* fgetxattr (193) reads an attribute through an open descriptor: it returns
 * the value's length and copies the value byte for byte (NUL bytes
 * included, no NUL added) to the start of the buffer, leaving the rest of
 * the buffer alone. Size 0 returns the length and copies nothing; a size
 * smaller than the value is ERANGE and copies nothing; an empty value
 * reads back as length 0. A missing name is ENODATA. */
#include "a_sys.h"

#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define ENODATA 61
#define D "/tmp/a_fgetxattr_basic"

static char buf[64];

static void fill(void) {
    for (int i = 0; i < 64; i++) buf[i] = '#';
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    expect("missing_before_set", syscall4(SYS_fgetxattr, fd, "user.a", buf, sizeof buf), -ENODATA);
    expect("setup_set", syscall5(SYS_fsetxattr, fd, "user.a", "a\0b\0", 4, 0), 0);
    expect("setup_set_empty", syscall5(SYS_fsetxattr, fd, "user.empty", "", 0, 0), 0);
    expect("setup_set_long", syscall5(SYS_fsetxattr, fd, "user.long", "0123456789abcdef", 16, 0), 0);
    fill();
    expect("read", syscall4(SYS_fgetxattr, fd, "user.a", buf, sizeof buf), 4);
    check("read_value_with_nuls", mem_eq(buf, "a\0b\0", 4), 0);
    check("read_rest_untouched", buf[4] == '#' && buf[63] == '#', buf[4]);
    fill();
    expect("read_exact_size", syscall4(SYS_fgetxattr, fd, "user.a", buf, 4), 4);
    check("exact_value", mem_eq(buf, "a\0b\0", 4), 0);
    fill();
    expect("size0_length", syscall4(SYS_fgetxattr, fd, "user.a", buf, 0), 4);
    check("size0_nothing_copied", buf[0] == '#', buf[0]);
    expect("size0_null_buffer", syscall4(SYS_fgetxattr, fd, "user.long", 0, 0), 16);
    expect("erange", syscall4(SYS_fgetxattr, fd, "user.long", buf, 15), -ERANGE);
    check("erange_nothing_copied", buf[0] == '#', buf[0]);
    expect("erange_size1", syscall4(SYS_fgetxattr, fd, "user.a", buf, 1), -ERANGE);
    expect("long_read", syscall4(SYS_fgetxattr, fd, "user.long", buf, sizeof buf), 16);
    check("long_value", mem_eq(buf, "0123456789abcdef", 16), 0);
    fill();
    expect("empty_value", syscall4(SYS_fgetxattr, fd, "user.empty", buf, sizeof buf), 0);
    check("empty_nothing_copied", buf[0] == '#', buf[0]);
    expect("empty_size0", syscall4(SYS_fgetxattr, fd, "user.empty", 0, 0), 0);
    expect("missing", syscall4(SYS_fgetxattr, fd, "user.b", buf, sizeof buf), -ENODATA);
    expect("case_differs", syscall4(SYS_fgetxattr, fd, "user.A", buf, sizeof buf), -ENODATA);
    expect("prefix_of_name", syscall4(SYS_fgetxattr, fd, "user.lon", buf, sizeof buf), -ENODATA);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
