/* setfsgid without CAP_SETGID: the filesystem gid may be set to the
 * current real, effective, saved or filesystem gid; any other value is
 * ignored without an error, and the call still returns the previous value
 * (so a caller cannot tell from the result whether it worked; a second
 * call shows it). Runs in a child with real/effective/saved gids
 * 100/200/300, no supplementary groups and all uids 1000.
 * poly-diff: needs=root (sets up ids of other users and groups)
 */
#include "a_sys.h"

#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_setfsgid 123

static i64 fs(i64 v) { return syscall1(SYS_setfsgid, v); }

static int body(void) {
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, 100, 200, 300);
    check("drop_uids", syscall3(SYS_setresuid, 1000, 1000, 1000) == 0, 0);
    check("starts_at_egid", fs(-1) == 200, fs(-1));
    i64 v = fs(5);
    check("other_returns_old", v == 200, v);
    check("other_ignored", fs(-1) == 200, fs(-1));
    v = fs(100);
    check("rgid_returns_old", v == 200, v);
    check("rgid_taken", fs(-1) == 100, fs(-1));
    v = fs(300);
    check("sgid_returns_old", v == 100, v);
    check("sgid_taken", fs(-1) == 300, fs(-1));
    v = fs(300);
    check("same_value_ok", v == 300 && fs(-1) == 300, v);
    v = fs(0);
    check("root_group_returns_old", v == 300, v);
    check("root_group_ignored", fs(-1) == 300, fs(-1));
    v = fs(0xfffffffe);
    check("large_ignored", v == 300 && fs(-1) == 300, v);
    v = fs(200);
    check("egid_taken", v == 300 && fs(-1) == 200, v);
    return 0;
}

int test_main(void) {
    a_block_sigchld(); /* children's ends must not interrupt at varying points */
    volatile i32 *counts = a_shared();
    i64 c = a_fork_sync();
    if (c == 0) {
        body();
        counts[0] = passed;
        counts[1] = failed;
        exit_group(0);
    }
    i32 st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    passed += counts[0];
    failed += counts[1];
    check("child_exited", A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);
    return finish();
}
