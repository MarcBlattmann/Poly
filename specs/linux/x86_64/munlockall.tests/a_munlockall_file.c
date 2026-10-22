/* munlockall on a locked shared file mapping: the pages become plain, the
 * data written through the mapping stays in the file. */
#include "a_sys.h"

int test_main(void) {
    const char *path = "/tmp/a_munlockall_file";
    syscall1(SYS_unlink, path);
    i64 fd = a_open(path, O_RDWR | O_CREAT | O_TRUNC, 0600);
    check("open", fd >= 0, fd);
    expect("ftruncate", syscall2(SYS_ftruncate, fd, 2 * A_PAGE), 0);
    i64 m = syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("mmap", m > 0, m);
    expect("mlockall", syscall1(SYS_mlockall, MCL_CURRENT), 0);
    ((volatile char *)m)[A_PAGE + 5] = 'q';
    expect("locked", syscall3(SYS_madvise, m, A_PAGE, MADV_DONTNEED), -EINVAL);
    expect("munlockall", syscall0(SYS_munlockall), 0);
    expect("dontneed_ok", syscall3(SYS_madvise, m, 2 * A_PAGE, MADV_DONTNEED), 0);
    check("data_kept", ((volatile char *)m)[A_PAGE + 5] == 'q', 0);
    char c = 0;
    expect("pread", syscall4(SYS_pread64, fd, &c, 1, A_PAGE + 5), 1);
    check("file_has_data", c == 'q', c);
    syscall2(SYS_munmap, m, 2 * A_PAGE);
    a_close(fd);
    expect("unlink", syscall1(SYS_unlink, path), 0);
    return finish();
}
