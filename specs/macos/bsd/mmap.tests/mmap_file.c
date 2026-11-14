/* mmap(2), BSD 197, files: the mapping shows the file; an offset that is
 * not page aligned is accepted and the address keeps the offset within the
 * page; a MAP_SHARED store reaches the file, a MAP_PRIVATE one does not;
 * mapping past the end of the file succeeds. */
#include "poly_mtest.h"

static i64 page_size(void) {
    i64 ps = 0;
    size_t len = sizeof ps;
    bsd6(SYS_sysctlbyname, "hw.pagesize", 11, &ps, &len, 0, 0);
    return ps;
}

int test_main(void) {
    i64 ps = page_size();
    sysret f = bsd3(SYS_open, "m.bin", O_CREAT | O_RDWR, 0644);
    bsd3(SYS_write, f.ret, "0123456789", 10);
    sysret m = bsd6(SYS_mmap, 0, ps, PROT_READ, MAP_PRIVATE, f.ret, 0);
    check_ok("map the file", m);
    const char *p = (const char *)m.ret;
    check("the mapping shows the file", mem_eq(p, "0123456789", 10), p[0]);
    check("and zeros after its end", p[10] == 0 && p[ps - 1] == 0, p[10]);
    sysret o = bsd6(SYS_mmap, 0, 4, PROT_READ, MAP_PRIVATE, f.ret, 3);
    check_ok("offset 3 is accepted", o);
    check("the address is 3 bytes into a page", o.ret % ps == 3, o.ret % ps);
    check("and shows the file from offset 3", *(const char *)o.ret == '3', *(const char *)o.ret);
    sysret s = bsd6(SYS_mmap, 0, ps, PROT_READ | PROT_WRITE, MAP_SHARED, f.ret, 0);
    ((char *)s.ret)[0] = 'X';
    sysret q = bsd6(SYS_mmap, 0, ps, PROT_READ | PROT_WRITE, MAP_PRIVATE, f.ret, 0);
    ((char *)q.ret)[1] = 'Y';
    char buf[10];
    bsd3(SYS_lseek, f.ret, 0, SEEK_SET);
    bsd3(SYS_read, f.ret, buf, 10);
    check("a MAP_SHARED store is in the file", buf[0] == 'X', buf[0]);
    check("a MAP_PRIVATE store is not", buf[1] == '1', buf[1]);
    sysret b = bsd6(SYS_mmap, 0, ps, PROT_READ, MAP_PRIVATE, f.ret, ps * 4);
    check("mapping past the end of the file succeeds", b.err == 0 && b.ret % ps == 0, b.err);
    return finish();
}
