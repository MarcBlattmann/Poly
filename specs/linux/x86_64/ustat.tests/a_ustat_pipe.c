/* ustat accepts the device of any file system the kernel has, also ones
 * that are not mounted anywhere: the st_dev of a pipe works, and its
 * counts are those fstatfs reports for the pipe. */
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
    int p[2];
    expect("pipe", syscall2(SYS_pipe2, p, 0), 0);
    struct a_stat st;
    expect("fstat_pipe", syscall2(SYS_fstat, p[0], &st), 0);
    struct a_statfs f;
    expect("fstatfs_pipe", syscall2(SYS_fstatfs, p[0], &f), 0);
    struct u_ustat u;
    memset(&u, 0x33, sizeof u);
    expect("ustat_pipe_dev", syscall2(SYS_ustat, st.st_dev, &u), 0);
    expect("tfree_is_bfree", u.f_tfree, (i32)f.f_bfree);
    expect("tinode_is_ffree", (i64)u.f_tinode, (i64)f.f_ffree);
    expect("padding_zero", u.pad, 0);
    a_close(p[0]);
    a_close(p[1]);
    return finish();
}
