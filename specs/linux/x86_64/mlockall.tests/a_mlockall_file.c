/* mlockall: MCL_FUTURE populates a new shared file mapping; locked pages
 * refuse MADV_DONTNEED (EINVAL) until they are unlocked. */
#include "a_sys.h"

static i64 resident(i64 addr, i64 pages) {
    unsigned char v[16];
    i64 r = syscall3(SYS_mincore, addr, pages * A_PAGE, v);
    if (r) return r;
    i64 n = 0;
    for (i64 i = 0; i < pages; i++) n += v[i] & 1;
    return n;
}

int test_main(void) {
    const char *path = "/tmp/a_mlockall_file";
    syscall1(SYS_unlink, path);
    i64 fd = a_open(path, O_RDWR | O_CREAT | O_TRUNC, 0600);
    check("open", fd >= 0, fd);
    expect("ftruncate", syscall2(SYS_ftruncate, fd, 4 * A_PAGE), 0);
    expect("mlockall", syscall1(SYS_mlockall, MCL_CURRENT | MCL_FUTURE), 0);
    i64 m = syscall6(SYS_mmap, 0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("mmap", m > 0, m);
    expect("file_mapping_resident", resident(m, 4), 4);
    check("reads_zero", ((volatile char *)m)[3 * A_PAGE] == 0, 0);
    i64 a = syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("dontneed_locked_einval", syscall3(SYS_madvise, a, A_PAGE, MADV_DONTNEED), -EINVAL);
    expect("munlock", syscall2(SYS_munlock, a, 2 * A_PAGE), 0);
    expect("dontneed_unlocked_ok", syscall3(SYS_madvise, a, A_PAGE, MADV_DONTNEED), 0);
    syscall2(SYS_munmap, m, 4 * A_PAGE);
    a_close(fd);
    expect("unlink", syscall1(SYS_unlink, path), 0);
    return finish();
}
