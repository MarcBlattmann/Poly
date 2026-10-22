/* utime with an unreadable times pointer or path pointer: EFAULT, and the
 * file is left alone. The times are read before the path is looked up, so
 * a bad times pointer is EFAULT even for a path that does not exist. */
#include "a_sys.h"

#define SYS_utime 132
#define D "/tmp/c_utime_efault"

struct utimbuf_k {
    i64 actime, modtime;
};

int test_main(void) {
    struct a_stat before, after;
    struct utimbuf_k t = {10, 20};
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_times", syscall2(SYS_utime, D "/f", &t), 0);
    a_stat_path(D "/f", &before);
    expect("bad_times", syscall2(SYS_utime, D "/f", (void *)8), -EFAULT);
    expect("kernel_times", syscall2(SYS_utime, D "/f", (void *)0xffffffff81000000UL), -EFAULT);
    a_stat_path(D "/f", &after);
    check("bad_times_no_change", after.st_atime_sec == 10 && after.st_mtime_sec == 20, (i64)after.st_mtime_sec);
    check("bad_times_ctime_kept",
          after.st_ctime_sec == before.st_ctime_sec && after.st_ctime_nsec == before.st_ctime_nsec,
          (i64)after.st_ctime_nsec);
    expect("bad_times_missing_file", syscall2(SYS_utime, D "/nope", (void *)8), -EFAULT);
    expect("bad_times_missing_dir", syscall2(SYS_utime, D "/nodir/x", (void *)8), -EFAULT);
    expect("bad_times_empty_path", syscall2(SYS_utime, "", (void *)8), -EFAULT);
    /* times whose second field lies on an unmapped page */
    char *pg = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("setup_mmap", (i64)pg > 0, (i64)pg);
    expect("setup_munmap", syscall2(SYS_munmap, pg + A_PAGE, A_PAGE), 0);
    struct utimbuf_k *half = (struct utimbuf_k *)(pg + A_PAGE - 8);
    half->actime = 99;
    expect("times_straddle_unmapped", syscall2(SYS_utime, D "/f", half), -EFAULT);
    a_stat_path(D "/f", &after);
    check("straddle_no_change", after.st_atime_sec == 10, (i64)after.st_atime_sec);
    struct utimbuf_k *end = (struct utimbuf_k *)(pg + A_PAGE - sizeof(struct utimbuf_k));
    end->actime = 30;
    end->modtime = 40;
    expect("times_at_page_end", syscall2(SYS_utime, D "/f", end), 0);
    a_stat_path(D "/f", &after);
    check("page_end_applied", after.st_atime_sec == 30 && after.st_mtime_sec == 40, (i64)after.st_mtime_sec);
    expect("null_path", syscall2(SYS_utime, 0, &t), -EFAULT);
    expect("null_path_null_times", syscall2(SYS_utime, 0, 0), -EFAULT);
    expect("bad_path", syscall2(SYS_utime, (void *)8, &t), -EFAULT);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
