/* getdents: d_ino of "." is the directory's own inode number, of ".."
 * its parent's (for a nested directory, and in a directory that is a
 * subdirectory of the opened one); for "/" "." is the root itself. A
 * subdirectory entry's d_ino equals the inode number the subdirectory
 * reports for its "." record. */
#include "a_sys.h"

#define SYS_getdents 78

struct g_dirent {
    u64 d_ino;
    u64 d_off;
    u16 d_reclen;
    char d_name[];
};

#define D "/tmp/a_getdents_dot_inodes"

static char buf[4096] __attribute__((aligned(8)));

/* d_ino of `name` in the listing of `dir` (0 if absent). */
static u64 ino_in(const char *dir, const char *name) {
    i64 fd = a_open(dir, O_RDONLY | O_DIRECTORY, 0);
    u64 found = 0;
    i64 n;
    while ((n = syscall3(SYS_getdents, fd, buf, sizeof buf)) > 0)
        for (i64 o = 0; o < n;) {
            struct g_dirent *d = (struct g_dirent *)(buf + o);
            if (str_eq(d->d_name, name)) found = d->d_ino;
            if (d->d_reclen == 0) break;
            o += d->d_reclen;
        }
    a_close(fd);
    return found;
}

static u64 st_ino(const char *path) {
    struct a_stat st;
    return a_stat_path(path, &st) == 0 ? st.st_ino : (u64)-1;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_mkdir_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_mkdir_subsub", a_mkdir(D "/sub/deeper", 0755), 0);

    check("dot_is_self", ino_in(D, ".") == st_ino(D), 0);
    check("dotdot_is_parent", ino_in(D, "..") == st_ino("/tmp"), 0);
    check("sub_dot_is_sub", ino_in(D "/sub", ".") == st_ino(D "/sub"), 0);
    check("sub_dotdot_is_dir", ino_in(D "/sub", "..") == st_ino(D), 0);
    check("deeper_dotdot_is_sub", ino_in(D "/sub/deeper", "..") == st_ino(D "/sub"), 0);
    check("entry_matches_its_dot", ino_in(D, "sub") == ino_in(D "/sub", "."), 0);
    check("entry_matches_child_dotdot", ino_in(D "/sub", "deeper") == ino_in(D "/sub/deeper", "."), 0);
    check("root_dot_is_root", ino_in("/", ".") == st_ino("/"), 0);
    check("nonzero", ino_in(D, ".") != 0 && ino_in(D, "..") != 0, 0);

    expect("cleanup_subsub", a_rmdir(D "/sub/deeper"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
