/* rmdir and name lengths: a name of 255 bytes works (also with a
 * trailing slash); a last name of 256 bytes or more is ENAMETOOLONG, as
 * is a path of 4096 bytes or more counting its NUL (4095 bytes plus NUL
 * works, even when padded with "./"). A too-long path is ENAMETOOLONG
 * before anything else is looked at, even under a missing directory; a
 * too-long component is reported only once the directories before it
 * exist (D/nope/<256> is ENOENT, D/f/<256> ENOTDIR). */
#include "a_sys.h"

#define D "/tmp/c_rmdir_name_length"

static i64 rm(const char *p) { return syscall1(SYS_rmdir, p); }
static char buf[8192];

/* D "/" + n copies of 'n' + suffix. */
static char *name(size_t n, const char *prefix, const char *suffix) {
    char *p = str_cpy(buf, prefix);
    memset(p, 'n', n);
    str_cpy(p + n, suffix);
    return buf;
}

int test_main(void) {
    struct a_stat st;
    expect("setup", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);

    expect("mk_255", a_mkdir(name(255, D "/", ""), 0755), 0);
    expect("rmdir_255", rm(name(255, D "/", "")), 0);
    expect("gone_255", a_lstat_path(name(255, D "/", ""), &st), -ENOENT);
    expect("mk_255_again", a_mkdir(name(255, D "/", ""), 0755), 0);
    expect("rmdir_255_slash", rm(name(255, D "/", "/")), 0);

    expect("missing_255_enoent", rm(name(255, D "/", "")), -ENOENT);
    expect("last_256_enametoolong", rm(name(256, D "/", "")), -ENAMETOOLONG);
    expect("last_256_slash_enametoolong", rm(name(256, D "/", "/")), -ENAMETOOLONG);
    expect("last_1000_enametoolong", rm(name(1000, D "/", "")), -ENAMETOOLONG);
    expect("way_256_enametoolong", rm(name(256, D "/", "/x")), -ENAMETOOLONG);
    expect("missing_then_256_enoent", rm(name(256, D "/nope/", "")), -ENOENT);
    expect("missing_then_256_then_more_enoent", rm(name(256, D "/nope/", "/x")), -ENOENT);
    expect("file_then_256_enotdir", rm(name(256, D "/f/", "")), -ENOTDIR);
    expect("256_then_missing_enametoolong", rm(name(256, D "/", "/nope/x")), -ENAMETOOLONG);

    /* Whole path length: build "D/" + "./" * k + "t" to exactly 4095 and 4096 bytes. */
    expect("mk_t", a_mkdir(D "/t", 0755), 0);
    {
        char *p = str_cpy(buf, D "/");
        size_t len = (size_t)(p - buf);
        while (len + 1 < 4095) { *p++ = '.'; *p++ = '/'; len += 2; }
        if (len + 1 > 4095) { p -= 2; len -= 2; *p++ = '/'; len += 1; }
        *p++ = 't';
        *p = 0;
        expect("path_4095_bytes", (i64)str_len(buf), 4095);
        expect("rmdir_path_4095_works", rm(buf), 0);
    }
    expect("mk_t2", a_mkdir(D "/t", 0755), 0);
    {
        char *p = str_cpy(buf, D "/");
        size_t len = (size_t)(p - buf);
        while (len + 1 < 4096) { *p++ = '.'; *p++ = '/'; len += 2; }
        if (len + 1 > 4096) { p -= 2; len -= 2; *p++ = '/'; len += 1; }
        *p++ = 't';
        *p = 0;
        expect("path_4096_bytes", (i64)str_len(buf), 4096);
        expect("rmdir_path_4096_enametoolong", rm(buf), -ENAMETOOLONG);
        expect("t_kept", a_stat_path(D "/t", &st), 0);
    }
    {
        char *p = str_cpy(buf, D "/nope/");
        memset(p, '/', 5000);
        str_cpy(p + 5000, "x");
        expect("long_path_under_missing_enametoolong", rm(buf), -ENAMETOOLONG);
    }
    expect("rm_t", rm(D "/t"), 0);

    expect("cleanup_file", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", rm(D), 0);
    return finish();
}
