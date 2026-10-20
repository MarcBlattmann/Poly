/* symlinkat: the new name is relative to the directory descriptor; the
 * target text is resolved later, relative to the link's own directory. */
#include "a_sys.h"

#define D "/tmp/a_symlinkat_dirfd"

int test_main(void) {
    char buf[64];
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0700), 0);
    expect("setup_file", a_make_file(D "/sub/t", "in sub", 0644), 0);
    expect("setup_top_file", a_make_file(D "/t", "at top", 0644), 0);
    i64 dfd = a_open(D "/sub", O_RDONLY | O_DIRECTORY, 0);
    expect("symlinkat_relative", syscall3(SYS_symlinkat, "t", dfd, "l"), 0);
    expect("made_in_dirfd", a_lstat_path(D "/sub/l", &st), 0);
    /* "t" is looked up next to the link, not in the caller's directory. */
    i64 fd = a_open(D "/sub/l", O_RDONLY, 0);
    i64 n = a_read(fd, buf, sizeof buf);
    expect("resolved_next_to_link", n, 6);
    check("resolved_contents", mem_eq(buf, "in sub", 6), buf[0]);
    a_close(fd);
    expect("symlinkat_dotdot_target", syscall3(SYS_symlinkat, "../t", dfd, "up"), 0);
    fd = a_open(D "/sub/up", O_RDONLY, 0);
    n = a_read(fd, buf, sizeof buf);
    check("dotdot_target_resolves", n == 6 && mem_eq(buf, "at top", 6), n);
    a_close(fd);
    /* A chain of links resolves through each one. */
    expect("chain_1", syscall3(SYS_symlinkat, "l", dfd, "l2"), 0);
    expect("chain_2", syscall3(SYS_symlinkat, "l2", dfd, "l3"), 0);
    expect("stat_through_chain", a_stat_path(D "/sub/l3", &st), 0);
    expect("chain_reaches_file", st.st_size, 6);
    /* A loop is ELOOP when followed. */
    expect("loop_a", syscall3(SYS_symlinkat, "lb", dfd, "la"), 0);
    expect("loop_b", syscall3(SYS_symlinkat, "la", dfd, "lb"), 0);
    expect("loop_eloop", a_open(D "/sub/la", O_RDONLY, 0), -ELOOP);
    expect("loop_lstat_ok", a_lstat_path(D "/sub/la", &st), 0);
    i64 ffd = a_open(D "/t", O_RDONLY, 0);
    expect("file_dirfd_enotdir", syscall3(SYS_symlinkat, "x", ffd, "y"), -ENOTDIR);
    a_close(ffd);
    expect("bad_dirfd_ebadf", syscall3(SYS_symlinkat, "x", 99, "y"), -EBADF);
    a_close(dfd);
    const char *names[] = {D "/sub/l", D "/sub/up", D "/sub/l2", D "/sub/l3", D "/sub/la", D "/sub/lb",
                           D "/sub/t", D "/t"};
    int ok = 1;
    for (int i = 0; i < 8; i++) ok &= a_unlink(names[i]) == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
