/* rename path errors, for either path: empty ENOENT; a missing
 * directory ENOENT; a file used as a directory ENOTDIR; a component of
 * 256 bytes ENAMETOOLONG (255 is fine); a path of 4096 bytes or more
 * with its NUL ENAMETOOLONG (4095 is fine); an unreadable pointer
 * EFAULT. Nothing changes on failure. */
#include "a_sys.h"

#define D "/tmp/a_rename_path_errors"

static i64 rn(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }

static char name255[300], name256[300], p255[400], p256[400];
static char path4095[4200], path4096[4200];

/* D "/" then "./" repeated, then `last`, padded with slashes to `len` bytes. */
static void long_path(char *out, i64 len, const char *last) {
    char *q = str_cpy(out, D "/");
    i64 tail = (i64)str_len(last);
    while (q - out + 2 <= len - tail) *q++ = '.', *q++ = '/';
    while (q - out < len - tail) *q++ = '/';
    str_cpy(q, last);
}

int test_main(void) {
    struct a_stat st;
    for (int i = 0; i < 256; i++) name256[i] = 'n';
    for (int i = 0; i < 255; i++) name255[i] = 'n';
    str_cat2(p255, D "/", name255);
    str_cat2(p256, D "/", name256);
    long_path(path4095, 4095, "f");
    long_path(path4096, 4096, "f");
    expect("path4095_len", str_len(path4095), 4095);
    expect("path4096_len", str_len(path4096), 4096);

    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);

    expect("src_empty", rn("", D "/z"), -ENOENT);
    expect("dst_empty", rn(D "/f", ""), -ENOENT);
    expect("src_missing", rn(D "/none", D "/z"), -ENOENT);
    expect("src_missing_dir", rn(D "/nodir/f", D "/z"), -ENOENT);
    expect("dst_missing_dir", rn(D "/f", D "/nodir/z"), -ENOENT);
    expect("src_file_as_dir", rn(D "/f/x", D "/z"), -ENOTDIR);
    expect("dst_file_as_dir", rn(D "/f", D "/f/x"), -ENOTDIR);
    expect("src_null", rn(0, D "/z"), -EFAULT);
    expect("dst_null", rn(D "/f", 0), -EFAULT);
    expect("src_bad_pointer", rn((const char *)1, D "/z"), -EFAULT);
    expect("dst_bad_pointer", rn(D "/f", (const char *)-1L), -EFAULT);

    expect("src_name256", rn(p256, D "/z"), -ENAMETOOLONG);
    expect("dst_name256", rn(D "/f", p256), -ENAMETOOLONG);
    expect("src_path4096", rn(path4096, D "/z"), -ENAMETOOLONG);
    expect("dst_path4096", rn(D "/f", path4096), -ENAMETOOLONG);
    expect("f_kept", a_stat_path(D "/f", &st), 0);
    expect("nothing_created", a_stat_path(D "/z", &st), -ENOENT);

    expect("dst_name255_ok", rn(D "/f", p255), 0);
    expect("src_name255_ok", rn(p255, D "/f"), 0);
    expect("src_path4095_ok", rn(path4095, D "/g"), 0);
    expect("dst_path4095_ok", rn(D "/g", path4095), 0);
    expect("back_at_f", a_stat_path(D "/f", &st), 0);

    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
