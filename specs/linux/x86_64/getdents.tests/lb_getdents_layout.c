/* getdents(2) (the old call, 78) fills buf with struct linux_dirent
 * records: d_ino (8 bytes), d_off (8), d_reclen (2), the NUL-terminated
 * name, padding, and the file type as the record's last byte; d_reclen is
 * 18 + name length + 2 rounded up to a multiple of 8. Every entry, "." and
 * ".." included, comes exactly once; at the end it returns 0. */
#include "a_sys.h"

#define D "/tmp/lb_getdents_layout"
#define SYS_getdents 78
#define SYS_mknodat 259

static unsigned char buf[4096];

static const char *names[] = {".", "..", "file", "subdir", "link", "fifo", "a_rather_long_name_of_thirty_two_b"};
static const u8 types[] = {DT_DIR, DT_DIR, DT_REG, DT_DIR, DT_LNK, DT_FIFO, DT_REG};
#define N 7

int test_main(void) {
    struct a_stat st;
    int seen[N] = {0};
    int bad_reclen = 0, bad_ino = 0, unknown = 0, bad_type = 0, bad_nul = 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/file", "x", 0644), 0);
    expect("setup_subdir", a_mkdir(D "/subdir", 0755), 0);
    expect("setup_link", syscall2(SYS_symlink, "file", D "/link"), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0644, 0), 0);
    expect("setup_long", a_make_file(D "/a_rather_long_name_of_thirty_two_b", "", 0644), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("open", fd >= 0, fd);

    i64 n = syscall3(SYS_getdents, fd, buf, sizeof buf);
    check("returned_bytes", n > 0, n);
    i64 total = 0, records = 0;
    for (i64 pos = 0; pos < n;) {
        u64 ino = *(u64 *)(buf + pos);
        u16 reclen = *(u16 *)(buf + pos + 16);
        const char *name = (const char *)(buf + pos + 18);
        size_t len = str_len(name);
        u8 type = buf[pos + reclen - 1];
        if (reclen != ((18 + len + 2 + 7) & ~7UL)) bad_reclen++;
        if (18 + len >= reclen) bad_nul++;
        int k = -1;
        for (int i = 0; i < N; i++)
            if (str_eq(name, names[i])) k = i;
        if (k < 0) {
            unknown++;
        } else {
            seen[k]++;
            if (type != types[k]) bad_type++;
            char path[128];
            str_cat2(path, D "/", name);
            if (a_lstat_path(path, &st) == 0 && st.st_ino != ino) bad_ino++;
        }
        records++;
        total += reclen;
        pos += reclen;
    }
    expect("records", records, N);
    expect("reclens_sum_to_return", total, n);
    expect("reclen_formula", bad_reclen, 0);
    expect("names_terminated", bad_nul, 0);
    expect("no_unknown_names", unknown, 0);
    expect("types", bad_type, 0);
    expect("inodes_match_lstat", bad_ino, 0);
    for (int i = 0; i < N; i++) expect(names[i], seen[i], 1);
    expect("end_returns_0", syscall3(SYS_getdents, fd, buf, sizeof buf), 0);
    expect("end_again_0", syscall3(SYS_getdents, fd, buf, sizeof buf), 0);
    /* rewinding starts over */
    expect("lseek_0", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    expect("again_same_bytes", syscall3(SYS_getdents, fd, buf, sizeof buf), n);
    expect("close", a_close(fd), 0);

    /* an empty directory has "." and ".." */
    fd = a_open(D "/subdir", O_RDONLY, 0);
    n = syscall3(SYS_getdents, fd, buf, sizeof buf);
    expect("empty_dir_bytes", n, 24 + 24);
    expect("empty_dir_end", syscall3(SYS_getdents, fd, buf, sizeof buf), 0);
    a_close(fd);

    expect("cleanup_long", a_unlink(D "/a_rather_long_name_of_thirty_two_b"), 0);
    expect("cleanup_fifo", a_unlink(D "/fifo"), 0);
    expect("cleanup_link", a_unlink(D "/link"), 0);
    expect("cleanup_file", a_unlink(D "/file"), 0);
    expect("cleanup_subdir", a_rmdir(D "/subdir"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
