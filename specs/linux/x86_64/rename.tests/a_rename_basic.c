/* rename: moves a name within a directory and into another one; the
 * inode, contents and mode go with the name and open descriptors keep
 * working; the old name is gone. Paths with doubled slashes and the
 * renameat call with AT_FDCWD behave the same. */
#include "a_sys.h"

#define D "/tmp/a_rename_basic"

static i64 rn(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }

int test_main(void) {
    struct a_stat st, st2;
    char buf[8];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_a", a_make_file(D "/a", "AAA", 0640), 0);
    expect("stat_a", a_stat_path(D "/a", &st), 0);
    i64 fd = a_open(D "/a", O_RDWR, 0);
    check("open_a", fd >= 0, fd);

    expect("rename_a_to_b", rn(D "/a", D "/b"), 0);
    expect("old_name_gone", a_stat_path(D "/a", &st2), -ENOENT);
    expect("new_name_there", a_stat_path(D "/b", &st2), 0);
    check("same_inode", st.st_ino == st2.st_ino, 0);
    expect("same_mode", st2.st_mode, S_IFREG | 0640);
    expect("same_size", st2.st_size, 3);
    expect("same_nlink", st2.st_nlink, 1);
    expect("fd_reads", a_read(fd, buf, 8), 3);
    expect("fd_writes", a_write(fd, "Z", 1), 1);
    expect("write_seen_under_new_name", a_stat_path(D "/b", &st2) == 0 && st2.st_size == 4, 1);
    a_close(fd);

    expect("rename_into_subdir", rn(D "/b", D "/sub/b"), 0);
    expect("gone_from_parent", a_stat_path(D "/b", &st2), -ENOENT);
    expect("in_subdir", a_stat_path(D "/sub/b", &st2), 0);
    check("still_same_inode", st.st_ino == st2.st_ino, 0);
    expect("rename_out_with_new_name", rn(D "/sub/b", D "/c"), 0);
    expect("out_of_subdir", a_stat_path(D "/c", &st2), 0);

    fd = a_open(D "/c", O_RDONLY, 0);
    expect("contents_kept", a_read(fd, buf, 8), 4);
    check("contents_bytes", mem_eq(buf, "AAAZ", 4), 0);
    a_close(fd);

    expect("double_slashes", rn(D "//c", D "///sub//d"), 0);
    expect("double_slashes_moved", a_stat_path(D "/sub/d", &st2), 0);
    expect("renameat_fdcwd", syscall4(SYS_renameat, AT_FDCWD, D "/sub/d", AT_FDCWD, D "/e"), 0);
    expect("renameat_moved", a_stat_path(D "/e", &st2), 0);
    expect("rename_returns_zero_dir", rn(D "/sub", D "/sub2"), 0);

    expect("cleanup_e", a_unlink(D "/e"), 0);
    expect("cleanup_sub2", a_rmdir(D "/sub2"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
