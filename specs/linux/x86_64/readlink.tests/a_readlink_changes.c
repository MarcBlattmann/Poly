/* readlink: follows what the file system holds now: a rename over a link
 * shows the new text, a renamed link keeps its text, a hard link to a
 * symbolic link reads the same, an unlinked link is ENOENT; st_size of a
 * link equals what readlink returns. */
#include "a_sys.h"

#define D "/tmp/a_readlink_changes"

static i64 rl(const char *p, char *buf, i64 n) { return syscall3(SYS_readlink, p, buf, n); }

int test_main(void) {
    char buf[64];
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_l", syscall2(SYS_symlink, "first-target-text", D "/l"), 0);
    expect("first", rl(D "/l", buf, sizeof buf), 17);
    expect("lstat", a_lstat_path(D "/l", &st), 0);
    expect("st_size_matches", st.st_size, 17);
    expect("setup_new", syscall2(SYS_symlink, "second", D "/new"), 0);
    expect("rename_over", syscall2(SYS_rename, D "/new", D "/l"), 0);
    memset(buf, 'z', sizeof buf);
    expect("after_rename_over", rl(D "/l", buf, sizeof buf), 6);
    check("after_rename_over_text", mem_eq(buf, "second", 6) && buf[6] == 'z', buf[6]);
    expect("old_name_gone", rl(D "/new", buf, sizeof buf), -ENOENT);
    expect("rename_link", syscall2(SYS_rename, D "/l", D "/moved"), 0);
    expect("moved_text", rl(D "/moved", buf, sizeof buf), 6);
    expect("hardlink_to_symlink", syscall5(SYS_linkat, AT_FDCWD, D "/moved", AT_FDCWD, D "/hard", 0), 0);
    memset(buf, 'z', sizeof buf);
    expect("hardlink_text", rl(D "/hard", buf, sizeof buf), 6);
    check("hardlink_text_same", mem_eq(buf, "second", 6), buf[0]);
    expect("unlink_moved", a_unlink(D "/moved"), 0);
    expect("unlinked", rl(D "/moved", buf, sizeof buf), -ENOENT);
    expect("hardlink_survives", rl(D "/hard", buf, sizeof buf), 6);
    /* Renaming the directory moves the links with it. */
    expect("rename_dir", syscall2(SYS_rename, D, D "2"), 0);
    expect("in_renamed_dir", rl(D "2/hard", buf, sizeof buf), 6);
    expect("old_dir_gone", rl(D "/hard", buf, sizeof buf), -ENOENT);
    expect("cleanup_hard", a_unlink(D "2/hard"), 0);
    expect("cleanup_rmdir", a_rmdir(D "2"), 0);
    return finish();
}
