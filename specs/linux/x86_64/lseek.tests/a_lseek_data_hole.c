/* lseek: SEEK_DATA and SEEK_HOLE on a file without holes, and past its
 * end (ENXIO). */
#include "a_sys.h"

#define F "/tmp/a_lseek_data_hole"

static i64 seek(i64 fd, i64 off, i64 whence) { return syscall3(SYS_lseek, fd, off, whence); }

int test_main(void) {
    expect("setup_file", a_make_file(F, "0123456789", 0644), 0);
    i64 fd = a_open(F, O_RDONLY, 0);
    expect("data_at_0", seek(fd, 0, SEEK_DATA), 0);
    expect("data_at_4", seek(fd, 4, SEEK_DATA), 4);
    expect("hole_at_0_is_end", seek(fd, 0, SEEK_HOLE), 10);
    expect("offset_moved_to_end", seek(fd, 0, SEEK_CUR), 10);
    expect("hole_at_9_is_end", seek(fd, 9, SEEK_HOLE), 10);
    expect("data_at_end_enxio", seek(fd, 10, SEEK_DATA), -ENXIO);
    expect("hole_at_end_enxio", seek(fd, 10, SEEK_HOLE), -ENXIO);
    expect("data_past_end_enxio", seek(fd, 50, SEEK_DATA), -ENXIO);
    expect("data_negative_enxio", seek(fd, -1, SEEK_DATA), -ENXIO);
    expect("hole_negative_enxio", seek(fd, -1, SEEK_HOLE), -ENXIO);
    a_close(fd);
    /* An empty file has neither data nor holes. */
    fd = a_open(F, O_RDWR | O_TRUNC, 0);
    expect("empty_data_enxio", seek(fd, 0, SEEK_DATA), -ENXIO);
    expect("empty_hole_enxio", seek(fd, 0, SEEK_HOLE), -ENXIO);
    expect("empty_end_0", seek(fd, 0, SEEK_END), 0);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
