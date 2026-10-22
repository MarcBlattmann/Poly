/* mlockall: MCL_FUTURE does not survive execve; the new program's
 * mappings are not populated. */
#include "a_sys.h"

static i64 resident(i64 addr, i64 pages) {
    unsigned char v[16];
    i64 r = syscall3(SYS_mincore, addr, pages * A_PAGE, v);
    if (r) return r;
    i64 n = 0;
    for (i64 i = 0; i < pages; i++) n += v[i] & 1;
    return n;
}

static i64 anon(i64 pages) {
    return syscall6(SYS_mmap, 0, pages * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}

int test_main(void) {
    if (a_argc() > 2 && str_eq(a_argv()[1], "again")) {
        passed = (int)a_atou(a_argv()[2]);
        expect("future_cleared_by_execve", resident(anon(4), 4), 0);
        return finish();
    }
    expect("mlockall", syscall1(SYS_mlockall, MCL_CURRENT | MCL_FUTURE), 0);
    expect("future_on_before_execve", resident(anon(4), 4), 4);
    if (failed) return finish();
    char count[24];
    a_utoa(count, (u64)passed);
    char *argv[] = {"x", "again", count, 0};
    char *envp[] = {0};
    syscall3(SYS_execve, a_self_path(), argv, envp);
    expect("execve_returned", 1, 0);
    return finish();
}
