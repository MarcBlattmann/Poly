/* getdents after changes to a directory: after lseek(fd, 0, SEEK_SET)
 * the listing shows names created since (files, directories, symbolic
 * links, hard links), no longer shows names unlinked or removed, and
 * shows a renamed entry under its new name only, with the same inode
 * number. A descriptor opened before the changes sees them too. */
#include "a_sys.h"

#define SYS_getdents 78

struct g_dirent {
    u64 d_ino;
    u64 d_off;
    u16 d_reclen;
    char d_name[];
};

#define D "/tmp/a_getdents_changes"

static char buf[4096] __attribute__((aligned(8)));
static const char *names[] = {".", "..", "keep", "gone", "old", "new", "sub", "sym", "hard", "rmd"};
#define N 10
static int seen[N];
static u64 inos[N];

static void list(i64 fd) {
    for (int i = 0; i < N; i++) seen[i] = 0, inos[i] = 0;
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    i64 n;
    while ((n = syscall3(SYS_getdents, fd, buf, sizeof buf)) > 0)
        for (i64 o = 0; o < n;) {
            struct g_dirent *d = (struct g_dirent *)(buf + o);
            for (int i = 0; i < N; i++)
                if (str_eq(d->d_name, names[i])) seen[i]++, inos[i] = d->d_ino;
            if (d->d_reclen == 0) return;
            o += d->d_reclen;
        }
}

static void want(const char *tag, const int *w) {
    char c[64];
    for (int i = 2; i < N; i++) {
        str_cat2(c, tag, names[i]);
        expect(c, seen[i], w[i]);
    }
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_keep", a_make_file(D "/keep", "", 0644), 0);
    expect("setup_gone", a_make_file(D "/gone", "", 0644), 0);
    expect("setup_old", a_make_file(D "/old", "", 0644), 0);
    expect("setup_rmd", a_mkdir(D "/rmd", 0755), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    list(fd);
    static const int before[N] = {1, 1, 1, 1, 1, 0, 0, 0, 0, 1};
    want("before_", before);
    u64 old_ino = inos[4];

    expect("change_unlink", a_unlink(D "/gone"), 0);
    expect("change_rename", syscall4(SYS_renameat, AT_FDCWD, D "/old", AT_FDCWD, D "/new"), 0);
    expect("change_mkdir", a_mkdir(D "/sub", 0755), 0);
    expect("change_symlink", syscall3(SYS_symlinkat, "keep", AT_FDCWD, D "/sym"), 0);
    expect("change_link", syscall5(SYS_linkat, AT_FDCWD, D "/keep", AT_FDCWD, D "/hard", 0), 0);
    expect("change_rmdir", a_rmdir(D "/rmd"), 0);

    list(fd);
    static const int after[N] = {1, 1, 1, 0, 0, 1, 1, 1, 1, 0};
    want("after_", after);
    check("renamed_keeps_inode", inos[5] == old_ino && old_ino != 0, 0);
    check("hard_link_same_inode", inos[8] == inos[2] && inos[2] != 0, 0);
    a_close(fd);

    expect("cleanup_keep", a_unlink(D "/keep"), 0);
    expect("cleanup_new", a_unlink(D "/new"), 0);
    expect("cleanup_sym", a_unlink(D "/sym"), 0);
    expect("cleanup_hard", a_unlink(D "/hard"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
