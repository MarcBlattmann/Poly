/* With both an unknown device and an unwritable buffer, ustat reports
 * EINVAL: the device is looked up before anything is written. */
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
    expect("bad_dev_null_buf", syscall2(SYS_ustat, 0, 0), -EINVAL);
    expect("bad_dev_kernel_buf", syscall2(SYS_ustat, 0xffffffffL, 0xffff800000000000UL), -EINVAL);
    struct a_stat st;
    a_stat_path("/dev/null", &st);
    expect("chardev_null_buf", syscall2(SYS_ustat, st.st_rdev, 0), -EINVAL);
    expect("good_dev_null_buf", syscall2(SYS_ustat, dev_of("/proc"), 0), -EFAULT);
    return finish();
}
