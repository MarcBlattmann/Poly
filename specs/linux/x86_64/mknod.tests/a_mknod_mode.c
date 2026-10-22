/* mknod: permission bits are mode & ~umask; set-user-ID, set-group-ID and
 * sticky bits are kept for regular files, FIFOs and sockets. In a
 * set-group-ID directory the new node takes the directory's group and does
 * not get the set-group-ID bit. */
#include "a_sys.h"

#define SYS_mknod 133
#define D "/tmp/a_mknod_mode"

static i64 mk(const char *p, i64 mode, i64 dev) { return syscall3(SYS_mknod, p, mode, dev); }

static i64 mode_of(const char *p) {
    struct a_stat st;
    return a_lstat_path(p, &st) == 0 ? (i64)st.st_mode : -1;
}

int test_main(void) {
    struct a_stat st;
    expect("umask_prev", syscall1(SYS_umask, 022), 022);
    expect("setup_mkdir", a_mkdir(D, 0777), 0);

    expect("reg_07777", mk(D "/r1", S_IFREG | 07777, 0), 0);
    expect("reg_07777_mode", mode_of(D "/r1"), S_IFREG | 07755);
    expect("fifo_07777", mk(D "/f1", S_IFIFO | 07777, 0), 0);
    expect("fifo_07777_mode", mode_of(D "/f1"), S_IFIFO | 07755);
    expect("sock_06777", mk(D "/s1", S_IFSOCK | 06777, 0), 0);
    expect("sock_06777_mode", mode_of(D "/s1"), S_IFSOCK | 06755);
    expect("reg_sticky", mk(D "/r2", 01644, 0), 0);
    expect("reg_sticky_mode", mode_of(D "/r2"), S_IFREG | 01644);
    expect("reg_0", mk(D "/r3", 0, 0), 0);
    expect("reg_0_mode", mode_of(D "/r3"), S_IFREG);

    syscall1(SYS_umask, 0);
    expect("umask0_fifo", mk(D "/f2", S_IFIFO | 0777, 0), 0);
    expect("umask0_fifo_mode", mode_of(D "/f2"), S_IFIFO | 0777);
    expect("umask0_sock", mk(D "/s2", S_IFSOCK | 0666, 0), 0);
    expect("umask0_sock_mode", mode_of(D "/s2"), S_IFSOCK | 0666);
    syscall1(SYS_umask, 0777);
    expect("umask777_reg", mk(D "/r4", S_IFREG | 06777, 0), 0);
    expect("umask777_reg_mode", mode_of(D "/r4"), S_IFREG | 06000);
    syscall1(SYS_umask, 07777);
    expect("umask_only_low_bits", mk(D "/r5", S_IFREG | 01777, 0), 0);
    expect("umask_only_low_bits_mode", mode_of(D "/r5"), S_IFREG | 01000);
    expect("umask_read_back", syscall1(SYS_umask, 027), 0777);
    expect("umask027_fifo", mk(D "/f3", S_IFIFO | 0777, 0), 0);
    expect("umask027_fifo_mode", mode_of(D "/f3"), S_IFIFO | 0750);
    syscall1(SYS_umask, 022);

    expect("setup_sgid_dir", a_mkdir(D "/sg", 0777), 0);
    expect("setup_sgid_chmod", syscall2(SYS_chmod, D "/sg", 02777), 0);
    struct a_stat dst;
    expect("sgid_dir_stat", a_stat_path(D "/sg", &dst), 0);
    expect("sgid_dir_mode", dst.st_mode, S_IFDIR | 02777);
    expect("sgid_fifo", mk(D "/sg/f", S_IFIFO | 0666, 0), 0);
    expect("sgid_fifo_stat", a_lstat_path(D "/sg/f", &st), 0);
    expect("sgid_fifo_gid", st.st_gid, dst.st_gid);
    expect("sgid_fifo_mode", st.st_mode, S_IFIFO | 0644);
    expect("sgid_reg", mk(D "/sg/r", 0640, 0), 0);
    expect("sgid_reg_gid", a_lstat_path(D "/sg/r", &st) == 0 ? (i64)st.st_gid : -1, dst.st_gid);
    expect("sgid_reg_mode", st.st_mode, S_IFREG | 0640);

    expect("cleanup_sg_f", a_unlink(D "/sg/f"), 0);
    expect("cleanup_sg_r", a_unlink(D "/sg/r"), 0);
    expect("cleanup_sg", a_rmdir(D "/sg"), 0);
    const char *names[] = {D "/r1", D "/f1", D "/s1", D "/r2", D "/r3", D "/f2", D "/s2", D "/r4", D "/r5", D "/f3"};
    int ok = 1;
    for (unsigned i = 0; i < sizeof names / sizeof names[0]; i++) ok &= a_unlink(names[i]) == 0;
    check("cleanup_nodes", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
