/* sync() with a dirty MAP_SHARED file mapping: 0; the mapping stays in
 * place and writable, read() of the file sees what was stored through the
 * mapping, the size does not change. */
#include "a_sys.h"

#define SYS_sync 162
#define F "/tmp/c_sync_mmap_shared"

int test_main(void) {
    struct a_stat st;
    char buf[8];
    a_unlink(F);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("ftruncate", syscall2(SYS_ftruncate, fd, 8192), 0);
    char *p = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("mmap", (i64)p > 0, (i64)p);
    if ((i64)p <= 0) return finish();
    p[0] = 'A';
    p[4096] = 'B';
    p[8191] = 'C';
    expect("sync", syscall0(SYS_sync), 0);
    expect("pread0", syscall4(SYS_pread64, fd, buf, 1, 0), 1);
    expect("byte0", buf[0], 'A');
    expect("pread1", syscall4(SYS_pread64, fd, buf, 1, 4096), 1);
    expect("byte4096", buf[0], 'B');
    expect("pread2", syscall4(SYS_pread64, fd, buf, 1, 8191), 1);
    expect("byte8191", buf[0], 'C');
    p[1] = 'Z';
    expect("sync_again", syscall0(SYS_sync), 0);
    expect("pread3", syscall4(SYS_pread64, fd, buf, 2, 0), 2);
    check("after_second_store", buf[0] == 'A' && buf[1] == 'Z', buf[1]);
    expect("fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("size", st.st_size, 8192);
    expect("munmap", syscall2(SYS_munmap, (i64)p, 8192), 0);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
