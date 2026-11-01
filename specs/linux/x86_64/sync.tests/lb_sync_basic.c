/* sync(2) commits all file system data to storage. It takes no
 * arguments (anything in the registers is ignored), always returns 0, and
 * changes nothing visible: contents, sizes and times stay. */
#include "a_sys.h"

#define D "/tmp/lb_sync_basic"
#define SYS_sync 162

int test_main(void) {
    struct a_stat st, st2;
    char c[16];
    expect("plain", syscall0(SYS_sync), 0);
    expect("garbage_args", syscall6(SYS_sync, -1, 1, 2, 3, 4, 5), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT, 0644);
    expect("write", a_write(fd, "synced", 6), 6);
    expect("stat0", a_stat_path(D "/f", &st), 0);
    expect("after_write", syscall0(SYS_sync), 0);
    expect("stat1", a_stat_path(D "/f", &st2), 0);
    expect("stat_identical", mem_eq(&st, &st2, sizeof st), 1);
    expect("offset_kept", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 6);
    expect("contents", syscall4(SYS_pread64, fd, c, 16, 0) == 6 && mem_eq(c, "synced", 6), 1);
    a_close(fd);
    for (int i = 0; i < 5; i++) expect("repeated", syscall0(SYS_sync), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    expect("after_cleanup", syscall0(SYS_sync), 0);
    return finish();
}
