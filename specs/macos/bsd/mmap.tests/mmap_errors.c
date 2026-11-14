/* mmap(2), BSD 197: EBADF without a file, EINVAL for an unaligned
 * MAP_FIXED address, a negative length or a directory, ENOMEM for a length
 * that cannot fit, EACCES for protections the descriptor does not allow,
 * ENODEV for a device that cannot be mapped. */
#include "poly_mtest.h"

static i64 page_size(void) {
    i64 ps = 0;
    size_t len = sizeof ps;
    bsd6(SYS_sysctlbyname, "hw.pagesize", 11, &ps, &len, 0, 0);
    return ps;
}

int test_main(void) {
    i64 ps = page_size();
    check_err("no MAP_ANON and fd -1: EBADF", bsd6(SYS_mmap, 0, ps, PROT_READ, MAP_PRIVATE, -1, 0), EBADF);
    check_err("fd 99: EBADF", bsd6(SYS_mmap, 0, ps, PROT_READ, MAP_PRIVATE, 99, 0), EBADF);
    sysret m = bsd6(SYS_mmap, 0, ps, PROT_READ, MAP_ANON | MAP_PRIVATE, -1, 0);
    check_err("MAP_FIXED at an unaligned address: EINVAL",
              bsd6(SYS_mmap, m.ret + 1, ps, PROT_READ, MAP_ANON | MAP_PRIVATE | MAP_FIXED, -1, 0), EINVAL);
    check_err("length 2^62: ENOMEM", bsd6(SYS_mmap, 0, (i64)1 << 62, PROT_READ, MAP_ANON | MAP_PRIVATE, -1, 0), ENOMEM);
    check_err("length -4096: EINVAL", bsd6(SYS_mmap, 0, -4096, PROT_READ, MAP_ANON | MAP_PRIVATE, -1, 0), EINVAL);
    bsd1(SYS_close, bsd3(SYS_open, "f.bin", O_CREAT | O_WRONLY, 0644).ret);
    sysret wo = bsd3(SYS_open, "f.bin", O_WRONLY, 0);
    check_err("PROT_READ of a write-only file: EACCES", bsd6(SYS_mmap, 0, ps, PROT_READ, MAP_PRIVATE, wo.ret, 0), EACCES);
    sysret ro = bsd3(SYS_open, "f.bin", O_RDONLY, 0);
    check_err("MAP_SHARED writable of a read-only file: EACCES",
              bsd6(SYS_mmap, 0, ps, PROT_READ | PROT_WRITE, MAP_SHARED, ro.ret, 0), EACCES);
    sysret pw = bsd6(SYS_mmap, 0, ps, PROT_READ | PROT_WRITE, MAP_PRIVATE, ro.ret, 0);
    check("MAP_PRIVATE writable of a read-only file works", pw.err == 0, pw.err);
    sysret d = bsd3(SYS_open, ".", O_RDONLY, 0);
    check_err("a directory: EINVAL", bsd6(SYS_mmap, 0, ps, PROT_READ, MAP_PRIVATE, d.ret, 0), EINVAL);
    sysret n = bsd3(SYS_open, "/dev/null", O_RDONLY, 0);
    check_err("/dev/null: ENODEV", bsd6(SYS_mmap, 0, ps, PROT_READ, MAP_PRIVATE, n.ret, 0), ENODEV);
    return finish();
}
