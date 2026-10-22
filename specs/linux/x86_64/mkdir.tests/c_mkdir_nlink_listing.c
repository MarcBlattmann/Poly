/* A new directory lists exactly "." and ".." (type directory), is listed
 * in its parent as a directory, and can be opened with O_DIRECTORY. Each
 * subdirectory made adds one to the parent's link count (files do not);
 * removing one takes it off again. */
#include "a_sys.h"

#define D "/tmp/c_mkdir_nlink_listing"

static i64 mk(const char *p, i64 m) { return syscall2(SYS_mkdir, p, m); }
static i64 nlink_of(const char *p) {
    struct a_stat st;
    i64 r = a_stat_path(p, &st);
    return r < 0 ? r : (i64)st.st_nlink;
}
static char dents[4096];

/* Bit 0: "." seen as DT_DIR or DT_UNKNOWN, bit 1: "..", bit 2: name, bit 3: anything else. */
static i64 scan(const char *dir, const char *name, int *name_type) {
    i64 fd = a_open(dir, O_RDONLY | O_DIRECTORY, 0);
    if (fd < 0) return fd;
    i64 seen = 0;
    for (;;) {
        i64 n = syscall3(SYS_getdents64, fd, dents, sizeof dents);
        if (n <= 0) break;
        for (i64 off = 0; off < n;) {
            struct a_dirent64 *e = (struct a_dirent64 *)(dents + off);
            if (str_eq(e->d_name, ".")) seen |= 1;
            else if (str_eq(e->d_name, "..")) seen |= 2;
            else if (name && str_eq(e->d_name, name)) {
                seen |= 4;
                *name_type = e->d_type;
            } else seen |= 8;
            off += e->d_reclen;
        }
    }
    a_close(fd);
    return seen;
}

int test_main(void) {
    int t = -1;
    expect("setup", a_mkdir(D, 0755), 0);
    expect("nlink_empty_parent", nlink_of(D), 2);
    expect("mk_a", mk(D "/a", 0755), 0);
    expect("new_lists_dot_dotdot_only", scan(D "/a", 0, &t), 3);
    expect("parent_lists_a", scan(D, "a", &t), 7);
    expect("a_listed_as_dir", t, DT_DIR);
    i64 fd = a_open(D "/a", O_RDONLY | O_DIRECTORY, 0);
    expect("open_o_directory", fd >= 0, 1);
    a_close(fd);
    expect("nlink_after_a", nlink_of(D), 3);
    expect("mk_b", mk(D "/b", 0755), 0);
    expect("mk_c", mk(D "/c", 0755), 0);
    expect("nlink_after_abc", nlink_of(D), 5);
    expect("file", a_make_file(D "/f", "", 0644), 0);
    expect("nlink_file_no_change", nlink_of(D), 5);
    expect("mk_a_sub", mk(D "/a/sub", 0755), 0);
    expect("nlink_a_3", nlink_of(D "/a"), 3);
    expect("nlink_parent_unchanged_by_grandchild", nlink_of(D), 5);
    expect("mk_failed_eexist", mk(D "/b", 0755), -EEXIST);
    expect("nlink_failed_no_change", nlink_of(D), 5);
    expect("rm_b", a_rmdir(D "/b"), 0);
    expect("nlink_after_rm_b", nlink_of(D), 4);
    expect("rm_a_sub", a_rmdir(D "/a/sub"), 0);
    expect("nlink_a_back_2", nlink_of(D "/a"), 2);
    expect("rm_a", a_rmdir(D "/a"), 0);
    expect("rm_c", a_rmdir(D "/c"), 0);
    expect("rm_f", a_unlink(D "/f"), 0);
    expect("nlink_back_2", nlink_of(D), 2);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
