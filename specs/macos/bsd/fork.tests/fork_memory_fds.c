/* fork(2), BSD 2: memory is copied (the child's writes do not show in the
 * parent), descriptors are shared (one file offset, the same pipe). */
#include "poly_mtest.h"

static int counter = 1;

int test_main(void) {
    sysret f = bsd3(SYS_open, "f.txt", O_CREAT | O_RDWR, 0644);
    bsd3(SYS_write, f.ret, "abcdef", 6);
    bsd3(SYS_lseek, f.ret, 0, SEEK_SET);
    sysret p = bsd0(SYS_pipe);
    sysret c = bsd0(SYS_fork);
    if (c.ret2 == 1) {
        char b[2];
        counter = 99;
        bsd3(SYS_read, f.ret, b, 2);
        bsd3(SYS_write, p.ret2, "child", 5);
        poly_exit(b[0] == 'a' && b[1] == 'b' ? 0 : 1);
    }
    int st = poly_wait(c.ret);
    check("the child read the first two bytes", W_EXITED(st) && W_EXITSTATUS(st) == 0, st);
    check("the parent's memory is unchanged", counter == 1, counter);
    char b[8] = {0};
    sysret r = bsd3(SYS_read, f.ret, b, 2);
    check("the offset is shared: the parent reads \"cd\"", r.ret == 2 && b[0] == 'c' && b[1] == 'd', r.ret);
    r = bsd3(SYS_read, p.ret, b, 8);
    check("the pipe carries the child's bytes", r.ret == 5 && mem_eq(b, "child", 5), r.ret);
    return finish();
}
