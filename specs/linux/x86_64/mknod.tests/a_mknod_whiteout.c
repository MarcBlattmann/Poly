/* mknod: a character device with device number 0:0 may be made by any
 * user (other device numbers need privilege). Only the low 32 bits of the
 * dev argument count, so 0x100000000 is 0:0 as well. Opening such a node
 * fails with ENXIO. */
#include "a_sys.h"

#define SYS_mknod 133
#define D "/tmp/a_mknod_whiteout"

static i64 mk(const char *p, i64 mode, i64 dev) { return syscall3(SYS_mknod, p, mode, dev); }

int test_main(void) {
    struct a_stat st;
    syscall1(SYS_umask, 022);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);

    expect("chr_0_0", mk(D "/w", S_IFCHR | 0666, 0), 0);
    expect("stat", a_lstat_path(D "/w", &st), 0);
    expect("mode", st.st_mode, S_IFCHR | 0644);
    expect("rdev", (i64)st.st_rdev, 0);
    expect("size", st.st_size, 0);
    expect("nlink", st.st_nlink, 1);
    expect("uid", st.st_uid, syscall0(SYS_geteuid));
    expect("open_enxio", a_open(D "/w", O_RDONLY, 0), -ENXIO);
    expect("open_wronly_enxio", a_open(D "/w", O_WRONLY, 0), -ENXIO);
    i64 pfd = a_open(D "/w", O_PATH, 0);
    check("open_opath", pfd >= 0, pfd);
    a_close(pfd);
    expect("again_eexist", mk(D "/w", S_IFCHR | 0666, 0), -EEXIST);

    expect("chr_high_dev_bits", mk(D "/h1", S_IFCHR | 0600, 0x100000000L), 0);
    expect("chr_high_dev_bits_rdev", a_lstat_path(D "/h1", &st) == 0 ? (i64)st.st_rdev : -1, 0);
    expect("chr_high_dev_bits_mode", st.st_mode, S_IFCHR | 0600);
    expect("chr_all_high_dev_bits", mk(D "/h2", S_IFCHR | 0600, (i64)0xffffffff00000000UL), 0);
    expect("chr_all_high_dev_bits_rdev", a_lstat_path(D "/h2", &st) == 0 ? (i64)st.st_rdev : -1, 0);

    syscall1(SYS_umask, 077);
    expect("chr_umask_077", mk(D "/u", S_IFCHR | 0777, 0), 0);
    expect("chr_umask_077_mode", a_lstat_path(D "/u", &st) == 0 ? st.st_mode : 0, S_IFCHR | 0700);
    syscall1(SYS_umask, 022);

    expect("cleanup_w", a_unlink(D "/w"), 0);
    expect("cleanup_h1", a_unlink(D "/h1"), 0);
    expect("cleanup_h2", a_unlink(D "/h2"), 0);
    expect("cleanup_u", a_unlink(D "/u"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
