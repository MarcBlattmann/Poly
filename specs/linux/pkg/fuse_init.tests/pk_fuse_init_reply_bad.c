/* Replies to FUSE_INIT that make the connection unusable. A major other
 * than 7 (8, 6, 0) or an error reply (ENOSYS, EPERM, EIO) is accepted by
 * the write, but from then on the kernel refuses every operation on the
 * mount with ECONNREFUSED (111): stat of files, open, statfs, even of the
 * mount's root, and creating things; no request but INIT reaches the
 * server. The mount can still be unmounted normally, and the server then
 * reads ENODEV.
 * (linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define P "/tmp/pk_fuse_irbad"

static void scenario(const char *tag, i32 major, i32 error) {
    char name[64];
    char *p = str_cpy(name, tag);
    fz->init_major = major;
    fz->init_error = error;
    int before = fz->n;
    fz_open();
    expect(name, fz_mount(0), 0);
    fz_start();
    struct a_stat st;
    str_cpy(p, "_stat_file");
    expect(name, a_stat_path(P "/hello", &st), -ECONNREFUSED);
    str_cpy(p, "_stat_root");
    expect(name, a_stat_path(P, &st), -ECONNREFUSED);
    str_cpy(p, "_open");
    expect(name, a_open(P "/hello", O_RDONLY, 0), -ECONNREFUSED);
    str_cpy(p, "_statfs");
    struct a_statfs sf;
    expect(name, syscall2(SYS_statfs, P, &sf), -ECONNREFUSED);
    str_cpy(p, "_mkdir");
    expect(name, a_mkdir(P "/d", 0755), -ECONNREFUSED);
    str_cpy(p, "_only_init");
    expect(name, fz->n - before, 1);
    str_cpy(p, "_umount");
    expect(name, syscall2(SYS_umount2, P, 0), 0);
    fz_wait();
    str_cpy(p, "_enodev");
    expect(name, fz->exit_read_err, -ENODEV);
}

int test_main(void) {
    expect("setup", fz_setup(P), 0);
    scenario("major8", 8, 0);
    scenario("major6", 6, 0);
    scenario("err_enosys", 0, ENOSYS);
    scenario("err_eperm", 0, EPERM);
    scenario("err_eio", 0, EIO);
    fz_cleanup();
    return finish();
}
