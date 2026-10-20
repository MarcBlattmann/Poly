/* close: closing one of two descriptors for the same open file leaves the
 * other working; an unlinked file stays readable until its last close. */
#include "a_sys.h"

#define F "/tmp/a_close_shared"

int test_main(void) {
    char buf[8];
    expect("setup_file", a_make_file(F, "abcdef", 0644), 0);
    i64 fd = a_open(F, O_RDONLY, 0);
    i64 copy = syscall1(SYS_dup, fd);
    expect("dup_is_4", copy, 4);
    expect("read_two", a_read(fd, buf, 2), 2);
    expect("close_original", a_close(fd), 0);
    expect("copy_still_reads", a_read(copy, buf, 2), 2);
    check("copy_shares_offset", buf[0] == 'c' && buf[1] == 'd', buf[0]);
    expect("close_copy", a_close(copy), 0);
    expect("copy_closed_ebadf", a_read(copy, buf, 1), -EBADF);

    fd = a_open(F, O_RDONLY, 0);
    expect("unlink_while_open", a_unlink(F), 0);
    expect("name_is_gone", a_open(F, O_RDONLY, 0), -ENOENT);
    expect("open_file_still_reads", a_read(fd, buf, 6), 6);
    check("unlinked_contents", mem_eq(buf, "abcdef", 6), 0);
    struct a_stat st;
    syscall2(SYS_fstat, fd, &st);
    expect("unlinked_nlink_zero", st.st_nlink, 0);
    expect("close_unlinked", a_close(fd), 0);

    /* Directory and O_PATH descriptors close like any other. */
    fd = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    expect("close_directory", a_close(fd), 0);
    fd = a_open("/tmp", O_PATH, 0);
    expect("close_path_fd", a_close(fd), 0);
    return finish();
}
