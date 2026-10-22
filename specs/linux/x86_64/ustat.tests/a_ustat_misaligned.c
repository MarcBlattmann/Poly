/* The buffer needs no alignment: a buffer at an odd address gets the same
 * 32 bytes as an aligned one, and the bytes around it are left alone. */
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

static unsigned char raw[80];

int test_main(void) {
    u64 dev = dev_of("/dev/shm");
    struct u_ustat a;
    memset(raw, 0x99, sizeof raw);
    i64 r1 = 0, r2 = 0;
    int same = 0;
    for (int i = 0; i < 20 && !same; i++) {
        r1 = syscall2(SYS_ustat, dev, raw + 3);
        r2 = syscall2(SYS_ustat, dev, &a);
        same = mem_eq(raw + 3, &a, 32);
    }
    expect("misaligned", r1, 0);
    expect("aligned", r2, 0);
    check("same_bytes", same, raw[3]);
    check("before_untouched", raw[0] == 0x99 && raw[1] == 0x99 && raw[2] == 0x99, raw[2]);
    check("after_untouched", raw[35] == 0x99 && raw[36] == 0x99, raw[35]);
    return finish();
}
