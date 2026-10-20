/* ftruncate: other descriptors and memory maps of the same file see the
 * new size; mtime and ctime change. */
#include "a_sys.h"

#define F "/tmp/a_ftruncate_shared"

int test_main(void) {
    struct a_stat a, b;
    char buf[16];
    i64 w = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    i64 r = a_open(F, O_RDONLY, 0);
    a_write(w, "hello world", 11);
    struct a_timespec old[2] = {{100, 0}, {100, 0}};
    syscall4(SYS_utimensat, w, 0, old, 0);
    syscall2(SYS_fstat, w, &a);
    expect("truncate", syscall2(SYS_ftruncate, w, 5), 0);
    syscall2(SYS_fstat, r, &b);
    expect("other_fd_sees_size", b.st_size, 5);
    check("mtime_changed", b.st_mtime_sec != 100, b.st_mtime_sec);
    check("ctime_not_earlier", b.st_ctime_sec >= a.st_ctime_sec, 0);
    expect("other_fd_reads_5", a_read(r, buf, sizeof buf), 5);
    /* A shared map of a grown file sees zeros in the new part. */
    expect("grow_to_page", syscall2(SYS_ftruncate, w, A_PAGE), 0);
    i64 m = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_SHARED, r, 0);
    check("mmap_ok", m > 0, m);
    if (m > 0) {
        char *p = (char *)m;
        check("map_old_data", mem_eq(p, "hello", 5), p[0]);
        check("map_new_zeros", p[5] == 0 && p[A_PAGE - 1] == 0, p[5]);
        syscall2(SYS_munmap, m, A_PAGE);
    }
    /* A truncate that does not change the size still updates mtime. */
    syscall4(SYS_utimensat, w, 0, old, 0);
    expect("same_size_truncate", syscall2(SYS_ftruncate, w, A_PAGE), 0);
    syscall2(SYS_fstat, w, &a);
    check("same_size_mtime_changed", a.st_mtime_sec != 100, a.st_mtime_sec);
    a_close(r);
    a_close(w);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
