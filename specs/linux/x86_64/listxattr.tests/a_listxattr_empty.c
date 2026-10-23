/* listxattr: a fresh file, directory and FIFO have no attributes a normal
 * user can see, so the list is empty: 0 for any size, and the buffer is not
 * used at all (NULL or an unmapped pointer with a nonzero size is fine). */
#include "a_sys.h"

#define SYS_mknodat 259
#define SYS_listxattr 194
#define D "/tmp/a_listxattr_empty"
#define BAD ((void *)8)

static char buf[64];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/p", 0010644, 0), 0);
    for (int i = 0; i < 64; i++) buf[i] = '#';
    expect("file_empty", syscall3(SYS_listxattr, D "/f", buf, sizeof buf), 0);
    check("file_buf_untouched", buf[0] == '#' && buf[63] == '#', buf[0]);
    expect("file_size0", syscall3(SYS_listxattr, D "/f", 0, 0), 0);
    expect("file_null_size", syscall3(SYS_listxattr, D "/f", 0, 100), 0);
    expect("file_bad_ptr", syscall3(SYS_listxattr, D "/f", BAD, 100), 0);
    expect("file_size1", syscall3(SYS_listxattr, D "/f", buf, 1), 0);
    expect("dir_empty", syscall3(SYS_listxattr, D, buf, sizeof buf), 0);
    expect("dir_size0", syscall3(SYS_listxattr, D, 0, 0), 0);
    expect("fifo_empty", syscall3(SYS_listxattr, D "/p", buf, sizeof buf), 0);
    expect("fifo_bad_ptr", syscall3(SYS_listxattr, D "/p", BAD, 100), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_p", a_unlink(D "/p"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
