/* setdomainname errors, in a new UTS namespace where the caller may set
 * the name: len above 64 or negative (as a 32-bit int) is EINVAL, also
 * with a NULL name (length is checked before the bytes are read); a name
 * that cannot be read in full (NULL, a low address, bytes running into
 * an unmapped page) is EFAULT. A failed call leaves the name as it was.
 * A name that ends exactly at the end of a mapping is fine.
 * poly-diff: needs=root (a new UTS namespace needs CAP_SYS_ADMIN)
 */
#include "a_sys.h"

#define SYS_setdomainname 171
#define SYS_unshare 272
#define CLONE_NEWUTS 0x04000000 /* linux/sched.h */

static struct a_utsname u;
static char big[128];

static int name_is(const char *want) {
    memset(&u, 0, sizeof u);
    syscall1(SYS_uname, &u);
    return str_eq(u.domainname, want);
}

static int child(void) {
    expect("unshare_newuts", syscall1(SYS_unshare, CLONE_NEWUTS), 0);
    expect("set_start", syscall2(SYS_setdomainname, "start", 5), 0);

    for (int i = 0; i < 128; i++) big[i] = 'n';
    expect("len_65_einval", syscall2(SYS_setdomainname, big, 65), -EINVAL);
    expect("len_100_einval", syscall2(SYS_setdomainname, big, 100), -EINVAL);
    expect("len_minus1_einval", syscall2(SYS_setdomainname, big, -1), -EINVAL);
    expect("len_0xffffffff_einval", syscall2(SYS_setdomainname, big, 0xffffffffL), -EINVAL);
    expect("null_len_65_einval", syscall2(SYS_setdomainname, 0, 65), -EINVAL);
    check("unchanged_after_einval", name_is("start"), 0);

    expect("null_len_3_efault", syscall2(SYS_setdomainname, 0, 3), -EFAULT);
    expect("low_address_efault", syscall2(SYS_setdomainname, 1, 3), -EFAULT);
    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, m + A_PAGE, A_PAGE);
    m[A_PAGE - 2] = 'e';
    m[A_PAGE - 1] = 'z';
    expect("crosses_unmapped_efault", syscall2(SYS_setdomainname, m + A_PAGE - 2, 4), -EFAULT);
    check("unchanged_after_efault", name_is("start"), 0);
    expect("ends_at_mapping_end", syscall2(SYS_setdomainname, m + A_PAGE - 2, 2), 0);
    check("name_ez", name_is("ez"), 0);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, child());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    return finish();
}
