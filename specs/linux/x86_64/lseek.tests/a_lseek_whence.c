/* lseek: SEEK_SET, SEEK_CUR and SEEK_END, positions past the end,
 * negative results and unknown whence values. */
#include "a_sys.h"

#define F "/tmp/a_lseek_whence"

static i64 seek(i64 fd, i64 off, i64 whence) { return syscall3(SYS_lseek, fd, off, whence); }

int test_main(void) {
    expect("setup_file", a_make_file(F, "0123456789", 0644), 0);
    i64 fd = a_open(F, O_RDONLY, 0);
    expect("set_3", seek(fd, 3, SEEK_SET), 3);
    expect("cur_plus_2", seek(fd, 2, SEEK_CUR), 5);
    expect("cur_minus_5", seek(fd, -5, SEEK_CUR), 0);
    expect("cur_0_reports_offset", seek(fd, 0, SEEK_CUR), 0);
    expect("end_0", seek(fd, 0, SEEK_END), 10);
    expect("end_minus_4", seek(fd, -4, SEEK_END), 6);
    expect("end_plus_100", seek(fd, 100, SEEK_END), 110);
    /* Far past the end, within any file system's size limit. */
    expect("set_1tib", seek(fd, 1L << 40, SEEK_SET), 1L << 40);
    expect("cur_plus_1_from_1tib", seek(fd, 1, SEEK_CUR), (1L << 40) + 1);
    expect("set_negative_einval", seek(fd, -1, SEEK_SET), -EINVAL);
    expect("offset_kept_after_einval", seek(fd, 0, SEEK_CUR), (1L << 40) + 1);
    seek(fd, 2, SEEK_SET);
    expect("cur_below_zero_einval", seek(fd, -3, SEEK_CUR), -EINVAL);
    expect("end_below_zero_einval", seek(fd, -11, SEEK_END), -EINVAL);
    expect("whence_5_einval", seek(fd, 0, 5), -EINVAL);
    expect("whence_negative_einval", seek(fd, 0, -1), -EINVAL);
    expect("offset_still_2", seek(fd, 0, SEEK_CUR), 2);
    char c;
    a_read(fd, &c, 1);
    check("read_at_2", c == '2', c);
    a_close(fd);
    expect("closed_ebadf", seek(fd, 0, SEEK_SET), -EBADF);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
