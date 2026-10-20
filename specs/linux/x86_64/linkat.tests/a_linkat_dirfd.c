/* linkat: both names relative to directory descriptors; AT_EMPTY_PATH
 * (needs CAP_DAC_READ_SEARCH, which root has) links an open file. */
#include "a_sys.h"

#define D "/tmp/a_linkat_dirfd"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_a", a_mkdir(D "/a", 0700), 0);
    expect("setup_b", a_mkdir(D "/b", 0700), 0);
    expect("setup_file", a_make_file(D "/a/f", "abc", 0644), 0);
    i64 da = a_open(D "/a", O_RDONLY | O_DIRECTORY, 0);
    i64 db = a_open(D "/b", O_RDONLY | O_DIRECTORY, 0);
    expect("relative_both", syscall5(SYS_linkat, da, "f", db, "g", 0), 0);
    expect("made_in_b", a_stat_path(D "/b/g", &st), 0);
    expect("nlink_2", st.st_nlink, 2);
    expect("bad_old_dirfd_ebadf", syscall5(SYS_linkat, 99, "f", db, "h", 0), -EBADF);
    expect("bad_new_dirfd_ebadf", syscall5(SYS_linkat, da, "f", 99, "h", 0), -EBADF);
    i64 fd = a_open(D "/a/f", O_RDONLY, 0);
    expect("empty_path_links_open_file", syscall5(SYS_linkat, fd, "", db, "e", AT_EMPTY_PATH), 0);
    a_stat_path(D "/b/e", &st);
    expect("nlink_3", st.st_nlink, 3);
    expect("empty_path_without_flag_enoent", syscall5(SYS_linkat, fd, "", db, "e2", 0), -ENOENT);
    a_close(fd);
    /* An O_TMPFILE file gets its first name with AT_EMPTY_PATH. */
    i64 t = a_open(D "/a", O_TMPFILE | O_RDWR, 0600);
    a_write(t, "tmp", 3);
    expect("tmpfile_link", syscall5(SYS_linkat, t, "", db, "named", AT_EMPTY_PATH), 0);
    a_stat_path(D "/b/named", &st);
    expect("tmpfile_named_size", st.st_size, 3);
    expect("tmpfile_named_nlink", st.st_nlink, 1);
    a_close(t);
    a_close(da);
    a_close(db);
    const char *names[] = {D "/a/f", D "/b/g", D "/b/e", D "/b/named"};
    int ok = 1;
    for (int i = 0; i < 4; i++) ok &= a_unlink(names[i]) == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_a", a_rmdir(D "/a"), 0);
    expect("cleanup_b", a_rmdir(D "/b"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
