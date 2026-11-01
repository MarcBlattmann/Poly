/* unshare in a process that shares things: with a second thread
 * (CLONE_VM|CLONE_FS|CLONE_FILES|CLONE_SIGHAND|CLONE_THREAD),
 * unshare(CLONE_VM), (CLONE_SIGHAND), (CLONE_THREAD) and (CLONE_NEWUSER)
 * are EINVAL, while CLONE_FILES, CLONE_FS and CLONE_SYSVSEM work. With
 * only the memory shared with another process (clone(CLONE_VM)),
 * unshare(CLONE_VM) is EINVAL but CLONE_SIGHAND and CLONE_THREAD succeed
 * (nothing of theirs is shared). No privilege is involved.
 */
#include "a_sys.h"

#define SYS_unshare 272
#define CLONE_NEWUSER 0x10000000 /* linux/sched.h */

static volatile i32 stop, started, done;

static int spinner(void *arg) {
    (void)arg;
    started = 1;
    while (!stop) a_relax();
    return 0;
}

static int threaded(void) {
    done = -1;
    a_clone(CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_CHILD_CLEARTID,
            a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&done, 0, spinner, 0);
    a_spin_until(&started, 1, A_SPIN_LIMIT);
    expect("thr_vm_einval", syscall1(SYS_unshare, CLONE_VM), -EINVAL);
    expect("thr_sighand_einval", syscall1(SYS_unshare, CLONE_SIGHAND), -EINVAL);
    expect("thr_thread_einval", syscall1(SYS_unshare, CLONE_THREAD), -EINVAL);
    expect("thr_newuser_einval", syscall1(SYS_unshare, CLONE_NEWUSER), -EINVAL);
    expect("thr_files_ok", syscall1(SYS_unshare, CLONE_FILES), 0);
    expect("thr_fs_ok", syscall1(SYS_unshare, CLONE_FS), 0);
    expect("thr_sysvsem_ok", syscall1(SYS_unshare, CLONE_SYSVSEM), 0);
    stop = 1;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

static int vm_shared(void) {
    a_block_sigchld();
    started = 0;
    stop = 0;
    i64 pid = a_spawn(1, &done, spinner, 0);
    a_spin_until(&started, 1, A_SPIN_LIMIT);
    expect("vmproc_vm_einval", syscall1(SYS_unshare, CLONE_VM), -EINVAL);
    expect("vmproc_sighand_ok", syscall1(SYS_unshare, CLONE_SIGHAND), 0);
    expect("vmproc_thread_ok", syscall1(SYS_unshare, CLONE_THREAD), 0);
    stop = 1;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("vmproc_exit_0", st, 0);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

static void run(int (*fn)(void)) {
    a_shared()[0] = a_shared()[1] = 0;
    passed = failed = 0;
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, fn());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    i32 p = a_shared()[0], f = a_shared()[1];
    passed = p;
    failed = f;
    return;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    run(threaded);
    i32 p = passed, f = failed;
    run(vm_shared);
    passed += p;
    failed += f;
    return finish();
}
