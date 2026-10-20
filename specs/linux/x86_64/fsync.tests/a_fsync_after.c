/* fsync after many writes, truncation and unlink: still 0, and the file
 * is unchanged by the call. */
#include "a_sys.h"

#define F "/tmp/a_fsync_after"

static char block[8192];

int test_main(void) {
    struct a_stat st;
    for (int i = 0; i < 8192; i++) block[i] = (char)i;
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    int ok = 1;
    for (int i = 0; i < 16; i++) ok &= a_write(fd, block, sizeof block) == sizeof block;
    check("write_128k", ok, 0);
    expect("fsync_after_writes", syscall1(SYS_fsync, fd), 0);
    syscall2(SYS_fstat, fd, &st);
    expect("size_kept", st.st_size, 16 * 8192);
    expect("truncate", syscall2(SYS_ftruncate, fd, 100), 0);
    expect("fdatasync_after_truncate", syscall1(SYS_fdatasync, fd), 0);
    syscall2(SYS_fstat, fd, &st);
    expect("truncated_size_kept", st.st_size, 100);
    expect("unlink_while_open", a_unlink(F), 0);
    expect("fsync_unlinked", syscall1(SYS_fsync, fd), 0);
    expect("offset_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 16 * 8192);
    a_close(fd);
    return finish();
}
