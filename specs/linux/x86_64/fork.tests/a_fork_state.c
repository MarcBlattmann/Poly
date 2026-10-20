/* fork: the child starts with the parent's working directory, umask,
 * resource limits and signal mask, and a copy of private mappings; its
 * changes stay its own. (Child reports through shared memory, then spins
 * until killed.) */
#include "a_sys.h"

#define D "/tmp/a_fork_state"

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    a_mkdir(D, 0700);
    syscall1(SYS_chdir, D);
    i64 old_mask = syscall1(SYS_umask, 0026);
    struct a_rlimit lim, old;
    syscall2(SYS_getrlimit, RLIMIT_NOFILE, &old);
    lim.rlim_cur = 77;
    lim.rlim_max = old.rlim_max;
    syscall2(SYS_setrlimit, RLIMIT_NOFILE, &lim);
    u64 block = A_SIGBIT(SIGUSR1);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &block, 0, 8);
    i64 priv = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    *(volatile char *)priv = 'p';
    i64 pid = syscall0(SYS_fork);
    if (pid == 0) {
        char cwd[64];
        sh[1] = syscall2(SYS_getcwd, cwd, sizeof cwd) > 0 && str_eq(cwd, D);
        sh[2] = (i32)syscall1(SYS_umask, 0);
        struct a_rlimit l;
        syscall2(SYS_getrlimit, RLIMIT_NOFILE, &l);
        sh[3] = (i32)l.rlim_cur;
        u64 mask = 0;
        syscall4(SYS_rt_sigprocmask, SIG_BLOCK, 0, &mask, 8);
        sh[4] = (mask & A_SIGBIT(SIGUSR1)) != 0;
        sh[5] = *(volatile char *)priv;
        *(volatile char *)priv = 'c';
        syscall1(SYS_chdir, "/");
        syscall1(SYS_umask, 0777);
        sh[0] = 1;
        for (;;) a_relax();
    }
    a_spin_until(&sh[0], 1, A_SPIN_LIMIT);
    syscall2(SYS_kill, pid, SIGKILL);
    i32 status;
    expect("wait4", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    check("fork", pid > 0, pid);
    expect("child_cwd_inherited", sh[1], 1);
    expect("child_umask_inherited", sh[2], 0026);
    expect("child_rlimit_inherited", sh[3], 77);
    expect("child_sigmask_inherited", sh[4], 1);
    expect("child_saw_private_data", sh[5], 'p');
    expect("parent_private_unchanged", *(volatile char *)priv, 'p');
    char cwd[64];
    syscall2(SYS_getcwd, cwd, sizeof cwd);
    check("parent_cwd_unchanged", str_eq(cwd, D), cwd[0]);
    syscall4(SYS_rt_sigprocmask, SIG_UNBLOCK, &block, 0, 8);
    syscall2(SYS_setrlimit, RLIMIT_NOFILE, &old);
    expect("parent_umask_unchanged", syscall1(SYS_umask, old_mask), 0026);
    syscall1(SYS_chdir, "/");
    expect("cleanup", a_rmdir(D), 0);
    return finish();
}
