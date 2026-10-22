/* personality: the persona belongs to each thread. A new thread starts
 * with its creator's persona; a change made by one thread is not seen by
 * the other.
 * poly-diff: syscalls=off (two threads)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_personality 135
#define QUERY 0xffffffffUL

static i64 personality(u64 p) { return syscall1(SYS_personality, p); }

static volatile i64 seen_start, seen_set, seen_after;
static volatile i32 step, tid_word;

static int thread_fn(void *arg) {
    (void)arg;
    seen_start = personality(QUERY);
    seen_set = personality(0x0006);
    step = 1;
    while (step != 2) a_relax();
    seen_after = personality(QUERY);
    step = 3;
    return 0;
}

int test_main(void) {
    i64 initial = personality(QUERY);
    expect("set_main", personality(0x0008), initial);
    tid_word = 1;
    long t = a_clone(CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM |
                         CLONE_CHILD_CLEARTID,
                     a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&tid_word, 0, thread_fn, 0);
    check("clone_ok", t > 0, t);
    if (t <= 0) return finish();
    while (step != 1) a_relax();
    expect("thread_inherited", seen_start, 0x0008);
    expect("thread_set_returned_inherited", seen_set, 0x0008);
    expect("main_not_changed_by_thread", personality(QUERY), 0x0008);
    expect("main_set", personality(0x0040000), 0x0008);
    step = 2;
    while (step != 3) a_relax();
    expect("thread_not_changed_by_main", seen_after, 0x0006);
    while (tid_word != 0) a_relax();
    expect("main_has_own", personality(QUERY), 0x0040000);
    return finish();
}
