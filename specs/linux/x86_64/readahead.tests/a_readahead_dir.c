/* A directory opened for reading is EINVAL, whatever the range. */
#include "a_sys.h"

#define SYS_readahead 187

static i64 ra(i64 fd, i64 off, i64 count) { return syscall3(SYS_readahead, fd, off, count); }

#define D "/tmp/a_readahead_dir"

int test_main(void) {
    a_rmdir(D);
    expect("mkdir", a_mkdir(D, 0755), 0);
    i64 d = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("opened", d >= 0, d);
    expect("dir", ra(d, 0, 4096), -EINVAL);
    expect("dir_count_0", ra(d, 0, 0), -EINVAL);
    expect("dir_neg_offset", ra(d, -1, 1), -EINVAL);
    expect("dir_bad_count", ra(d, 0, -1), -EINVAL);
    expect("dir_offset_unchanged", syscall3(SYS_lseek, d, 0, SEEK_CUR), 0);
    i64 root = a_open("/", O_RDONLY | O_DIRECTORY, 0);
    expect("root_dir", ra(root, 0, 4096), -EINVAL);
    i64 tmp = a_open("/tmp", O_RDONLY, 0);
    expect("tmp_dir", ra(tmp, 0, 1), -EINVAL);
    a_close(root);
    a_close(tmp);
    a_close(d);
    expect("cleanup", a_rmdir(D), 0);
    return finish();
}
