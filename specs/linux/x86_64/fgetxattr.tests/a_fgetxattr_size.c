/* size is a size_t compared in full: values above 65536 (65537, 2^32,
 * 2^32 + 1, SIZE_MAX) act as a large buffer, never E2BIG. Only the value's
 * bytes must be writable: a buffer ending exactly where its mapping ends
 * works with a larger size; NULL or unmapped buffers, or one running into
 * an unmapped page, are EFAULT once the value fits. A size smaller than
 * the value is ERANGE even with a NULL buffer, a missing attribute
 * ENODATA even with a NULL buffer, and size 0 never touches the buffer. */
#include "a_sys.h"

#define SYS_mmap 9
#define SYS_munmap 11
#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define ENODATA 61
#define D "/tmp/a_fgetxattr_size"

static char buf[64];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    expect("setup_set", syscall5(SYS_fsetxattr, fd, "user.a", "abcd", 4, 0), 0);
    expect("size_65536", syscall4(SYS_fgetxattr, fd, "user.a", buf, 65536), 4);
    expect("size_65537", syscall4(SYS_fgetxattr, fd, "user.a", buf, 65537), 4);
    expect("size_2_32", syscall4(SYS_fgetxattr, fd, "user.a", buf, 0x100000000ULL), 4);
    expect("size_2_32_plus_1", syscall4(SYS_fgetxattr, fd, "user.a", buf, 0x100000001ULL), 4);
    expect("size_max", syscall4(SYS_fgetxattr, fd, "user.a", buf, 0xffffffffffffffffULL), 4);
    check("size_max_value", mem_eq(buf, "abcd", 4), 0);
    expect("null_buffer", syscall4(SYS_fgetxattr, fd, "user.a", 0, 4), -EFAULT);
    expect("low_buffer", syscall4(SYS_fgetxattr, fd, "user.a", (void *)8, 64), -EFAULT);
    expect("null_buffer_too_small", syscall4(SYS_fgetxattr, fd, "user.a", 0, 3), -ERANGE);
    expect("null_buffer_missing", syscall4(SYS_fgetxattr, fd, "user.b", 0, 64), -ENODATA);
    expect("null_buffer_size0", syscall4(SYS_fgetxattr, fd, "user.a", 0, 0), 4);
    i64 m = syscall6(SYS_mmap, 0, 8192, 3, 0x22, -1, 0); /* PROT_READ|WRITE, MAP_PRIVATE|ANONYMOUS */
    check("setup_mmap", m > 0, m);
    char *page = (char *)m;
    expect("setup_unmap_second", syscall2(SYS_munmap, page + 4096, 4096), 0);
    expect("edge_exact", syscall4(SYS_fgetxattr, fd, "user.a", page + 4092, 4), 4);
    check("edge_value", mem_eq(page + 4092, "abcd", 4), 0);
    expect("edge_exact_larger_size", syscall4(SYS_fgetxattr, fd, "user.a", page + 4092, 4096), 4);
    expect("edge_over", syscall4(SYS_fgetxattr, fd, "user.a", page + 4093, 4), -EFAULT);
    expect("unmapped", syscall4(SYS_fgetxattr, fd, "user.a", page + 4096, 4), -EFAULT);
    expect("unmapped_size0", syscall4(SYS_fgetxattr, fd, "user.a", page + 4096, 0), 4);
    expect("cleanup_unmap", syscall2(SYS_munmap, page, 4096), 0);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
