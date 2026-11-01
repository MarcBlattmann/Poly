/* sethostname(name, len) in a new UTS namespace: the node name becomes
 * exactly the len bytes given (no terminator needed), as uname shows it;
 * a shorter name leaves nothing of the old one; 64 bytes is the maximum;
 * len 0 (also with a NULL name) makes it empty; bytes after an embedded
 * NUL are stored but uname shows the name up to it; only the low 32 bits
 * of len count. The domain name is not touched, and the test's own
 * namespace keeps its node name.
 * poly-diff: needs=root (a new UTS namespace needs CAP_SYS_ADMIN)
 */
#include "a_sys.h"

#define SYS_sethostname 170
#define SYS_unshare 272
#define CLONE_NEWUTS 0x04000000 /* linux/sched.h */

static struct a_utsname before, u;
static char name64[64];

static void get(void) {
    memset(&u, 0x55, sizeof u);
    syscall1(SYS_uname, &u);
}

static int child(void) {
    expect("unshare_newuts", syscall1(SYS_unshare, CLONE_NEWUTS), 0);
    get();
    check("starts_with_old_name", str_eq(u.nodename, before.nodename), 0);

    expect("set_abc", syscall2(SYS_sethostname, "abcdef", 3), 0);
    get();
    check("uname_abc", str_eq(u.nodename, "abc"), 0);
    check("domain_untouched", str_eq(u.domainname, before.domainname), 0);

    expect("set_long", syscall2(SYS_sethostname, "poly-test-node", 14), 0);
    expect("set_short", syscall2(SYS_sethostname, "xy", 2), 0);
    get();
    check("short_replaces_whole_name", str_eq(u.nodename, "xy") && u.nodename[3] == 0 && u.nodename[13] == 0, 0);

    for (int i = 0; i < 64; i++) name64[i] = (char)('a' + i % 26);
    expect("set_64", syscall2(SYS_sethostname, name64, 64), 0);
    get();
    check("uname_64_bytes", str_len(u.nodename) == 64 && mem_eq(u.nodename, name64, 64), (i64)str_len(u.nodename));

    expect("set_len0", syscall2(SYS_sethostname, "zz", 0), 0);
    get();
    check("len0_empty", u.nodename[0] == 0, u.nodename[0]);

    expect("set_null_len0", syscall2(SYS_sethostname, 0, 0), 0);
    get();
    check("null_len0_empty", u.nodename[0] == 0, u.nodename[0]);

    expect("set_embedded_nul", syscall2(SYS_sethostname, "ab\0cd", 5), 0);
    get();
    check("uname_up_to_nul", str_eq(u.nodename, "ab"), 0);
    check("rest_stored", mem_eq(u.nodename, "ab\0cd\0", 6), 0);

    expect("len_high_bits_ignored", syscall2(SYS_sethostname, "hijklm", (1L << 32) | 3), 0);
    get();
    check("uname_hij", str_eq(u.nodename, "hij"), 0);

    /* A process forked now is in the same namespace. */
    i64 pid = a_fork_sync();
    if (pid == 0) {
        get();
        syscall1(SYS_exit_group, str_eq(u.nodename, "hij") ? 0 : 1);
    }
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("forked_child_sees_name", st, 0);
    /* Hand the counts to the parent, which prints the summary. */
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    syscall1(SYS_uname, &before);
    a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, child());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    get();
    check("own_namespace_unchanged", str_eq(u.nodename, before.nodename), 0);
    return finish();
}
