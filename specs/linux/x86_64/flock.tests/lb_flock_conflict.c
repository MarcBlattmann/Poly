/* flock(2) locks belong to the open file description: two open() calls
 * of the same file conflict like two processes (EAGAIN with LOCK_NB);
 * dup'ed descriptors and fork children share the lock (any of them can
 * unlock it); the lock goes when the last descriptor of the description is
 * closed. Shared locks coexist; exclusive excludes everything; hard links
 * are the same file. */
#include "a_sys.h"

#define D "/tmp/lb_flock_conflict"
#define SYS_flock 73
#define LOCK_SH 1
#define LOCK_EX 2
#define LOCK_NB 4
#define LOCK_UN 8

static i64 flock_(i64 fd, i64 op) { return syscall2(SYS_flock, fd, op); }

static i64 a, b;

static int child_unlocks(void) {
    /* the child shares the parent's open file description */
    expect("child_relock_same_description", flock_(a, LOCK_EX | LOCK_NB), 0);
    expect("child_other_description_blocked", flock_(b, LOCK_SH | LOCK_NB), -EAGAIN);
    expect("child_unlock", flock_(a, LOCK_UN), 0);
    return finish();
}

int test_main(void) {
    a_block_sigchld();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_h", syscall2(SYS_link, D "/f", D "/h"), 0);
    a = a_open(D "/f", O_RDWR, 0);
    b = a_open(D "/f", O_RDONLY, 0);
    i64 c = a_open(D "/h", O_RDONLY, 0);
    check("opened", a >= 0 && b >= 0 && c >= 0, c);

    expect("a_ex", flock_(a, LOCK_EX), 0);
    expect("b_ex_nb", flock_(b, LOCK_EX | LOCK_NB), -EAGAIN);
    expect("b_sh_nb", flock_(b, LOCK_SH | LOCK_NB), -EAGAIN);
    expect("hardlink_sh_nb", flock_(c, LOCK_SH | LOCK_NB), -EAGAIN);
    expect("a_to_sh", flock_(a, LOCK_SH), 0);
    expect("b_sh_now", flock_(b, LOCK_SH | LOCK_NB), 0);
    expect("c_sh_too", flock_(c, LOCK_SH | LOCK_NB), 0);
    expect("a_to_ex_blocked_by_sharers", flock_(a, LOCK_EX | LOCK_NB), -EAGAIN);
    /* a failed conversion dropped a's shared lock: b can be the only one */
    expect("c_un", flock_(c, LOCK_UN), 0);
    expect("b_to_ex", flock_(b, LOCK_EX | LOCK_NB), 0);
    expect("b_un", flock_(b, LOCK_UN), 0);
    expect("a_ex_free", flock_(a, LOCK_EX | LOCK_NB), 0);

    /* dup shares the lock; closing one duplicate keeps it */
    i64 a2 = syscall1(SYS_dup, a);
    expect("dup_relock", flock_(a2, LOCK_EX | LOCK_NB), 0);
    expect("close_dup", a_close(a2), 0);
    expect("still_locked", flock_(b, LOCK_SH | LOCK_NB), -EAGAIN);
    a2 = syscall1(SYS_dup, a);
    expect("unlock_through_dup", flock_(a2, LOCK_UN), 0);
    expect("free_after_dup_unlock", flock_(b, LOCK_EX | LOCK_NB), 0);
    expect("b_un2", flock_(b, LOCK_UN), 0);

    /* a fork child shares it too */
    expect("a_ex_again", flock_(a, LOCK_EX), 0);
    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(child_unlocks());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);
    expect("free_after_child_unlock", flock_(b, LOCK_EX | LOCK_NB), 0);
    expect("b_un3", flock_(b, LOCK_UN), 0);

    /* closing every descriptor of the description releases the lock */
    expect("a_ex_last", flock_(a, LOCK_EX), 0);
    expect("close_a2", a_close(a2), 0);
    expect("held_by_a", flock_(b, LOCK_EX | LOCK_NB), -EAGAIN);
    expect("close_a", a_close(a), 0);
    expect("released_on_close", flock_(b, LOCK_EX | LOCK_NB), 0);
    /* unlinking does not release: the lock is on the file */
    expect("unlink_both", a_unlink(D "/f") == 0 && a_unlink(D "/h") == 0, 1);
    expect("c_blocked_after_unlink", flock_(c, LOCK_SH | LOCK_NB), -EAGAIN);

    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
