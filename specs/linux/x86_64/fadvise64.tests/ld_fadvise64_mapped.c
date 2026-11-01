/* fadvise64 is only advice: POSIX_FADV_DONTNEED on a file (on /tmp and
 * on the tmpfs /dev/shm) while a MAP_SHARED mapping of it exists, and
 * after writes through the mapping and through write(2), changes no byte
 * the program can see, neither through the mapping nor through pread.
 * WILLNEED on a range that is then read gives the file's bytes.
 */
#include "a_sys.h"

#define SYS_fadvise64 221
#define POSIX_FADV_WILLNEED 3
#define POSIX_FADV_DONTNEED 4

static void run(const char *tag, const char *path) {
    char name[64];
    i64 fd = a_open(path, O_RDWR | O_CREAT | O_TRUNC, 0600);
    expect(str_cat2(name, tag, "_open"), fd >= 0, 1);
    expect(str_cat2(name, tag, "_truncate"), syscall2(SYS_ftruncate, fd, 2 * A_PAGE), 0);
    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check(str_cat2(name, tag, "_mmap"), (i64)m > 0, (i64)m);
    m[0] = 'M';
    m[A_PAGE + 1] = 'N';
    expect(str_cat2(name, tag, "_pwrite"), syscall4(SYS_pwrite64, fd, "W", 1, 10), 1);
    expect(str_cat2(name, tag, "_dontneed"), syscall4(SYS_fadvise64, fd, 0, 0, POSIX_FADV_DONTNEED), 0);
    check(str_cat2(name, tag, "_map_keeps_M"), m[0] == 'M', m[0]);
    check(str_cat2(name, tag, "_map_keeps_N"), m[A_PAGE + 1] == 'N', m[A_PAGE + 1]);
    check(str_cat2(name, tag, "_map_sees_W"), m[10] == 'W', m[10]);
    char b[2] = {0, 0};
    expect(str_cat2(name, tag, "_willneed"), syscall4(SYS_fadvise64, fd, A_PAGE, A_PAGE, POSIX_FADV_WILLNEED), 0);
    expect(str_cat2(name, tag, "_pread"), syscall4(SYS_pread64, fd, b, 2, A_PAGE), 2);
    check(str_cat2(name, tag, "_pread_N"), b[0] == 0 && b[1] == 'N', b[1]);
    expect(str_cat2(name, tag, "_pread0"), syscall4(SYS_pread64, fd, b, 1, 0), 1);
    check(str_cat2(name, tag, "_pread_M"), b[0] == 'M', b[0]);
    syscall2(SYS_munmap, m, 2 * A_PAGE);
    expect(str_cat2(name, tag, "_dontneed_unmapped"), syscall4(SYS_fadvise64, fd, 0, 0, POSIX_FADV_DONTNEED), 0);
    expect(str_cat2(name, tag, "_pread_after"), syscall4(SYS_pread64, fd, b, 1, 10), 1);
    check(str_cat2(name, tag, "_still_W"), b[0] == 'W', b[0]);
    a_close(fd);
    expect(str_cat2(name, tag, "_unlink"), a_unlink(path), 0);
}

int test_main(void) {
    run("tmp", "/tmp/ld_fadvise64_mapped");
    run("shm", "/dev/shm/ld_fadvise64_mapped");
    return finish();
}
