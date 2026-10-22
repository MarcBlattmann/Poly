/* getresgid across processes: a child inherits ids its parent set; ids a
 * child changes are its own, the parent's stay as they were.
 * poly-diff: needs=root (only root may set arbitrary group ids)
 */
#include "a_sys.h"

#define SYS_setresgid 119 /* asm/unistd_64.h */
#define SYS_getresgid 120 /* asm/unistd_64.h */

int test_main(void) {
    a_block_sigchld();
    expect("parent_set", syscall3(SYS_setresgid, 11, 12, 13), 0);
    volatile i32 *sh = a_shared();
    for (int i = 0; i < 8; i++) sh[i] = -1;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        u32 c[3];
        syscall3(SYS_getresgid, &c[0], &c[1], &c[2]);
        sh[0] = (i32)c[0];
        sh[1] = (i32)c[1];
        sh[2] = (i32)c[2];
        sh[3] = (i32)syscall3(SYS_setresgid, 21, 22, 23);
        syscall3(SYS_getresgid, &c[0], &c[1], &c[2]);
        sh[4] = (i32)c[0];
        sh[5] = (i32)c[1];
        sh[6] = (i32)c[2];
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    check("child_inherits", sh[0] == 11 && sh[1] == 12 && sh[2] == 13, sh[0]);
    expect("child_set", sh[3], 0);
    check("child_sees_own", sh[4] == 21 && sh[5] == 22 && sh[6] == 23, sh[4]);
    u32 r, e, s;
    syscall3(SYS_getresgid, &r, &e, &s);
    check("parent_unchanged", r == 11 && e == 12 && s == 13, (i64)r);
    syscall3(SYS_setresgid, 0, 0, 0);
    return finish();
}
