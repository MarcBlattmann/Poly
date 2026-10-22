/* rename with problems in both paths: which error wins. Each path is
 * checked as a string (unreadable EFAULT, 4096 bytes or more
 * ENAMETOOLONG, empty ENOENT) and its directories walked before the
 * other path's; the old path goes first. Then the old name is looked up
 * (missing ENOENT, over 255 bytes ENAMETOOLONG), then the new name
 * (over 255 bytes ENAMETOOLONG), then the rename itself is judged. */
#include "a_sys.h"

#define D "/tmp/a_rename_order"

static i64 rn(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }

static char n256[300], long_src[400], long_dst[400], long_in_nope[400], long_in_file[400];
static char path4096[4200];

int test_main(void) {
    for (int i = 0; i < 256; i++) n256[i] = 'n';
    str_cat2(long_src, D "/", n256);
    str_cat2(long_dst, D "/", n256);
    str_cat2(long_in_nope, D "/nope/", n256);
    str_cat2(long_in_file, D "/f/", n256);
    char *q = str_cpy(path4096, "/");
    while (q - path4096 < 4096) *q++ = 'a';
    *q = 0;

    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    expect("setup_d", a_mkdir(D "/d", 0755), 0);

    /* Strings are read before anything is looked up. */
    expect("src_null_dst_empty", rn(0, ""), -EFAULT);
    expect("src_empty_dst_null", rn("", 0), -ENOENT);
    expect("src_missing_dst_null", rn(D "/missing", 0), -EFAULT);
    expect("src_missing_dst_empty", rn(D "/missing", ""), -ENOENT);
    expect("src_missing_dst_4096", rn(D "/missing", path4096), -ENAMETOOLONG);
    expect("src_long_dst_4096", rn(long_src, path4096), -ENAMETOOLONG);
    expect("src_4096_dst_null", rn(path4096, 0), -ENAMETOOLONG);

    /* The old path's directories come before the new path's string. */
    expect("src_missing_dir_dst_null", rn(D "/nope/x", 0), -ENOENT);
    expect("src_file_dir_dst_null", rn(D "/f/x", 0), -ENOTDIR);
    expect("src_missing_dir_dst_4096", rn(D "/nope/x", path4096), -ENOENT);
    expect("src_missing_dir_dst_empty", rn(D "/nope/x", ""), -ENOENT);
    expect("src_long_in_missing_dir", rn(long_in_nope, path4096), -ENOENT);

    /* Both directories before either last name. */
    expect("src_missing_dst_missing_dir", rn(D "/missing", D "/nope/x"), -ENOENT);
    expect("src_missing_dst_file_dir", rn(D "/missing", D "/f/y"), -ENOTDIR);
    expect("src_long_dst_file_dir", rn(long_src, D "/f/x"), -ENOTDIR);
    expect("src_long_dst_missing_dir", rn(long_src, D "/nope/x"), -ENOENT);
    expect("src_missing_dst_long_in_file", rn(D "/missing", long_in_file), -ENOTDIR);

    /* Then the old name, then the new name. */
    expect("src_missing_dst_long", rn(D "/missing", long_dst), -ENOENT);
    expect("src_long_dst_long", rn(long_src, long_dst), -ENAMETOOLONG);
    expect("src_ok_dst_long", rn(D "/f", long_dst), -ENAMETOOLONG);
    expect("src_dot_dst_long", rn(D "/d/.", long_dst), -EBUSY);
    expect("src_missing_dst_dot", rn(D "/missing", D "/d/."), -EBUSY);

    /* Then the rename itself. */
    expect("file_over_dir", rn(D "/f", D "/d"), -EISDIR);
    expect("dir_over_file", rn(D "/d", D "/f"), -ENOTDIR);
    expect("dir_into_itself", rn(D "/d", D "/d/x"), -EINVAL);

    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_d", a_rmdir(D "/d"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
