/* Value sizes through fsetxattr: up to 65536 bytes are accepted by the
 * call (how much a file system keeps differs; small values fit
 * everywhere); 65537, 2^32 + 1 and SIZE_MAX are E2BIG (size_t is compared
 * in full). With size > 0 an unreadable value (NULL, unmapped, running
 * into an unmapped page) is EFAULT; a buffer ending exactly at an unmapped
 * page works. With size 0 the pointer is never read. */
#include "a_sys.h"

#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define ENODATA 61
#define D "/tmp/a_fsetxattr_sizes"

static char buf[600];
static char val[600];

static i64 fd;

static i64 fset(const char *n, const void *v, i64 size) { return syscall5(SYS_fsetxattr, fd, n, v, size, 0); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    fd = a_open(D "/f", O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    for (int i = 0; i < 600; i++) val[i] = (char)(i * 7);
    expect("size_500", fset("user.big", val, 500), 0);
    expect("size_500_get", syscall4(SYS_fgetxattr, fd, "user.big", buf, sizeof buf), 500);
    check("size_500_bytes", mem_eq(buf, val, 500), 0);
    expect("size_65537", fset("user.e", val, 65537), -E2BIG);
    expect("size_2p32p1", fset("user.e", val, 0x100000001LL), -E2BIG);
    expect("size_max", fset("user.e", val, -1), -E2BIG);
    expect("e2big_absent", syscall4(SYS_fgetxattr, fd, "user.e", buf, sizeof buf), -ENODATA);
    expect("null_value", fset("user.n", 0, 1), -EFAULT);
    expect("bad_value", fset("user.n", (void *)16, 4), -EFAULT);
    expect("null_65536", fset("user.n", 0, 65536), -EFAULT);
    expect("efault_absent", syscall4(SYS_fgetxattr, fd, "user.n", buf, sizeof buf), -ENODATA);
    expect("size0_bad_ptr", fset("user.z", (void *)16, 0), 0);
    expect("size0_get", syscall4(SYS_fgetxattr, fd, "user.z", buf, sizeof buf), 0);
    char *m = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("setup_mmap", (i64)m > 0, (i64)m);
    expect("setup_unmap", syscall2(SYS_munmap, m + 4096, 4096), 0);
    m[4093] = 'e', m[4094] = 'n', m[4095] = 'd';
    expect("ends_at_hole", fset("user.p", m + 4093, 3), 0);
    expect("ends_at_hole_get", syscall4(SYS_fgetxattr, fd, "user.p", buf, sizeof buf), 3);
    check("ends_at_hole_bytes", mem_eq(buf, "end", 3), 0);
    expect("runs_into_hole", fset("user.q", m + 4093, 4), -EFAULT);
    expect("unmapped", fset("user.q", m + 4096, 1), -EFAULT);
    a_close(fd);
    a_unlink(D "/f");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
