/* renameat2 without flags: moves a name, replaces an existing file
 * atomically, keeps the inode; open descriptors follow the file. */
#include "a_sys.h"

#define D "/tmp/a_renameat2_basic"

static i64 rename2(const char *a, const char *b, i64 flags) {
    return syscall5(SYS_renameat2, AT_FDCWD, a, AT_FDCWD, b, flags);
}

int test_main(void) {
    struct a_stat st, st2;
    char buf[8];
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0700), 0);
    expect("setup_a", a_make_file(D "/a", "AAA", 0644), 0);
    expect("setup_b", a_make_file(D "/b", "B", 0644), 0);
    a_stat_path(D "/a", &st);
    i64 fd = a_open(D "/a", O_RDONLY, 0);

    expect("rename_a_to_c", rename2(D "/a", D "/c", 0), 0);
    expect("old_name_gone", a_stat_path(D "/a", &st2), -ENOENT);
    expect("new_name_there", a_stat_path(D "/c", &st2), 0);
    check("same_inode", st.st_ino == st2.st_ino, 0);
    expect("open_fd_still_reads", a_read(fd, buf, 8), 3);
    a_close(fd);

    expect("rename_over_existing", rename2(D "/c", D "/b", 0), 0);
    expect("source_gone", a_stat_path(D "/c", &st2), -ENOENT);
    a_stat_path(D "/b", &st2);
    check("target_replaced", st2.st_ino == st.st_ino && st2.st_size == 3, st2.st_size);

    expect("rename_into_subdir", rename2(D "/b", D "/sub/b", 0), 0);
    expect("moved_into_subdir", a_stat_path(D "/sub/b", &st2), 0);
    expect("rename_to_itself", rename2(D "/sub/b", D "/sub/b", 0), 0);
    expect("still_there_after_self_rename", a_stat_path(D "/sub/b", &st2), 0);
    expect("hard_link", syscall5(SYS_linkat, AT_FDCWD, D "/sub/b", AT_FDCWD, D "/sub/h", 0), 0);
    expect("rename_onto_own_hard_link", rename2(D "/sub/b", D "/sub/h", 0), 0);
    expect("both_names_kept", a_stat_path(D "/sub/b", &st2) == 0 && a_stat_path(D "/sub/h", &st2) == 0, 1);
    expect("missing_source_enoent", rename2(D "/none", D "/x", 0), -ENOENT);
    expect("missing_target_dir_enoent", rename2(D "/sub/b", D "/nodir/x", 0), -ENOENT);
    expect("rename_dir", rename2(D "/sub", D "/sub2", 0), 0);
    expect("contents_moved_with_dir", a_stat_path(D "/sub2/b", &st2), 0);
    /* The old rename call. */
    expect("rename_syscall", syscall2(SYS_rename, D "/sub2/b", D "/sub2/c"), 0);

    expect("cleanup_c", a_unlink(D "/sub2/c"), 0);
    expect("cleanup_h", a_unlink(D "/sub2/h"), 0);
    expect("cleanup_sub2", a_rmdir(D "/sub2"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
