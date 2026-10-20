/* getdents64: every entry of a directory exactly once, with "." and "..",
 * the right types and record lengths; then 0 at the end. The order is
 * not specified, so names are checked as a set. */
#include "a_sys.h"

#define D "/tmp/a_getdents64_list"

static char buf[4096];

static const char *names[] = {".", "..", "file", "dir", "link", "a_much_longer_file_name_0123456789"};
static const int types[] = {DT_DIR, DT_DIR, DT_REG, DT_DIR, DT_LNK, DT_REG};
#define N 6

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/file", "x", 0644), 0);
    expect("setup_dir", a_mkdir(D "/dir", 0700), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "file", AT_FDCWD, D "/link"), 0);
    expect("setup_long", a_make_file(D "/a_much_longer_file_name_0123456789", "", 0644), 0);

    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i64 n = syscall3(SYS_getdents64, fd, buf, sizeof buf);
    /* Each record is 19 header bytes + name + NUL, rounded up to 8. */
    i64 want = 0;
    for (int i = 0; i < N; i++) want += (19 + str_len(names[i]) + 1 + 7) & ~7L;
    expect("all_in_one_call_bytes", n, want);
    int seen[N] = {0}, count = 0, types_ok = 1, reclen_ok = 1, ino_ok = 1;
    for (i64 off = 0; off < n;) {
        struct a_dirent64 *d = (struct a_dirent64 *)(buf + off);
        count++;
        reclen_ok &= d->d_reclen == ((19 + str_len(d->d_name) + 1 + 7) & ~7L);
        ino_ok &= d->d_ino != 0;
        for (int i = 0; i < N; i++)
            if (str_eq(d->d_name, names[i])) {
                seen[i]++;
                types_ok &= d->d_type == types[i];
            }
        off += d->d_reclen;
    }
    expect("entry_count", count, N);
    int each_once = 1;
    for (int i = 0; i < N; i++) each_once &= seen[i] == 1;
    check("each_name_once", each_once, 0);
    check("types", types_ok, 0);
    check("record_lengths", reclen_ok, 0);
    check("inode_numbers_nonzero", ino_ok, 0);
    expect("end_of_directory_0", syscall3(SYS_getdents64, fd, buf, sizeof buf), 0);
    expect("still_0", syscall3(SYS_getdents64, fd, buf, sizeof buf), 0);
    a_close(fd);

    expect("cleanup_long", a_unlink(D "/a_much_longer_file_name_0123456789"), 0);
    expect("cleanup_link", a_unlink(D "/link"), 0);
    expect("cleanup_dir", a_rmdir(D "/dir"), 0);
    expect("cleanup_file", a_unlink(D "/file"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
