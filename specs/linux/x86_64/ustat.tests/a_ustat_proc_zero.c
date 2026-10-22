/* ustat on the device of /proc: success, and every one of the 32 bytes of
 * struct ustat is written (all zero: proc reports no free blocks or
 * inodes); the bytes after the structure are left alone. */
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
    unsigned char buf[48];
    memset(buf, 0xAA, sizeof buf);
    u64 dev = dev_of("/proc");
    check("proc_dev_found", dev != 0, (i64)dev);
    expect("ustat_proc", syscall2(SYS_ustat, dev, buf), 0);
    int zero = 1, kept = 1;
    for (int i = 0; i < 32; i++)
        if (buf[i] != 0) zero = 0;
    for (int i = 32; i < 48; i++)
        if (buf[i] != 0xAA) kept = 0;
    check("all_32_bytes_zero", zero, buf[0]);
    check("bytes_after_untouched", kept, buf[32]);
    return finish();
}
