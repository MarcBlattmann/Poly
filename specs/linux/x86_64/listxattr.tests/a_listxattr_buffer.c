/* listxattr: only the list's own bytes must be writable. A buffer ending
 * exactly at an unmapped page works even with a larger size; one that runs
 * into the unmapped page before the list ends, an unmapped pointer and NULL
 * with a size that fits are EFAULT. Without attributes the buffer is never
 * touched. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_listxattr 194
#define EFAULT 14
#define D "/tmp/a_listxattr_buffer"
#define F D "/f"

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_empty_file", a_make_file(D "/e", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, F, "user.abc", "x", 1, 0), 0);
    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("setup_mmap", (i64)m > 0, (i64)m);
    if ((i64)m <= 0) return finish();
    expect("setup_unmap_second", syscall2(SYS_munmap, m + A_PAGE, A_PAGE), 0);
    char *edge = m + A_PAGE - 9;
    expect("edge_exact", syscall3(SYS_listxattr, F, edge, 100), 9);
    check("edge_bytes", mem_eq(edge, "user.abc\0", 9), 0);
    expect("edge_exact_size9", syscall3(SYS_listxattr, F, edge, 9), 9);
    expect("edge_short", syscall3(SYS_listxattr, F, m + A_PAGE - 8, 100), -EFAULT);
    expect("unmapped", syscall3(SYS_listxattr, F, m + A_PAGE, 100), -EFAULT);
    expect("low_pointer", syscall3(SYS_listxattr, F, (void *)8, 100), -EFAULT);
    expect("null_buffer", syscall3(SYS_listxattr, F, 0, 100), -EFAULT);
    expect("null_buffer_size_max", syscall3(SYS_listxattr, F, 0, -1L), -EFAULT);
    expect("empty_list_unmapped", syscall3(SYS_listxattr, D "/e", m + A_PAGE, 100), 0);
    expect("cleanup_munmap", syscall2(SYS_munmap, m, A_PAGE), 0);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_e", a_unlink(D "/e"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
