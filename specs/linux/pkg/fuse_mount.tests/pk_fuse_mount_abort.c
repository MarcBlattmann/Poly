/* The end of a connection. When the server's descriptor is closed (here:
 * the server process is killed) every operation on the mount fails with
 * ENOTCONN (107), also stat of the mount's root and open of files; the
 * mount stays until it is unmounted, which then succeeds. Unmounting a
 * live mount while a file on it is open is EBUSY, as is a working
 * directory on it; after the last user is gone umount2 works and the
 * server's next read of /dev/fuse fails with ENODEV (19), which ends it.
 * MNT_DETACH with an open file succeeds at once, the mount point is an
 * ordinary empty directory again, and the open file still reads.
 * (linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define P "/tmp/pk_fuse_abort"

static void fresh(void) {
    expect("open", fz_open() >= 0, 1);
    expect("mount", fz_mount(0), 0);
    expect("start", fz_start() > 0, 1);
}

int test_main(void) {
    expect("setup", fz_setup(P), 0);

    /* 1. server dies */
    fresh();
    struct a_stat st;
    expect("stat_ok_while_alive", a_stat_path(P "/hello", &st), 0);
    fz_stop();
    expect("stat_after_death", a_stat_path(P "/hello", &st), -ENOTCONN);
    expect("stat_root_after_death", a_stat_path(P, &st), -ENOTCONN);
    expect("open_after_death", a_open(P "/hello", O_RDONLY, 0), -ENOTCONN);
    expect("unmount_after_death", syscall2(SYS_umount2, P, 0), 0);
    expect("mount_point_plain_dir", a_stat_path(P, &st), 0);

    /* 2. EBUSY, then a clean unmount ends the server with ENODEV */
    fresh();
    i64 fd = a_open(P "/hello", O_RDONLY, 0);
    check("open_file", fd >= 0, fd);
    expect("umount_busy_open_file", syscall2(SYS_umount2, P, 0), -EBUSY);
    char b[32];
    expect("read_still_works", a_read(fd, b, sizeof b), 16);
    a_close(fd);
    i64 dfd = a_open(P "/dir", O_RDONLY | O_DIRECTORY, 0);
    check("open_dir", dfd >= 0, dfd);
    expect("umount_busy_open_dir", syscall2(SYS_umount2, P, 0), -EBUSY);
    a_close(dfd);
    expect("umount_idle", syscall2(SYS_umount2, P, 0), 0);
    i32 status = fz_wait();
    expect("server_exited_normally", status, 0);
    expect("server_read_enodev", fz->exit_read_err, -ENODEV);
    expect("mount_point_empty_dir", a_stat_path(P "/hello", &st), -ENOENT);

    /* 3. working directory inside */
    fresh();
    expect("chdir_in", syscall1(SYS_chdir, P), 0);
    expect("umount_busy_cwd", syscall2(SYS_umount2, P, 0), -EBUSY);
    expect("chdir_out", syscall1(SYS_chdir, "/"), 0);
    expect("umount_after_chdir", syscall2(SYS_umount2, P, 0), 0);
    fz_wait();

    /* 4. lazy unmount with an open file */
    fresh();
    fd = a_open(P "/hello", O_RDONLY, 0);
    check("open_file_again", fd >= 0, fd);
    expect("umount_detach", syscall2(SYS_umount2, P, MNT_DETACH), 0);
    expect("tree_gone_from_path", a_stat_path(P "/hello", &st), -ENOENT);
    expect("open_file_reads", a_read(fd, b, sizeof b), 16);
    check("open_file_content", mem_eq(b, "hello from fuse\n", 16), 0);
    a_close(fd);
    fz_wait();
    fz_cleanup();
    return finish();
}
