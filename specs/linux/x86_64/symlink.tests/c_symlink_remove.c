/* symlink: the link is a name of its own. unlink removes the link and
 * leaves the target; rmdir on a link to a directory is ENOTDIR; rename
 * moves the link with its text; a hard link to the link (linkat without
 * AT_SYMLINK_FOLLOW) shares its inode (two links). Removing the target
 * leaves the link dangling, and a new target under the same name is
 * found again. The directory lists the link as DT_LNK. */
#include "a_sys.h"

#define D "/tmp/c_symlink_remove"

static char buf[64];
static char dents[4096];

/* d_type of `name` in D, or -1. */
static int dtype_of(const char *name) {
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    if (fd < 0) return -2;
    int t = -1;
    for (;;) {
        i64 n = syscall3(SYS_getdents64, fd, dents, sizeof dents);
        if (n <= 0) break;
        for (i64 off = 0; off < n;) {
            unsigned short reclen = *(unsigned short *)(dents + off + 16);
            if (str_eq(dents + off + 19, name)) t = (unsigned char)dents[off + 18];
            off += reclen;
        }
    }
    a_close(fd);
    return t;
}

int test_main(void) {
    struct a_stat st, ln;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("link_file", syscall2(SYS_symlink, "f", D "/lf"), 0);
    expect("link_dir", syscall2(SYS_symlink, "sub", D "/ld"), 0);
    expect("dirent_type_lnk", dtype_of("lf"), DT_LNK);
    expect("dirent_type_reg", dtype_of("f"), DT_REG);

    expect("rmdir_dirlink_enotdir", a_rmdir(D "/ld"), -ENOTDIR);
    expect("dir_still_there", a_stat_path(D "/sub", &st), 0);
    expect("unlink_dirlink", a_unlink(D "/ld"), 0);
    expect("dir_kept", a_stat_path(D "/sub", &st), 0);

    expect("hardlink_to_link", syscall5(SYS_linkat, AT_FDCWD, D "/lf", AT_FDCWD, D "/hl", 0), 0);
    expect("lstat_lf", a_lstat_path(D "/lf", &ln), 0);
    expect("link_nlink_two", ln.st_nlink, 2);
    expect("lstat_hl", a_lstat_path(D "/hl", &st), 0);
    check("same_inode", st.st_ino == ln.st_ino, 0);
    expect("hl_is_link", st.st_mode, S_IFLNK | 0777);
    expect("target_nlink_one", a_stat_path(D "/f", &st), 0);
    expect("target_nlink", st.st_nlink, 1);
    expect("unlink_hl", a_unlink(D "/hl"), 0);

    expect("rename_link", syscall2(SYS_rename, D "/lf", D "/sub/moved"), 0);
    expect("old_name_gone", a_lstat_path(D "/lf", &st), -ENOENT);
    expect("moved_text", syscall3(SYS_readlink, D "/sub/moved", buf, sizeof buf), 1);
    check("moved_text_kept", buf[0] == 'f', buf[0]);
    expect("moved_resolves_in_new_dir", a_stat_path(D "/sub/moved", &st), -ENOENT);
    expect("rename_back", syscall2(SYS_rename, D "/sub/moved", D "/lf"), 0);
    expect("resolves_again", a_stat_path(D "/lf", &st), 0);

    expect("unlink_target", a_unlink(D "/f"), 0);
    expect("now_dangling", a_stat_path(D "/lf", &st), -ENOENT);
    expect("link_survives", a_lstat_path(D "/lf", &st), 0);
    expect("new_target", a_make_file(D "/f", "longer", 0600), 0);
    expect("found_again", a_stat_path(D "/lf", &st), 0);
    expect("new_target_size", st.st_size, 6);

    expect("unlink_link", a_unlink(D "/lf"), 0);
    expect("link_gone", a_lstat_path(D "/lf", &st), -ENOENT);
    expect("target_kept", a_stat_path(D "/f", &st), 0);
    expect("dirent_gone", dtype_of("lf"), -1);

    int ok = a_unlink(D "/f") == 0;
    ok &= a_rmdir(D "/sub") == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
