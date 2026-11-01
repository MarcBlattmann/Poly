/* setregid without CAP_SETGID: the real gid may only be set to the
 * current real or effective gid; the effective gid only to the current
 * real, effective or saved gid. Anything else is EPERM and nothing changes
 * (also when the other half of the call would have been allowed). The
 * saved gid follows the same rule as with privilege: it becomes the new
 * effective gid when rgid is given or egid is given and differs from the
 * old real gid. Each case runs in a fresh child with real/effective/saved
 * gids 100/200/300, no supplementary groups and all uids 1000.
 * poly-diff: needs=root (sets up ids of other users and groups)
 */
#include "a_sys.h"

#define SYS_setregid 114
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_getresgid 120
#define SYS_setfsgid 123

static volatile i32 *sh;

static void run_case(i64 rg, i64 eg) {
    i64 c = a_fork_sync();
    if (c == 0) {
        syscall2(SYS_setgroups, 0, 0);
        syscall3(SYS_setresgid, 100, 200, 300);
        sh[6] = (i32)syscall3(SYS_setresuid, 1000, 1000, 1000);
        sh[0] = (i32)syscall2(SYS_setregid, rg, eg);
        u32 r = 0, e = 0, s = 0;
        syscall3(SYS_getresgid, &r, &e, &s);
        sh[1] = (i32)r;
        sh[2] = (i32)e;
        sh[3] = (i32)s;
        sh[4] = (i32)syscall1(SYS_setfsgid, -1);
        exit_group(0);
    }
    i32 st;
    syscall4(SYS_wait4, c, &st, 0, 0);
}

static void ok_case(const char *name, i64 rg, i64 eg, u32 r, u32 e, u32 s) {
    run_case(rg, eg);
    check(name, sh[6] == 0 && sh[0] == 0 && (u32)sh[1] == r && (u32)sh[2] == e && (u32)sh[3] == s && (u32)sh[4] == e,
          sh[0]);
}

static void eperm_case(const char *name, i64 rg, i64 eg) {
    run_case(rg, eg);
    check(name, sh[6] == 0 && sh[0] == -EPERM && sh[1] == 100 && sh[2] == 200 && sh[3] == 300 && sh[4] == 200, sh[0]);
}

int test_main(void) {
    a_block_sigchld(); /* children's ends must not interrupt at varying points */
    sh = a_shared();
    ok_case("neither", -1, -1, 100, 200, 300);
    ok_case("rgid_to_rgid", 100, -1, 100, 200, 200);
    ok_case("rgid_to_egid", 200, -1, 200, 200, 200);
    eperm_case("rgid_to_sgid_eperm", 300, -1);
    eperm_case("rgid_to_other_eperm", 5, -1);
    ok_case("egid_to_rgid", -1, 100, 100, 100, 300);
    ok_case("egid_to_egid", -1, 200, 100, 200, 200);
    ok_case("egid_to_sgid", -1, 300, 100, 300, 300);
    eperm_case("egid_to_other_eperm", -1, 5);
    ok_case("swap", 200, 100, 200, 100, 100);
    ok_case("both_to_egid", 200, 200, 200, 200, 200);
    eperm_case("good_egid_bad_rgid_eperm", 300, 300);
    eperm_case("good_rgid_bad_egid_eperm", 100, 5);
    eperm_case("large_gid_eperm", 0xfffffffe, -1);
    return finish();
}
