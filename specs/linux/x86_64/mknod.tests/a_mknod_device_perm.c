/* mknod: character and block devices other than character 0:0 need
 * privilege (CAP_MKNOD): any other user gets EPERM and nothing is made;
 * root makes them. Block 0:0 is no exception. */
#include "a_sys.h"

#define SYS_mknod 133
#define D "/tmp/a_mknod_device_perm"

static i64 mk(const char *p, i64 mode, i64 dev) { return syscall3(SYS_mknod, p, mode, dev); }

static int root;

/* Root expects success, everyone else EPERM with nothing created. */
static void privileged(const char *name, const char *made, const char *path, i64 mode, i64 dev) {
    i64 r = mk(path, mode, dev);
    check(name, root ? r == 0 : r == -EPERM, r);
    struct a_stat st;
    i64 s = a_lstat_path(path, &st);
    check(made, root ? s == 0 && (st.st_mode & S_IFMT) == (mode & S_IFMT) : s == -ENOENT, s);
    if (s == 0) a_unlink(path);
}

int test_main(void) {
    root = syscall0(SYS_geteuid) == 0;
    syscall1(SYS_umask, 022);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);

    privileged("chr_1_3", "chr_1_3_made", D "/c", S_IFCHR | 0666, 0x0103);
    privileged("chr_0_1", "chr_0_1_made", D "/c", S_IFCHR | 0666, 1);
    privileged("chr_1_0", "chr_1_0_made", D "/c", S_IFCHR | 0666, 0x100);
    privileged("chr_minor_only_high_bits", "chr_minor_only_high_bits_made", D "/c", S_IFCHR | 0666, 0x100000001L);
    privileged("blk_8_1", "blk_8_1_made", D "/b", S_IFBLK | 0660, 0x0801);
    privileged("blk_0_0", "blk_0_0_made", D "/b", S_IFBLK | 0660, 0);
    privileged("blk_high_dev_bits", "blk_high_dev_bits_made", D "/b", S_IFBLK | 0660, 0x100000000L);

    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
