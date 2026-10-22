/* ustat of a device number that no mounted file system has: EINVAL, and
 * the buffer is not written. Device 0, the character device of /dev/null
 * (its st_rdev), and the all-ones number are such numbers. */
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
    struct u_ustat u;
    memset(&u, 0x77, sizeof u);
    expect("dev_zero", syscall2(SYS_ustat, 0, &u), -EINVAL);
    struct a_stat st;
    expect("stat_null", a_stat_path("/dev/null", &st), 0);
    expect("dev_null_rdev", syscall2(SYS_ustat, st.st_rdev, &u), -EINVAL);
    expect("dev_all_ones_32", syscall2(SYS_ustat, 0xffffffffL, &u), -EINVAL);
    expect("dev_minus_one", syscall2(SYS_ustat, -1L, &u), -EINVAL);
    expect("buffer_untouched", u.pad, 0x77777777);
    expect("buffer_untouched_tfree", u.f_tfree, 0x77777777);
    return finish();
}
