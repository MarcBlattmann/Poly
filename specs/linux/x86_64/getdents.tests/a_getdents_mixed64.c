/* getdents and getdents64 on one descriptor share its position: a
 * listing can be read with both calls alternating, and every entry still
 * comes once. For the same entry both give the same inode number, d_off,
 * record length and type (getdents64 has d_type at byte 18 and the name
 * at 19, getdents the name at 18 and the type in the last byte). */
#include "a_sys.h"

#define SYS_getdents 78

struct g_dirent {
    u64 d_ino;
    u64 d_off;
    u16 d_reclen;
    char d_name[];
};

#define D "/tmp/a_getdents_mixed64"

static char buf[4096] __attribute__((aligned(8)));
static char buf64[4096] __attribute__((aligned(8)));
static const char *names[] = {".", "..", "a", "sub", "link", "name_of_twenty_chars"};
#define N 6

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_a", a_make_file(D "/a", "", 0644), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "a", AT_FDCWD, D "/link"), 0);
    expect("setup_long", a_make_file(D "/name_of_twenty_chars", "", 0644), 0);

    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i64 fd64 = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i64 n = syscall3(SYS_getdents, fd, buf, sizeof buf);
    i64 n64 = syscall3(SYS_getdents64, fd64, buf64, sizeof buf64);
    expect("same_byte_count", n, n64);
    /* Record by record, in the same order. */
    int same = 1, count = 0;
    for (i64 o = 0; o < n && o < n64 && count < 64; count++) {
        struct g_dirent *d = (struct g_dirent *)(buf + o);
        struct a_dirent64 *e = (struct a_dirent64 *)(buf64 + o);
        same &= d->d_ino == e->d_ino && (i64)d->d_off == e->d_off && d->d_reclen == e->d_reclen &&
                str_eq(d->d_name, e->d_name) && (u8)buf[o + d->d_reclen - 1] == e->d_type;
        if (d->d_reclen == 0 || d->d_reclen != e->d_reclen) break;
        o += d->d_reclen;
    }
    check("same_records", same, 0);
    expect("record_count", count, N);
    a_close(fd64);

    /* Alternate, one record per call, on one descriptor. */
    expect("rewind", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    int seen[N] = {0}, calls = 0;
    for (int guard = 0; guard < 20; guard++) {
        char name[64];
        i64 r;
        if (guard % 2 == 0) {
            r = syscall3(SYS_getdents, fd, buf, 47);
            if (r > 0) {
                struct g_dirent *d = (struct g_dirent *)buf;
                if (d->d_reclen != r) same = 0;
                str_cpy(name, d->d_name);
            }
        } else {
            r = syscall3(SYS_getdents64, fd, buf64, 40);
            if (r > 0) {
                struct a_dirent64 *e = (struct a_dirent64 *)buf64;
                if (e->d_reclen != r) same = 0;
                str_cpy(name, e->d_name);
            }
        }
        if (r <= 0) {
            expect("alternating_end", r, 0);
            break;
        }
        calls++;
        for (int i = 0; i < N; i++) seen[i] += str_eq(name, names[i]);
    }
    int once = 1;
    for (int i = 0; i < N; i++) once &= seen[i] == 1;
    check("alternating_each_once", once, 0);
    expect("alternating_calls", calls, N);
    check("alternating_one_record_per_call", same, 0);
    a_close(fd);

    expect("cleanup_long", a_unlink(D "/name_of_twenty_chars"), 0);
    expect("cleanup_link", a_unlink(D "/link"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_a", a_unlink(D "/a"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
