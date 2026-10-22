/* getdents on tmpfs (/dev/shm): the same order and d_off values as
 * getdents64 there (see the getdents64 spec's directory_order). A
 * directory only added to lists ".", "..", then newest first. d_off of
 * "." is 1, of ".." the cookie of the first entry, of each other entry
 * the cookie of the one after it, 2147483647 for the last; cookies are
 * 3, 4, ... in creation order. After a call the position is the d_off of
 * the last record; lseek to a d_off reads the record after it. */
#include "a_sys.h"

#define SYS_getdents 78

/* linux/magic.h */
#define TMPFS_MAGIC 0x01021994

struct g_dirent {
    u64 d_ino;
    u64 d_off;
    u16 d_reclen;
    char d_name[];
};

#define D "/dev/shm/d_getdents_tmpfs_order"

static char buf[4096] __attribute__((aligned(8)));
static const char *created[] = {"mm", "cc", "xx", "aa", "zz", "bb", "yy", "dd"};
static const char *listed[] = {".", "..", "dd", "yy", "bb", "zz", "aa", "xx", "cc", "mm"};
static const i64 offs[] = {1, 10, 9, 8, 7, 6, 5, 4, 3, 2147483647};
#define N 10

int test_main(void) {
    char p[96], c[64];
    struct a_statfs sf;
    i64 r = syscall2(SYS_statfs, "/dev/shm", &sf);
    check("setup_dev_shm_is_tmpfs", r == 0 && sf.f_type == TMPFS_MAGIC, r == 0 ? sf.f_type : r);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    for (int i = 0; i < 8; i++) expect("setup_create", a_make_file(str_cat2(p, D "/", created[i]), "", 0644), 0);

    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i64 n = syscall3(SYS_getdents, fd, buf, sizeof buf);
    expect("one_call_bytes", n, N * 24);
    int k = 0;
    for (i64 o = 0; o < n && k < N; k++) {
        struct g_dirent *d = (struct g_dirent *)(buf + o);
        check(str_cat2(c, "order_", listed[k]), str_eq(d->d_name, listed[k]), k);
        expect(str_cat2(c, "d_off_", listed[k]), (i64)d->d_off, offs[k]);
        if (d->d_reclen == 0) break;
        o += d->d_reclen;
    }
    expect("position_at_end", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 2147483647);
    expect("end", syscall3(SYS_getdents, fd, buf, sizeof buf), 0);

    /* Three records per call: the position follows the last d_off. */
    expect("rewind", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    expect("three_records", syscall3(SYS_getdents, fd, buf, 72), 72);
    expect("position_after_three", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 9);
    n = syscall3(SYS_getdents, fd, buf, 72);
    expect("next_three", n, 72);
    check("next_starts_with_yy", str_eq(((struct g_dirent *)buf)->d_name, "yy"), 0);
    expect("position_after_six", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 6);

    /* lseek to each d_off: the next call starts with the record after it. */
    for (int i = 0; i < N - 1; i++) {
        expect("seek_to_d_off", syscall3(SYS_lseek, fd, offs[i], SEEK_SET), offs[i]);
        n = syscall3(SYS_getdents, fd, buf, 24);
        check(str_cat2(c, "after_seek_reads_", listed[i + 1]), n == 24 && str_eq(((struct g_dirent *)buf)->d_name, listed[i + 1]), n);
    }
    expect("seek_to_last_d_off", syscall3(SYS_lseek, fd, 2147483647, SEEK_SET), 2147483647);
    expect("after_last_d_off_end", syscall3(SYS_getdents, fd, buf, sizeof buf), 0);
    a_close(fd);

    for (int i = 0; i < 8; i++) expect("cleanup", a_unlink(str_cat2(p, D "/", created[i])), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
