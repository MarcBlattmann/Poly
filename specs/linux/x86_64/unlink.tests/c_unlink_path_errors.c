/* unlink path errors: empty path, bad pointers, missing or non-directory
 * components, over-long names and paths, and which one wins when a path
 * has several problems. */
#include "a_sys.h"

#define D "/tmp/c_unlink_path_errors"

static i64 u(const char *p) { return syscall1(SYS_unlink, p); }

static char path[8192];

/* D + "/" + prefix + n times 'a' (+ suffix) into path. */
static const char *longname(const char *prefix, int n, const char *suffix) {
    char *p = str_cpy(str_cpy(path, D "/"), prefix);
    for (int i = 0; i < n; i++) *p++ = 'a';
    str_cpy(p, suffix);
    return path;
}

/* D, then slashes, then "f": `len` bytes before the NUL. */
static const char *slashed(int len) {
    char *p = str_cpy(path, D);
    while (p - path < len - 1) *p++ = '/';
    str_cpy(p, "f");
    return path;
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("empty_enoent", u(""), -ENOENT);
    expect("null_efault", u((const char *)0), -EFAULT);
    expect("bad_pointer_efault", u((const char *)-1L), -EFAULT);
    expect("missing_enoent", u(D "/nope"), -ENOENT);
    expect("missing_dir_enoent", u(D "/nope/f"), -ENOENT);
    expect("file_component_enotdir", u(D "/f/x"), -ENOTDIR);
    expect("name_256_enametoolong", u(longname("", 256, "")), -ENAMETOOLONG);
    expect("name_255_missing_enoent", u(longname("", 255, "")), -ENOENT);
    expect("name_255_create", a_make_file(longname("", 255, ""), "", 0644), 0);
    expect("name_255_unlink", u(longname("", 255, "")), 0);
    expect("name_255_gone", a_stat_path(longname("", 255, ""), &st), -ENOENT);
    /* Order: the first failing component decides. */
    expect("missing_then_long_enoent", u(longname("nope/", 256, "")), -ENOENT);
    expect("file_then_long_enotdir", u(longname("f/", 256, "")), -ENOTDIR);
    expect("long_then_missing_enametoolong", u(longname("", 256, "/nope/x")), -ENAMETOOLONG);
    /* A whole path of 4096 bytes or more with its NUL is too long, even
     * when made mostly of slashes; 4095 bytes plus the NUL is fine. */
    expect("path_4094_ok", u(slashed(4094)), 0);
    expect("path_4094_removed", a_stat_path(D "/f", &st), -ENOENT);
    expect("recreate_f", a_make_file(D "/f", "", 0644), 0);
    expect("path_4095_ok", u(slashed(4095)), 0);
    expect("path_4095_removed", a_stat_path(D "/f", &st), -ENOENT);
    expect("recreate_f_2", a_make_file(D "/f", "", 0644), 0);
    expect("path_4096_enametoolong", u(slashed(4096)), -ENAMETOOLONG);
    expect("missing_4096_enametoolong", u(longname("", 4096 - sizeof D, "")), -ENAMETOOLONG);
    expect("f_kept", a_stat_path(D "/f", &st), 0);
    expect("cleanup_f", u(D "/f"), 0);
    expect("cleanup_dir", syscall1(SYS_rmdir, D), 0);
    return finish();
}
