/* sync() with a file that is still open but already unlinked: 0, the
 * file's data stays readable through the descriptor, its link count is
 * 0, and the name stays gone. */
#include "a_sys.h"

#define SYS_sync 162
#define F "/tmp/c_sync_unlinked_open"

int test_main(void) {
    struct a_stat st;
    char buf[8];
    a_unlink(F);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0600);
    expect("write", a_write(fd, "orphan!", 7), 7);
    expect("unlink", a_unlink(F), 0);
    expect("sync", syscall0(SYS_sync), 0);
    expect("fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("nlink", st.st_nlink, 0);
    expect("size", st.st_size, 7);
    expect("pread", syscall4(SYS_pread64, fd, buf, 7, 0), 7);
    check("data", buf[0] == 'o' && buf[6] == '!', buf[0]);
    expect("name_gone", a_stat_path(F, &st), -ENOENT);
    expect("close", a_close(fd), 0);
    expect("sync_after_close", syscall0(SYS_sync), 0);
    return finish();
}
