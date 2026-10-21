/* stat: directories, their link counts, the root, and spellings of a
 * directory's path (trailing slashes, ".", "..", repeated slashes). */
#include "a_sys.h"

#define D "/tmp/a_stat_dir"

static i64 st(const char *p, struct a_stat *s) { return syscall2(SYS_stat, p, s); }

int test_main(void) {
    struct a_stat s, t;
    a_rmdir(D "/sub");
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0711), 0);
    expect("stat_dir", st(D, &s), 0);
    expect("dir_mode", s.st_mode, S_IFDIR | 0711);
    expect("empty_dir_nlink_2", s.st_nlink, 2);
    check("dir_size_positive", s.st_size > 0, s.st_size);
    expect("dir_owner", s.st_uid, syscall0(SYS_geteuid));

    expect("trailing_slash", st(D "/", &t), 0);
    check("trailing_slash_same_inode", t.st_ino == s.st_ino && t.st_dev == s.st_dev, 0);
    expect("many_trailing_slashes", st(D "///", &t), 0);
    check("many_trailing_same_inode", t.st_ino == s.st_ino, 0);
    expect("trailing_dot", st(D "/.", &t), 0);
    check("trailing_dot_same_inode", t.st_ino == s.st_ino, 0);
    expect("repeated_slashes", st("//tmp//a_stat_dir", &t), 0);
    check("repeated_slashes_same_inode", t.st_ino == s.st_ino, 0);

    expect("mkdir_sub", a_mkdir(D "/sub", 0700), 0);
    st(D, &s);
    expect("nlink_3_with_subdir", s.st_nlink, 3);
    expect("sub_dotdot", st(D "/sub/..", &t), 0);
    check("sub_dotdot_is_parent", t.st_ino == s.st_ino, 0);
    expect("sub_dot_dot_sub", st(D "/sub/../sub/.", &t), 0);
    expect("sub_mode", t.st_mode, S_IFDIR | 0700);

    struct a_stat r, rp;
    expect("root", st("/", &r), 0);
    expect("root_is_dir", r.st_mode & S_IFMT, S_IFDIR);
    expect("root_dotdot", st("/..", &rp), 0);
    check("root_dotdot_is_root", rp.st_ino == r.st_ino && rp.st_dev == r.st_dev, 0);
    expect("root_dotdot_dotdot_tmp", st("/../../tmp/../tmp", &rp), 0);
    expect("tmp_is_dir", rp.st_mode & S_IFMT, S_IFDIR);

    expect("rmdir_sub", a_rmdir(D "/sub"), 0);
    st(D, &s);
    expect("nlink_back_to_2", s.st_nlink, 2);
    expect("removed_sub_enoent", st(D "/sub", &t), -ENOENT);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
