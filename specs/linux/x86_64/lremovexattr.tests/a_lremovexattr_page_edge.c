/* lremovexattr reads the name up to its NUL or 256 bytes: a name whose NUL
 * is the last byte before an unmapped page works; one that runs into the
 * unmapped page without a NUL within its first 255 bytes is EFAULT; 256 or
 * more readable bytes without a NUL are ERANGE even with the unmapped page
 * after them. The same holds when the path is a symbolic link. */
#include "a_sys.h"

#define SYS_mmap 9
#define SYS_munmap 11
#define SYS_setxattr 188
#define SYS_lremovexattr 198
#define ENODATA 61
#define D "/tmp/a_lremovexattr_page_edge"
#define F D "/f"
#define L D "/l"

static i64 lrm(const char *p, const char *name) { return syscall2(SYS_lremovexattr, p, name); }

static char *prefix(char *at) {
    at[0] = 'u'; at[1] = 's'; at[2] = 'e'; at[3] = 'r'; at[4] = '.';
    return at;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, L), 0);
    char *pg = (char *)syscall6(SYS_mmap, 0, 8192, 3, 0x22, -1, 0);
    check("mmap", (i64)pg > 0, (i64)pg);
    if ((i64)pg <= 0) return finish();
    expect("munmap_second", syscall2(SYS_munmap, pg + 4096, 4096), 0);
    char *end = pg + 4096;
    str_cpy(end - 7, "user.e");
    expect("set_e", syscall5(SYS_setxattr, F, "user.e", "v", 1, 0), 0);
    expect("terminated_at_edge_ok", lrm(F, end - 7), 0);
    expect("terminated_at_edge_again", lrm(F, end - 7), -ENODATA);
    expect("terminated_at_edge_link_eperm", lrm(L, end - 7), -EPERM);
    char *u = end - 6;
    prefix(u)[5] = 'e';
    expect("unterminated_short_efault", lrm(F, u), -EFAULT);
    expect("unterminated_short_link_efault", lrm(L, u), -EFAULT);
    for (int i = 0; i < 4096; i++) pg[i] = 'x';
    expect("unterminated_255_efault", lrm(F, prefix(end - 255)), -EFAULT);
    expect("unterminated_256_erange", lrm(F, prefix(end - 256)), -ERANGE);
    expect("unterminated_256_link_erange", lrm(L, prefix(end - 256)), -ERANGE);
    expect("unterminated_300_erange", lrm(F, prefix(end - 300)), -ERANGE);
    syscall2(SYS_munmap, pg, 4096);
    a_unlink(L);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
