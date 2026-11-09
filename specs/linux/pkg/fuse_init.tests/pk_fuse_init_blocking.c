/* Until the server answers FUSE_INIT, operations on the mount wait. The
 * mount call itself returns at once. A second process stats a file while
 * the server holds the INIT reply back: the stat does not return (the
 * server saw INIT, and nothing else, no LOOKUP is sent yet). When the
 * reply is written the stat completes with the tree's answer.
 * (linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define P "/tmp/pk_fuse_iblock"

int test_main(void) {
    expect("setup", fz_setup(P), 0);
    fz->init_hold = 1;
    expect("open", fz_open() >= 0, 1);
    expect("mount_does_not_wait", fz_mount(0), 0);
    expect("start", fz_start() > 0, 1);
    check("init_seen", fz_wait_count(FZ_INIT, 1, 3000) == 1, fz_count(FZ_INIT));

    volatile i32 *r = a_shared();
    r[0] = 0; /* done */
    r[1] = -9999; /* result */
    i64 c = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (c == 0) {
        struct a_stat st;
        r[1] = (i32)a_stat_path(P "/hello", &st);
        r[2] = (i32)st.st_size;
        r[0] = 1;
        syscall1(SYS_exit_group, 0);
    }
    fz_msleep(300);
    expect("stat_waits", r[0], 0);
    expect("no_lookup_before_reply", fz_count(FZ_LOOKUP), 0);

    fz->go = 1;
    i32 status = -1;
    syscall4(SYS_wait4, c, &status, 0, 0);
    expect("stat_client_exit", status, 0);
    expect("stat_result_after_reply", r[1], 0);
    expect("stat_size_after_reply", r[2], 16);
    struct a_stat st;
    expect("later_stat_immediate", a_stat_path(P "/dir/inner", &st), 0);
    expect("later_size", st.st_size, 6);
    fz_cleanup();
    return finish();
}
