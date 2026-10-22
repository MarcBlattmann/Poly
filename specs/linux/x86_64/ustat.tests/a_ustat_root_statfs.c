/* ustat on the file system holding /tmp: f_tfree is statfs f_bfree (free
 * blocks, not f_bavail) and f_tinode is statfs f_ffree. Other processes
 * may write to that file system, so a measurement is repeated until a
 * statfs before and after the ustat agree. */
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
    u64 dev = dev_of("/tmp");
    struct a_statfs a, b;
    struct u_ustat u;
    int ok = 0;
    i64 r = 0;
    for (int i = 0; i < 20 && !ok; i++) {
        syscall2(SYS_statfs, "/tmp", &a);
        r = syscall2(SYS_ustat, dev, &u);
        syscall2(SYS_statfs, "/tmp", &b);
        if (a.f_bfree == b.f_bfree && a.f_ffree == b.f_ffree) ok = 1;
    }
    expect("ustat_tmp", r, 0);
    check("quiet_measurement", ok, 0);
    expect("tfree_is_bfree", u.f_tfree, (i32)b.f_bfree);
    expect("tinode_is_ffree", (i64)u.f_tinode, (i64)b.f_ffree);
    return finish();
}
