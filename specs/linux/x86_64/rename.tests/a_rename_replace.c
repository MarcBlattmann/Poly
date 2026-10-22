/* rename over an existing file: the target name now refers to the
 * source inode. The replaced inode survives while another name or an
 * open descriptor holds it (its link count drops by one); the source
 * keeps its link count. The directory then lists each name once. */
#include "a_sys.h"

#define D "/tmp/a_rename_replace"

static i64 rn(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }

/* Count entries of `path` other than "." and "..", and whether `name` is one. */
static i64 count_entries(const char *path, const char *name, int *found) {
    static char buf[4096];
    i64 fd = a_open(path, O_RDONLY | O_DIRECTORY, 0);
    if (fd < 0) return fd;
    i64 n, count = 0;
    *found = 0;
    while ((n = syscall3(SYS_getdents64, fd, buf, sizeof buf)) > 0) {
        for (i64 off = 0; off < n;) {
            struct a_dirent64 *d = (struct a_dirent64 *)(buf + off);
            if (!str_eq(d->d_name, ".") && !str_eq(d->d_name, "..")) {
                count++;
                if (str_eq(d->d_name, name)) *found = 1;
            }
            off += d->d_reclen;
        }
    }
    a_close(fd);
    return count;
}

int test_main(void) {
    struct a_stat sa, sb, st;
    char buf[8];
    int found;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_a", a_make_file(D "/a", "new", 0600), 0);
    expect("setup_b", a_make_file(D "/b", "old!", 0644), 0);
    a_stat_path(D "/a", &sa);
    a_stat_path(D "/b", &sb);
    i64 bfd = a_open(D "/b", O_RDONLY, 0);

    expect("a_over_b", rn(D "/a", D "/b"), 0);
    expect("a_gone", a_stat_path(D "/a", &st), -ENOENT);
    expect("b_is_a", a_stat_path(D "/b", &st), 0);
    check("b_inode_is_a", st.st_ino == sa.st_ino, 0);
    expect("b_mode_is_a", st.st_mode, S_IFREG | 0600);
    expect("b_size_is_a", st.st_size, 3);
    syscall2(SYS_fstat, bfd, &st);
    check("old_b_fd_same_inode", st.st_ino == sb.st_ino, 0);
    expect("old_b_nlink_0", st.st_nlink, 0);
    expect("old_b_fd_reads_old", a_read(bfd, buf, 8), 4);
    check("old_b_contents", mem_eq(buf, "old!", 4), 0);
    a_close(bfd);
    expect("one_entry", count_entries(D, "b", &found), 1);
    expect("entry_is_b", found, 1);

    /* The replaced file has a second name: it survives with one link less. */
    expect("setup_c", a_make_file(D "/c", "C", 0644), 0);
    expect("link_c_c2", syscall5(SYS_linkat, AT_FDCWD, D "/c", AT_FDCWD, D "/c2", 0), 0);
    expect("b_over_c", rn(D "/b", D "/c"), 0);
    expect("c2_survives", a_stat_path(D "/c2", &st), 0);
    expect("c2_nlink_1", st.st_nlink, 1);
    expect("c2_size", st.st_size, 1);
    expect("c_is_a", a_stat_path(D "/c", &st), 0);
    check("c_inode_is_a", st.st_ino == sa.st_ino, 0);

    /* A source with two names: moving one of them keeps the count. */
    expect("link_c_h", syscall5(SYS_linkat, AT_FDCWD, D "/c", AT_FDCWD, D "/h", 0), 0);
    expect("h_over_c2", rn(D "/h", D "/c2"), 0);
    expect("c2_now_a", a_stat_path(D "/c2", &st), 0);
    check("c2_inode_is_a", st.st_ino == sa.st_ino, 0);
    expect("a_nlink_2", st.st_nlink, 2);
    expect("h_gone", a_stat_path(D "/h", &st), -ENOENT);
    expect("two_entries", count_entries(D, "c2", &found), 2);
    expect("entry_c2", found, 1);

    expect("cleanup_c", a_unlink(D "/c"), 0);
    expect("cleanup_c2", a_unlink(D "/c2"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
