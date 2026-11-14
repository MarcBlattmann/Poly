/* write(2), BSD 4: the byte count comes back; a count of 0 writes nothing
 * and returns 0; writes to a file append at the offset, and with O_APPEND
 * at the end. */
#include "poly_mtest.h"

int test_main(void) {
    sysret r = bsd3(SYS_write, 1, "line written by write\n", 22);
    check("write to stdout returns the count", r.err == 0 && r.ret == 22, r.err ? r.err : r.ret);
    r = bsd3(SYS_write, 1, "x", 0);
    check("a count of 0 returns 0", r.err == 0 && r.ret == 0, r.err ? r.err : r.ret);
    sysret f = bsd3(SYS_open, "w.txt", O_CREAT | O_RDWR, 0644);
    check_ok("open a new file", f);
    r = bsd3(SYS_write, f.ret, "hello", 5);
    check("first write returns 5", r.err == 0 && r.ret == 5, r.ret);
    r = bsd3(SYS_write, f.ret, "abc", 3);
    check("second write returns 3", r.err == 0 && r.ret == 3, r.ret);
    r = bsd3(SYS_lseek, f.ret, 0, SEEK_END);
    check("the file is 8 bytes", r.err == 0 && r.ret == 8, r.ret);
    bsd1(SYS_close, f.ret);
    sysret a = bsd3(SYS_open, "w.txt", O_WRONLY | O_APPEND, 0);
    bsd3(SYS_lseek, a.ret, 0, SEEK_SET);
    r = bsd3(SYS_write, a.ret, "Z", 1);
    check("an O_APPEND write returns 1", r.err == 0 && r.ret == 1, r.ret);
    bsd1(SYS_close, a.ret);
    char buf[16] = {0};
    sysret rd = bsd3(SYS_open, "w.txt", O_RDONLY, 0);
    r = bsd3(SYS_read, rd.ret, buf, sizeof buf);
    check("O_APPEND wrote at the end, not at offset 0", r.ret == 9 && mem_eq(buf, "helloabcZ", 9), r.ret);
    return finish();
}
