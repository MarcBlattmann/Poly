/* getdents64: a buffer too small for one record is EINVAL; a buffer for
 * one record returns one at a time; reading resumes where it stopped. */
#include "a_sys.h"

#define D "/tmp/a_getdents64_small"

static char buf[512];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_a", a_make_file(D "/aaaa", "", 0644), 0);
    expect("setup_b", a_make_file(D "/bbbb", "", 0644), 0);
    expect("setup_c", a_make_file(D "/cccc", "", 0644), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("zero_size_einval", syscall3(SYS_getdents64, fd, buf, 0), -EINVAL);
    expect("size_8_einval", syscall3(SYS_getdents64, fd, buf, 8), -EINVAL);
    /* "aaaa" etc. need 24 bytes; "." and ".." also fit in 24. */
    int one_each = 1, count = 0;
    i64 n;
    while ((n = syscall3(SYS_getdents64, fd, buf, 24)) > 0) {
        one_each &= n == 24;
        count++;
    }
    expect("one_record_per_call_end", n, 0);
    check("each_call_one_record", one_each, 0);
    expect("five_records", count, 5);

    /* After a rewind, two records then the rest: together the same names
     * as one large read. (d_off values are opaque cookies; the test does
     * not pass them to lseek, so the trace does not depend on them.) */
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    i64 total = syscall3(SYS_getdents64, fd, buf, sizeof buf);
    expect("all_bytes", total, 5 * 24);
    char names[5][8];
    for (int i = 0; i < 5; i++) str_cpy(names[i], ((struct a_dirent64 *)(buf + 24 * i))->d_name);
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    expect("first_two", syscall3(SYS_getdents64, fd, buf, 48), 48);
    check("first_two_same_order", str_eq(((struct a_dirent64 *)buf)->d_name, names[0]) &&
                                      str_eq(((struct a_dirent64 *)(buf + 24))->d_name, names[1]), 0);
    n = syscall3(SYS_getdents64, fd, buf, sizeof buf);
    expect("rest_after_two", n, 3 * 24);
    check("resumes_at_third", str_eq(((struct a_dirent64 *)buf)->d_name, names[2]), 0);
    a_close(fd);

    i64 f = a_open(D "/aaaa", O_RDONLY, 0);
    expect("file_enotdir", syscall3(SYS_getdents64, f, buf, sizeof buf), -ENOTDIR);
    a_close(f);
    expect("closed_ebadf", syscall3(SYS_getdents64, f, buf, sizeof buf), -EBADF);
    fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("bad_buffer_efault", syscall3(SYS_getdents64, fd, (void *)16, sizeof buf), -EFAULT);
    a_close(fd);
    fd = a_open(D, O_PATH, 0);
    expect("path_fd_ebadf", syscall3(SYS_getdents64, fd, buf, sizeof buf), -EBADF);
    a_close(fd);
    expect("cleanup_a", a_unlink(D "/aaaa"), 0);
    expect("cleanup_b", a_unlink(D "/bbbb"), 0);
    expect("cleanup_c", a_unlink(D "/cccc"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
