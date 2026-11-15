/* read(2), BSD 3: reads what is there, up to the count, then 0 at the end
 * of the file; a count of 0 returns 0. */
#include "poly_mtest.h"

int test_main(void) {
    sysret f = bsd3(SYS_open, "r.txt", O_CREAT | O_WRONLY, 0644);
    bsd3(SYS_write, f.ret, "abcde", 5);
    bsd1(SYS_close, f.ret);
    sysret r = bsd3(SYS_open, "r.txt", O_RDONLY, 0);
    char buf[16] = {0};
    sysret n = bsd3(SYS_read, r.ret, buf, 2);
    check("a count of 2 reads 2", n.err == 0 && n.ret == 2 && mem_eq(buf, "ab", 2), n.ret);
    n = bsd3(SYS_read, r.ret, buf, sizeof buf);
    check("the rest: 3 bytes", n.err == 0 && n.ret == 3 && mem_eq(buf, "cde", 3), n.ret);
    n = bsd3(SYS_read, r.ret, buf, sizeof buf);
    check("at the end: 0", n.err == 0 && n.ret == 0, n.err ? n.err : n.ret);
    bsd3(SYS_lseek, r.ret, 0, SEEK_SET);
    n = bsd3(SYS_read, r.ret, buf, 0);
    check("a count of 0: 0", n.err == 0 && n.ret == 0, n.ret);
    n = bsd3(SYS_read, r.ret, buf, sizeof buf);
    check("and the offset did not move", n.ret == 5, n.ret);
    return finish();
}
