/* getdents (the old x86-64 call, number 78): records are struct
 * linux_dirent: d_ino (u64), d_off (u64), d_reclen (u16), d_name at byte
 * 18, NUL-terminated, and d_type in the record's last byte. d_reclen is
 * 18 + strlen(name) + 2 rounded up to a multiple of 8. Every entry,
 * "." and ".." included, comes exactly once, with the inode number lstat
 * reports for it; then 0. The order is not specified (names as a set). */
#include "a_sys.h"

#define SYS_getdents 78

/* struct linux_dirent, getdents(2) */
struct g_dirent {
    u64 d_ino;
    u64 d_off;
    u16 d_reclen;
    char d_name[];
};

#define D "/tmp/a_getdents_layout"

static char buf[4096] __attribute__((aligned(8)));

static const char *names[] = {".", "..", "f", "dir", "abcde", "abcdefghijkl", "abcdefghijklm", "a_rather_long_file_name_0123456789"};
#define N 8

int test_main(void) {
    char p[128];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    for (int i = 2; i < N; i++) {
        str_cat2(p, D "/", names[i]);
        if (i == 3)
            expect("setup_subdir", a_mkdir(p, 0755), 0);
        else
            expect("setup_file", a_make_file(p, "x", 0644), 0);
    }

    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("open", fd >= 0, fd);
    i64 n = syscall3(SYS_getdents, fd, buf, sizeof buf);
    i64 want = 0;
    for (int i = 0; i < N; i++) want += (18 + str_len(names[i]) + 2 + 7) & ~7L;
    expect("all_in_one_call_bytes", n, want);

    int seen[N] = {0}, count = 0, reclen_ok = 1, ino_ok = 1, nul_ok = 1, aligned_ok = 1;
    for (i64 off = 0; off < n && count < 64;) {
        struct g_dirent *d = (struct g_dirent *)(buf + off);
        size_t len = str_len(d->d_name);
        count++;
        reclen_ok &= d->d_reclen == ((18 + len + 2 + 7) & ~7L);
        aligned_ok &= d->d_reclen % 8 == 0;
        nul_ok &= 18 + len < (size_t)d->d_reclen - 1;
        for (int i = 0; i < N; i++)
            if (str_eq(d->d_name, names[i])) {
                struct a_stat st;
                seen[i]++;
                if (i == 0)
                    str_cpy(p, D);
                else if (i == 1)
                    str_cpy(p, "/tmp");
                else
                    str_cat2(p, D "/", names[i]);
                ino_ok &= a_lstat_path(p, &st) == 0 && st.st_ino == d->d_ino && d->d_ino != 0;
            }
        if (d->d_reclen == 0) break;
        off += d->d_reclen;
    }
    expect("entry_count", count, N);
    int each_once = 1;
    for (int i = 0; i < N; i++) each_once &= seen[i] == 1;
    check("each_name_once", each_once, 0);
    check("record_lengths", reclen_ok, 0);
    check("record_lengths_multiple_of_8", aligned_ok, 0);
    check("name_nul_before_type_byte", nul_ok, 0);
    check("inode_numbers_match_lstat", ino_ok, 0);
    expect("end_of_directory_0", syscall3(SYS_getdents, fd, buf, sizeof buf), 0);
    expect("still_0", syscall3(SYS_getdents, fd, buf, sizeof buf), 0);
    a_close(fd);

    for (int i = N - 1; i >= 2; i--) {
        str_cat2(p, D "/", names[i]);
        expect("cleanup", i == 3 ? a_rmdir(p) : a_unlink(p), 0);
    }
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
