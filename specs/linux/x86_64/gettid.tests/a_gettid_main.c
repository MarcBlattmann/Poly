/* gettid: in a single-threaded process the thread id is the pid; it is
 * the same on every call and survives an execve-free life unchanged. */
#include "a_sys.h"

int test_main(void) {
    i64 tid = syscall0(SYS_gettid);
    check("positive", tid > 0, tid);
    expect("main_tid_is_pid", tid, syscall0(SYS_getpid));
    int same = 1;
    for (int i = 0; i < 8; i++) same &= syscall0(SYS_gettid) == tid;
    check("stable", same, 0);
    expect("set_tid_address_agrees", syscall1(SYS_set_tid_address, 0), tid);
    return finish();
}
