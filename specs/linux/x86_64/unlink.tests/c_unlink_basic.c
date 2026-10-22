/* unlink: a file's name goes away; a second unlink of the same name is
 * ENOENT; the directory no longer lists it; the name can be used again. */
#include "a_sys.h"

#define D "/tmp/c_unlink_basic"

static i64 u(const char *p) { return syscall1(SYS_unlink, p); }

/* Number of entries getdents64 returns for the directory, "." and ".."
 * included; *seen is set when an entry is called `name`. */
static i64 count_entries(const char *dir, const char *name, int *seen) {
    char buf[4096];
    i64 fd = a_open(dir, O_RDONLY | O_DIRECTORY, 0), n = 0, r;
    *seen = 0;
    if (fd < 0) return fd;
    while ((r = syscall3(SYS_getdents64, fd, buf, sizeof buf)) > 0)
        for (i64 off = 0; off < r;) {
            struct a_dirent64 *e = (struct a_dirent64 *)(buf + off);
            if (str_eq(e->d_name, name)) *seen = 1;
            n++;
            off += e->d_reclen;
        }
    a_close(fd);
    return n;
}

int test_main(void) {
    struct a_stat st;
    int seen;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "hello", 0644), 0);
    expect("listed_before", count_entries(D, "f", &seen), 3);
    expect("seen_before", seen, 1);
    expect("unlink_file", u(D "/f"), 0);
    expect("stat_gone_enoent", a_stat_path(D "/f", &st), -ENOENT);
    expect("lstat_gone_enoent", a_lstat_path(D "/f", &st), -ENOENT);
    expect("open_gone_enoent", a_open(D "/f", O_RDONLY, 0), -ENOENT);
    expect("listed_after", count_entries(D, "f", &seen), 2);
    expect("not_seen_after", seen, 0);
    expect("unlink_again_enoent", u(D "/f"), -ENOENT);
    expect("recreate", a_make_file(D "/f", "", 0600), 0);
    expect("recreated_stat", a_stat_path(D "/f", &st), 0);
    expect("recreated_empty", st.st_size, 0);
    expect("recreated_nlink_1", st.st_nlink, 1);
    expect("unlink_recreated", u(D "/f"), 0);
    expect("dir_empty_rmdir", syscall1(SYS_rmdir, D), 0);
    return finish();
}
