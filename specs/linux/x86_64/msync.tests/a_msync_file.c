/* msync: data written through a shared file mapping is in the file
 * (read and pread see it) after MS_SYNC, MS_ASYNC or MS_INVALIDATE;
 * writes through write() are visible in the mapping. */
#include "a_sys.h"

#define F "/tmp/a_msync_file"

static char page[A_PAGE];

int test_main(void) {
    char buf[8];
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    memset(page, '.', sizeof page);
    a_write(fd, page, sizeof page);
    i64 m = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("map_file", m > 0, m);
    if (m <= 0) return finish();
    volatile char *p = (volatile char *)m;
    p[0] = 'M';
    expect("msync_sync", syscall3(SYS_msync, m, A_PAGE, MS_SYNC), 0);
    syscall4(SYS_pread64, fd, buf, 1, 0);
    check("file_has_mapped_write", buf[0] == 'M', buf[0]);
    p[1] = 'A';
    expect("msync_async", syscall3(SYS_msync, m, A_PAGE, MS_ASYNC), 0);
    syscall4(SYS_pread64, fd, buf, 2, 0);
    check("file_has_second_write", buf[1] == 'A', buf[1]);
    expect("msync_invalidate", syscall3(SYS_msync, m, A_PAGE, MS_INVALIDATE), 0);
    expect("msync_sync_invalidate", syscall3(SYS_msync, m, A_PAGE, MS_SYNC | MS_INVALIDATE), 0);
    syscall4(SYS_pwrite64, fd, "W", 1, 2);
    check("mapping_sees_pwrite", p[2] == 'W', p[2]);
    expect("msync_flags_0", syscall3(SYS_msync, m, A_PAGE, 0), 0);
    syscall2(SYS_munmap, m, A_PAGE);
    a_close(fd);
    /* After unmapping and closing, the file keeps the data. */
    fd = a_open(F, O_RDONLY, 0);
    expect("reread", a_read(fd, buf, 3), 3);
    check("data_persisted", buf[0] == 'M' && buf[1] == 'A' && buf[2] == 'W', buf[0]);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
