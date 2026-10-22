/* ustat on the tmpfs at /dev/shm reports the same free block and free
 * inode counts as statfs and fstatfs on it (measured until quiet). */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_ustat 136

/* linux/types.h struct ustat (x86-64): __kernel_daddr_t (int), 4 bytes of
 * padding, __kernel_ino_t (unsigned long), char[6], char[6], 4 bytes of
 * padding: 32 bytes. `tail` lies past the structure. */
struct u_ustat {
    i32 f_tfree;
    u32 pad;
    u64 f_tinode;
    char f_fname[6];
    char f_fpack[6];
    u32 pad2;
    u32 tail;
};

static A_UNUSED u64 dev_of(const char *path) {
    struct a_stat st;
    if (a_stat_path(path, &st) != 0) return 0;
    return st.st_dev;
}

int test_main(void) {
    u64 dev = dev_of("/dev/shm");
    struct a_statfs a, b;
    struct u_ustat u;
    int ok = 0;
    for (int i = 0; i < 20 && !ok; i++) {
        syscall2(SYS_statfs, "/dev/shm", &a);
        expect("ustat_shm", syscall2(SYS_ustat, dev, &u), 0);
        syscall2(SYS_statfs, "/dev/shm", &b);
        if (a.f_bfree == b.f_bfree && a.f_ffree == b.f_ffree) ok = 1;
    }
    check("quiet_measurement", ok, 0);
    expect("tfree_is_bfree", u.f_tfree, (i32)b.f_bfree);
    expect("tinode_is_ffree", (i64)u.f_tinode, (i64)b.f_ffree);
    i64 fd = a_open("/dev/shm", O_RDONLY | O_DIRECTORY, 0);
    struct a_statfs f;
    expect("fstatfs_shm", syscall2(SYS_fstatfs, fd, &f), 0);
    expect("same_type_as_tmpfs", f.f_type, 0x01021994);
    a_close(fd);
    return finish();
}
