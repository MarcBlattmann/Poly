/* Like pipes, sockets and eventfds live on file systems that are mounted
 * nowhere; their st_dev is still a device ustat knows, with the counts
 * fstatfs reports. */
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

#define SYS_socket 41
#define SYS_eventfd2 290
#define AF_UNIX 1
#define SOCK_STREAM 1

static void one(const char *name, i64 fd) {
    struct a_stat st;
    struct a_statfs f;
    struct u_ustat u;
    check(name, fd >= 0, fd);
    expect("fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("fstatfs", syscall2(SYS_fstatfs, fd, &f), 0);
    memset(&u, 0x66, sizeof u);
    expect("ustat", syscall2(SYS_ustat, st.st_dev, &u), 0);
    expect("tfree_is_bfree", u.f_tfree, (i32)f.f_bfree);
    expect("tinode_is_ffree", (i64)u.f_tinode, (i64)f.f_ffree);
    expect("padding_zero", u.pad, 0);
}

int test_main(void) {
    one("socket", syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0));
    one("eventfd", syscall2(SYS_eventfd2, 0, 0));
    return finish();
}
