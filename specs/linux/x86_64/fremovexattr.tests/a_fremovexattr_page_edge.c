/* The name is read up to its NUL from user memory, at most 256 bytes: a
 * name whose NUL is the last byte before an unmapped page works; a name
 * that runs into the unmapped page before a NUL within its first 255
 * bytes is EFAULT (also with a bad descriptor: the name comes first);
 * 256 bytes without a NUL directly before the unmapped page are ERANGE. */
#include "a_sys.h"

#define SYS_mmap 9
#define SYS_munmap 11
#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define SYS_fremovexattr 199
#define ENODATA 61
#define D "/tmp/a_fremovexattr_page_edge"

static char buf[16];

static i64 frm(i64 fd, const char *n) { return syscall2(SYS_fremovexattr, fd, n); }

int test_main(void) {
    char *m = (char *)syscall6(SYS_mmap, 0, 8192, 3, 0x22, -1, 0);
    check("setup_mmap", (i64)m > 0, (i64)m);
    expect("setup_munmap", syscall2(SYS_munmap, m + 4096, 4096), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    expect("set_e", syscall5(SYS_fsetxattr, fd, "user.e", "v", 1, 0), 0);
    char *n = m + 4096 - 7;
    str_cpy(n, "user.e");
    expect("nul_at_page_end", frm(fd, n), 0);
    expect("removed", syscall4(SYS_fgetxattr, fd, "user.e", buf, sizeof buf), -ENODATA);
    n = m + 4096 - 6;
    for (int i = 0; i < 6; i++) n[i] = "user.e"[i];
    expect("runs_into_unmapped", frm(fd, n), -EFAULT);
    expect("runs_into_unmapped_badfd", frm(-1, n), -EFAULT);
    n = m + 4096 - 255;
    for (int i = 0; i < 255; i++) n[i] = 'a';
    expect("255_then_unmapped", frm(fd, n), -EFAULT);
    n = m + 4096 - 256;
    for (int i = 0; i < 256; i++) n[i] = 'a';
    expect("256_then_unmapped", frm(fd, n), -ERANGE);
    expect("256_then_unmapped_badfd", frm(-1, n), -ERANGE);
    a_close(fd);
    a_unlink(D "/f");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
