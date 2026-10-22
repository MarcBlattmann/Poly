/* getresgid across execve of a program without the set-group-ID bit: the
 * real and effective ids are kept and the saved id becomes the effective
 * id.
 * poly-diff: needs=root (only root may set arbitrary group ids)
 */
#include "a_sys.h"

#define SYS_setresgid 119 /* asm/unistd_64.h */
#define SYS_getresgid 120 /* asm/unistd_64.h */

int test_main(void) {
    u32 r = 0, e = 0, s = 0;
    if (a_argc() > 2 && str_eq(a_argv()[1], "again")) {
        passed = (int)a_atou(a_argv()[2]);
        expect("after_execve_ok", syscall3(SYS_getresgid, &r, &e, &s), 0);
        expect("real_kept", r, 10);
        expect("effective_kept", e, 20);
        expect("saved_becomes_effective", s, 20);
        syscall3(SYS_setresgid, 0, 0, 0);
        return finish();
    }
    expect("set", syscall3(SYS_setresgid, 10, 20, 30), 0);
    syscall3(SYS_getresgid, &r, &e, &s);
    check("before_execve", r == 10 && e == 20 && s == 30, (i64)s);
    if (failed) return finish();
    char count[24];
    a_utoa(count, (u64)passed);
    char *argv[] = {"x", "again", count, 0};
    char *envp[] = {0};
    syscall3(SYS_execve, a_self_path(), argv, envp);
    expect("execve_returned", 1, 0);
    return finish();
}
