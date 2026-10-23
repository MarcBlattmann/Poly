/* flistxattr buffer checks: only the list's bytes must be writable. A
 * buffer ending exactly at an unmapped page works with a larger size; one
 * running into the unmapped page before the list ends, an unmapped
 * pointer, a low address and NULL are EFAULT. ERANGE comes before the
 * buffer is looked at, size 0 never uses it, and an empty list never
 * touches it (0 even for NULL with SIZE_MAX). */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_flistxattr 196
#define D "/tmp/a_flistxattr_buffer"

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_empty_file", a_make_file(D "/e", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.buf", "x", 1, 0), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    i64 ed = a_open(D "/e", O_RDONLY, 0);
    check("setup_open", fd >= 0 && ed >= 0, fd);
    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("setup_mmap", (i64)m > 0, (i64)m);
    if ((i64)m <= 0) return finish();
    expect("setup_unmap_second", syscall2(SYS_munmap, m + A_PAGE, A_PAGE), 0);
    char *edge = m + A_PAGE - 9;
    expect("edge_exact", syscall3(SYS_flistxattr, fd, edge, 100), 9);
    check("edge_bytes", mem_eq(edge, "user.buf\0", 9), 0);
    expect("edge_short", syscall3(SYS_flistxattr, fd, m + A_PAGE - 8, 100), -EFAULT);
    expect("unmapped", syscall3(SYS_flistxattr, fd, m + A_PAGE, 100), -EFAULT);
    expect("low_pointer", syscall3(SYS_flistxattr, fd, (void *)8, 100), -EFAULT);
    expect("null_buffer", syscall3(SYS_flistxattr, fd, 0, 100), -EFAULT);
    expect("null_size_max", syscall3(SYS_flistxattr, fd, 0, ~0ULL), -EFAULT);
    expect("null_exact", syscall3(SYS_flistxattr, fd, 0, 9), -EFAULT);
    expect("erange_null", syscall3(SYS_flistxattr, fd, 0, 8), -ERANGE);
    expect("erange_unmapped", syscall3(SYS_flistxattr, fd, m + A_PAGE, 3), -ERANGE);
    expect("size0_unmapped", syscall3(SYS_flistxattr, fd, m + A_PAGE, 0), 9);
    expect("empty_unmapped", syscall3(SYS_flistxattr, ed, m + A_PAGE, 100), 0);
    expect("empty_null", syscall3(SYS_flistxattr, ed, 0, 100), 0);
    expect("empty_null_size_max", syscall3(SYS_flistxattr, ed, 0, ~0ULL), 0);
    expect("cleanup_munmap", syscall2(SYS_munmap, m, A_PAGE), 0);
    a_close(ed);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_e", a_unlink(D "/e"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
