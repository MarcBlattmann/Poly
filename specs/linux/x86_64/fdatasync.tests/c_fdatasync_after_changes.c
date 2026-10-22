/* fdatasync after large writes, a shared-mapping store, growth and
 * truncation by ftruncate, chmod to 0 and unlink: still 0, and the file
 * shows each change. */
#include "a_sys.h"

#define F "/tmp/c_fdatasync_after_changes"

static char block[65536];

int test_main(void) {
    struct a_stat st;
    for (int i = 0; i < (int)sizeof block; i++) block[i] = (char)(i * 7);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    int ok = 1;
    for (int i = 0; i < 32; i++) ok &= a_write(fd, block, sizeof block) == sizeof block;
    check("write_2m", ok, 0);
    expect("after_writes", syscall1(SYS_fdatasync, fd), 0);
    syscall2(SYS_fstat, fd, &st);
    expect("size_2m", st.st_size, 32 * 65536);
    expect("grow", syscall2(SYS_ftruncate, fd, 3 * 1024 * 1024), 0);
    expect("after_grow", syscall1(SYS_fdatasync, fd), 0);
    expect("shrink", syscall2(SYS_ftruncate, fd, 8192), 0);
    expect("after_shrink", syscall1(SYS_fdatasync, fd), 0);
    syscall2(SYS_fstat, fd, &st);
    expect("size_8k", st.st_size, 8192);
    char *m = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("mmap", (i64)m > 0, (i64)m);
    m[5000] = 'Q';
    expect("after_mmap_store", syscall1(SYS_fdatasync, fd), 0);
    char c = 0;
    expect("pread", syscall4(SYS_pread64, fd, &c, 1, 5000), 1);
    expect("mapped_byte", c, 'Q');
    expect("munmap", syscall2(SYS_munmap, m, 8192), 0);
    expect("chmod_0", syscall2(SYS_fchmod, fd, 0), 0);
    expect("after_chmod_0", syscall1(SYS_fdatasync, fd), 0);
    expect("unlink", a_unlink(F), 0);
    expect("after_unlink", syscall1(SYS_fdatasync, fd), 0);
    syscall2(SYS_fstat, fd, &st);
    expect("nlink_0", st.st_nlink, 0);
    a_close(fd);
    return finish();
}
