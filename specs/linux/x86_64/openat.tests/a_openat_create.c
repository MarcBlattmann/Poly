/* openat: O_CREAT, O_EXCL, O_TRUNC, the lowest free descriptor, missing
 * paths and the mode of a new file. */
#include "a_sys.h"

#define D "/tmp/a_openat_create"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);

    i64 fd = a_open(D "/f", O_RDWR | O_CREAT, 0640);
    expect("create_returns_lowest_fd_3", fd, 3);
    expect("created_file_is_empty", syscall2(SYS_fstat, fd, &st) == 0 ? st.st_size : -1, 0);
    expect("created_mode_is_regular_0640", st.st_mode, S_IFREG | 0640);
    expect("write_new_file", a_write(fd, "hello", 5), 5);

    i64 fd2 = a_open(D "/f", O_RDWR | O_CREAT, 0600);
    expect("create_existing_opens_it_fd_4", fd2, 4);
    syscall2(SYS_fstat, fd2, &st);
    expect("create_existing_keeps_contents", st.st_size, 5);
    expect("create_existing_keeps_mode", st.st_mode, S_IFREG | 0640);
    a_close(fd2);

    expect("excl_on_existing_eexist", a_open(D "/f", O_RDWR | O_CREAT | O_EXCL, 0600), -EEXIST);
    i64 fd3 = a_open(D "/g", O_RDWR | O_CREAT | O_EXCL, 0600);
    expect("excl_on_new_succeeds_fd_4", fd3, 4);
    a_close(fd3);

    expect("missing_without_creat_enoent", a_open(D "/missing", O_RDONLY, 0), -ENOENT);
    expect("missing_parent_enoent", a_open(D "/nodir/x", O_RDWR | O_CREAT, 0600), -ENOENT);
    expect("parent_is_file_enotdir", a_open(D "/f/x", O_RDWR | O_CREAT, 0600), -ENOTDIR);
    expect("trailing_slash_on_file_enotdir", a_open(D "/f/", O_RDONLY, 0), -ENOTDIR);
    expect("creat_trailing_slash_eisdir", a_open(D "/new/", O_RDWR | O_CREAT, 0600), -EISDIR);

    /* The lowest free descriptor is reused. */
    a_close(fd);
    fd = a_open(D "/f", O_RDWR | O_TRUNC, 0);
    expect("reopen_reuses_fd_3", fd, 3);
    syscall2(SYS_fstat, fd, &st);
    expect("trunc_empties_file", st.st_size, 0);
    a_close(fd);

    /* O_TRUNC with O_RDONLY still truncates a regular file. */
    a_make_file(D "/f", "data", 0640);
    fd = a_open(D "/f", O_RDONLY | O_TRUNC, 0);
    expect("rdonly_trunc_opens", fd, 3);
    syscall2(SYS_fstat, fd, &st);
    expect("rdonly_trunc_truncates", st.st_size, 0);
    a_close(fd);

    expect("cleanup_unlink_f", a_unlink(D "/f"), 0);
    expect("cleanup_unlink_g", a_unlink(D "/g"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
