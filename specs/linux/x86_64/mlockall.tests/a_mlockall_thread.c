/* mlockall: the setting belongs to the address space, so a mapping made by
 * another task sharing it (CLONE_VM) is populated too. */
#include "a_sys.h"

static i64 resident(i64 addr, i64 pages) {
    unsigned char v[16];
    i64 r = syscall3(SYS_mincore, addr, pages * A_PAGE, v);
    if (r) return r;
    i64 n = 0;
    for (i64 i = 0; i < pages; i++) n += v[i] & 1;
    return n;
}

static volatile i64 mapped;

static int child(void *arg) {
    (void)arg;
    mapped = syscall6(SYS_mmap, 0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    expect("mlockall", syscall1(SYS_mlockall, MCL_FUTURE), 0);
    volatile i32 done;
    i64 pid = a_spawn(0, &done, child, 0);
    check("spawn", pid > 0, pid);
    check("child_ended", a_spin_until(&done, 0, A_SPIN_LIMIT), done);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    check("child_mapped", mapped > 0, mapped);
    expect("child_mapping_populated", resident(mapped, 4), 4);
    return finish();
}
