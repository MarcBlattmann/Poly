/* After each advice (RANDOM, SEQUENTIAL, NOREUSE, WILLNEED, DONTNEED,
 * NORMAL) on a 64 KiB file, sequential reads, backward reads and pread
 * still return exactly the file's bytes: advice never changes data or the
 * results of reads. */
#include "a_sys.h"

#define SYS_fadvise64 221
#define D "/tmp/fadvise64_read_after"
#define SIZE (16 * 4096)

static i64 fadv(i64 fd, i64 off, i64 len, i64 advice) { return syscall4(SYS_fadvise64, fd, off, len, advice); }

static char pattern(i64 i) { return (char)('A' + (i * 7 + i / 4096) % 26); }

int test_main(void) {
    static char buf[SIZE], back[4096];
    for (i64 i = 0; i < SIZE; i++) buf[i] = pattern(i);
    a_mkdir(D, 0755);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open", fd >= 0, fd);
    expect("write", a_write(fd, buf, SIZE), SIZE);
    static const i64 order[6] = {1, 2, 5, 3, 4, 0};
    static const char *names[6] = {"random", "sequential", "noreuse", "willneed", "dontneed", "normal"};
    for (int k = 0; k < 6; k++) {
        char name[48];
        str_cat2(name, names[k], "_advice");
        expect(name, fadv(fd, 0, 0, order[k]), 0);
        int ok = 1;
        syscall3(SYS_lseek, fd, 0, SEEK_SET);
        for (i64 off = 0; off < SIZE; off += 4096) {
            ok &= a_read(fd, back, 4096) == 4096 && mem_eq(back, buf + off, 4096);
        }
        ok &= a_read(fd, back, 1) == 0;
        str_cat2(name, names[k], "_forward");
        check(name, ok, k);
        ok = 1;
        for (i64 off = SIZE - 4096; off >= 0; off -= 4096) {
            ok &= syscall4(SYS_pread64, fd, back, 4096, off) == 4096 && mem_eq(back, buf + off, 4096);
        }
        str_cat2(name, names[k], "_backward");
        check(name, ok, k);
        ok = syscall4(SYS_pread64, fd, back, 100, 4000) == 100 && mem_eq(back, buf + 4000, 100);
        str_cat2(name, names[k], "_unaligned");
        check(name, ok, k);
    }
    a_close(fd);
    a_unlink(D "/f");
    a_rmdir(D);
    return finish();
}
