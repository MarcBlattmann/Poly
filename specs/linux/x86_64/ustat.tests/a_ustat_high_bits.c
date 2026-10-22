/* The device argument is 32 bits wide: bits 32-63 of the register are
 * ignored, so a valid device number with junk above it still works and
 * gives the same answer. */
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
    u64 dev = dev_of("/proc");
    struct u_ustat a, b, c;
    memset(&b, 0x11, sizeof b);
    memset(&c, 0x22, sizeof c);
    expect("plain", syscall2(SYS_ustat, dev, &a), 0);
    expect("bit_32_set", syscall2(SYS_ustat, dev | (1UL << 32), &b), 0);
    expect("high_half_ones", syscall2(SYS_ustat, dev | 0xffffffff00000000UL, &c), 0);
    check("same_answer_b", mem_eq(&a, &b, 32), b.f_tfree);
    check("same_answer_c", mem_eq(&a, &c, 32), c.f_tfree);
    expect("zero_low_half", syscall2(SYS_ustat, 0xffffffff00000000UL, &a), -EINVAL);
    return finish();
}
