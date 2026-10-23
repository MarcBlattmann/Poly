/* llistxattr buffer checks on a file: only the list's bytes must be
 * writable. A buffer ending exactly at an unmapped page works with a
 * larger size; one running into it before the list ends, an unmapped
 * pointer and NULL are EFAULT. ERANGE comes before the buffer is looked
 * at (a short size with a NULL buffer is ERANGE), and an empty list never
 * touches the buffer. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_llistxattr 195
#define D "/tmp/a_llistxattr_buffer"
#define F D "/f"

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_empty_file", a_make_file(D "/e", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, F, "user.xyz", "x", 1, 0), 0);
    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("setup_mmap", (i64)m > 0, (i64)m);
    if ((i64)m <= 0) return finish();
    expect("setup_unmap_second", syscall2(SYS_munmap, m + A_PAGE, A_PAGE), 0);
    char *edge = m + A_PAGE - 9;
    expect("edge_exact", syscall3(SYS_llistxattr, F, edge, 100), 9);
    check("edge_bytes", mem_eq(edge, "user.xyz\0", 9), 0);
    expect("edge_short", syscall3(SYS_llistxattr, F, m + A_PAGE - 8, 100), -EFAULT);
    expect("unmapped", syscall3(SYS_llistxattr, F, m + A_PAGE, 100), -EFAULT);
    expect("low_pointer", syscall3(SYS_llistxattr, F, (void *)8, 100), -EFAULT);
    expect("null_buffer", syscall3(SYS_llistxattr, F, 0, 100), -EFAULT);
    expect("erange_before_efault", syscall3(SYS_llistxattr, F, 0, 3), -ERANGE);
    expect("erange_unmapped", syscall3(SYS_llistxattr, F, m + A_PAGE, 8), -ERANGE);
    expect("size0_unmapped", syscall3(SYS_llistxattr, F, m + A_PAGE, 0), 9);
    expect("empty_list_unmapped", syscall3(SYS_llistxattr, D "/e", m + A_PAGE, 100), 0);
    expect("empty_list_null", syscall3(SYS_llistxattr, D "/e", 0, 100), 0);
    expect("cleanup_munmap", syscall2(SYS_munmap, m, A_PAGE), 0);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_e", a_unlink(D "/e"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
