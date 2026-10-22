/* getdents with a buffer whose end runs into an inaccessible page: the
 * records that fit in the writable part are stored and their byte count
 * returned (not EFAULT); the position is after the last of them, so the
 * next call goes on with the record that did not fit. If not even the
 * first record can be written, EFAULT. All records here are 24 bytes. */
#include "a_sys.h"

#define SYS_getdents 78

struct g_dirent {
    u64 d_ino;
    u64 d_off;
    u16 d_reclen;
    char d_name[];
};

#define D "/tmp/a_getdents_partial_fault"

static char buf[4096] __attribute__((aligned(8)));
static const char *names[] = {".", "..", "a", "b", "c", "d"};
#define N 6

static int seen[N];

static void take(const char *p, i64 n) {
    for (i64 off = 0; off < n;) {
        const struct g_dirent *d = (const struct g_dirent *)(p + off);
        for (int i = 0; i < N; i++) seen[i] += str_eq(d->d_name, names[i]);
        if (d->d_reclen == 0) break;
        off += d->d_reclen;
    }
}

int test_main(void) {
    char p[96];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    for (int i = 2; i < N; i++) expect("setup_file", a_make_file(str_cat2(p, D "/", names[i]), "", 0644), 0);

    i64 m = syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("setup_mmap", m > 0, m);
    expect("setup_guard_page", syscall3(SYS_mprotect, m + 4096, 4096, PROT_NONE), 0);
    char *page_end = (char *)(m + 4096);

    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    /* 24 writable bytes: one record. */
    i64 n = syscall3(SYS_getdents, fd, page_end - 24, 4096);
    expect("writable_24_one_record", n, 24);
    take(page_end - 24, n > 0 ? n : 0);
    /* 40 writable bytes: still one record. */
    n = syscall3(SYS_getdents, fd, page_end - 40, 4096);
    expect("writable_40_one_record", n, 24);
    take(page_end - 40, n > 0 ? n : 0);
    /* 50 writable bytes: two records. */
    n = syscall3(SYS_getdents, fd, page_end - 50, 4096);
    expect("writable_50_two_records", n, 48);
    take(page_end - 50, n > 0 ? n : 0);
    /* 10 writable bytes: not even one record. */
    expect("writable_10_efault", syscall3(SYS_getdents, fd, page_end - 10, 4096), -EFAULT);
    /* The rest with a good buffer: the last two. */
    n = syscall3(SYS_getdents, fd, buf, sizeof buf);
    expect("rest_two_records", n, 48);
    take(buf, n > 0 ? n : 0);
    expect("end", syscall3(SYS_getdents, fd, buf, sizeof buf), 0);
    int once = 1;
    for (int i = 0; i < N; i++) once &= seen[i] == 1;
    check("each_name_once_over_all_calls", once, 0);
    a_close(fd);

    for (int i = 2; i < N; i++) expect("cleanup", a_unlink(str_cat2(p, D "/", names[i])), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
