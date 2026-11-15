/* proc_info, BSD 336, PROC_PIDLISTFDS (flavor 1): an array of struct
 * proc_fdinfo {int32 fd, uint32 type} (8 bytes), one per open descriptor in
 * ascending order, and the call returns the bytes used: type 1 for a file
 * (PROX_FDTYPE_VNODE), 6 for a pipe end (PIPE), 5 for a kqueue (KQUEUE); a
 * closed descriptor disappears from the list. */
#include "poly_mtest.h"

static u8 buf[1024];

static i64 type_of(i64 n, i32 fd) {
    for (i64 i = 0; i + 8 <= n; i += 8)
        if (*(i32 *)(buf + i) == fd) return *(u32 *)(buf + i + 4);
    return -1;
}

int test_main(void) {
    i64 me = bsd0(SYS_getpid).ret;
    sysret f = bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644);
    sysret p = bsd0(SYS_pipe);
    sysret k = bsd0(SYS_kqueue);
    sysret r = bsd6(SYS_proc_info, PROC_INFO_CALL_PIDINFO, me, PROC_PIDLISTFDS, 0, buf, sizeof buf);
    check("the call returns a multiple of 8", r.err == 0 && r.ret > 0 && r.ret % 8 == 0, r.err ? r.err : r.ret);
    i64 n = r.ret;
    check("the file (fd 3) is a vnode", type_of(n, f.ret) == PROX_FDTYPE_VNODE, type_of(n, f.ret));
    check("the pipe's read end (fd 4) is a pipe", type_of(n, p.ret) == PROX_FDTYPE_PIPE, type_of(n, p.ret));
    check("and its write end (fd 5)", type_of(n, p.ret2) == PROX_FDTYPE_PIPE, type_of(n, p.ret2));
    check("the kqueue (fd 6) is a kqueue", type_of(n, k.ret) == PROX_FDTYPE_KQUEUE, type_of(n, k.ret));
    check("an fd that is not open is not listed", type_of(n, 40) == -1, type_of(n, 40));
    i64 first = *(i32 *)buf;
    check("the list is in ascending order, from fd 0", first == 0 && *(i32 *)(buf + 8) == 1, first);
    bsd1(SYS_close, f.ret);
    r = bsd6(SYS_proc_info, PROC_INFO_CALL_PIDINFO, me, PROC_PIDLISTFDS, 0, buf, sizeof buf);
    check("a closed descriptor disappears from the list", type_of(r.ret, f.ret) == -1, type_of(r.ret, f.ret));
    return finish();
}
