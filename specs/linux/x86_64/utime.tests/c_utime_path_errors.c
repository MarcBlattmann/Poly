/* utime path errors: missing names, empty path, a file used as a directory,
 * names and paths that are too long. A failed call changes nothing. */
#include "a_sys.h"

#define SYS_utime 132
#define D "/tmp/c_utime_path_errors"

struct utimbuf_k {
    i64 actime, modtime;
};

static char longname[400];
static char longpath[4200];

static void build(size_t len) {
    str_cpy(longpath, D);
    size_t n = str_len(longpath);
    if ((len - n) % 2) longpath[n++] = '/';
    while (n < len - 2) {
        longpath[n++] = '/';
        longpath[n++] = '.';
    }
    longpath[n++] = '/';
    longpath[n++] = 'f';
    longpath[n] = 0;
}

int test_main(void) {
    struct a_stat st;
    struct utimbuf_k t = {111, 222};
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    struct utimbuf_k base = {5, 6};
    expect("setup_times", syscall2(SYS_utime, D "/f", &base), 0);
    expect("missing", syscall2(SYS_utime, D "/nope", &t), -ENOENT);
    expect("missing_null", syscall2(SYS_utime, D "/nope", 0), -ENOENT);
    expect("missing_dir", syscall2(SYS_utime, D "/nodir/f", &t), -ENOENT);
    expect("empty", syscall2(SYS_utime, "", &t), -ENOENT);
    expect("empty_null", syscall2(SYS_utime, "", 0), -ENOENT);
    expect("file_as_dir", syscall2(SYS_utime, D "/f/x", &t), -ENOTDIR);
    /* A 256-byte name. */
    str_cpy(longname, D "/");
    size_t n = str_len(longname);
    for (size_t i = 0; i < 256; i++) longname[n + i] = 'a';
    longname[n + 256] = 0;
    expect("name_256", syscall2(SYS_utime, longname, &t), -ENAMETOOLONG);
    longname[n + 255] = 0;
    expect("name_255_missing", syscall2(SYS_utime, longname, &t), -ENOENT);
    /* Paths of D, "/." repeated, then "/f" (a doubled slash makes the
     * length odd): 4096 bytes before the NUL is too long, 4095 resolves. */
    build(4096);
    check("path_len_4096", str_len(longpath) == 4096, (i64)str_len(longpath));
    expect("path_4096", syscall2(SYS_utime, longpath, &t), -ENAMETOOLONG);
    build(4200 - 2);
    expect("path_4198", syscall2(SYS_utime, longpath, &t), -ENAMETOOLONG);
    build(4095);
    check("path_len_4095", str_len(longpath) == 4095, (i64)str_len(longpath));
    expect("path_4095_ok", syscall2(SYS_utime, longpath, &t), 0);
    a_stat_path(D "/f", &st);
    check("short_path_applied", st.st_atime_sec == 111 && st.st_mtime_sec == 222, (i64)st.st_mtime_sec);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
