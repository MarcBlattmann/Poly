/* utimes with an unreadable times pointer (32 bytes: two timevals) or path
 * pointer: EFAULT, and the file is left alone. */
#include "a_sys.h"

#define D "/tmp/c_utimes_efault"

struct timeval_k {
    i64 tv_sec, tv_usec;
};

int test_main(void) {
    struct a_stat before, after;
    struct timeval_k t[2] = {{10, 0}, {20, 0}};
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_times", syscall2(SYS_utimes, D "/f", t), 0);
    a_stat_path(D "/f", &before);
    expect("bad_times", syscall2(SYS_utimes, D "/f", (void *)8), -EFAULT);
    expect("kernel_times", syscall2(SYS_utimes, D "/f", (void *)0xffffffff81000000UL), -EFAULT);
    a_stat_path(D "/f", &after);
    check("bad_times_no_change", after.st_atime_sec == 10 && after.st_mtime_sec == 20, (i64)after.st_mtime_sec);
    check("bad_times_ctime_kept",
          after.st_ctime_sec == before.st_ctime_sec && after.st_ctime_nsec == before.st_ctime_nsec,
          (i64)after.st_ctime_nsec);
    expect("bad_times_empty_path", syscall2(SYS_utimes, "", (void *)8), -EFAULT);
    char *pg = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("setup_mmap", (i64)pg > 0, (i64)pg);
    expect("setup_munmap", syscall2(SYS_munmap, pg + A_PAGE, A_PAGE), 0);
    /* The second timeval lies on the unmapped page. */
    struct timeval_k *half = (struct timeval_k *)(pg + A_PAGE - sizeof(struct timeval_k));
    half->tv_sec = 99;
    half->tv_usec = 0;
    expect("mtime_on_unmapped_page", syscall2(SYS_utimes, D "/f", half), -EFAULT);
    /* Only the last 8 bytes (mtime's usec) are unmapped. */
    struct timeval_k *last = (struct timeval_k *)(pg + A_PAGE - 24);
    last[0].tv_sec = 99;
    last[0].tv_usec = 0;
    last[1].tv_sec = 99;
    expect("mtime_usec_on_unmapped_page", syscall2(SYS_utimes, D "/f", last), -EFAULT);
    a_stat_path(D "/f", &after);
    check("straddle_no_change", after.st_atime_sec == 10 && after.st_mtime_sec == 20, (i64)after.st_atime_sec);
    struct timeval_k *end = (struct timeval_k *)(pg + A_PAGE - 2 * sizeof(struct timeval_k));
    end[0].tv_sec = 30;
    end[0].tv_usec = 1;
    end[1].tv_sec = 40;
    end[1].tv_usec = 2;
    expect("times_at_page_end", syscall2(SYS_utimes, D "/f", end), 0);
    a_stat_path(D "/f", &after);
    check("page_end_applied", after.st_atime_sec == 30 && after.st_atime_nsec == 1000 && after.st_mtime_sec == 40 &&
                                  after.st_mtime_nsec == 2000,
          (i64)after.st_mtime_sec);
    expect("null_path", syscall2(SYS_utimes, 0, t), -EFAULT);
    expect("null_path_null_times", syscall2(SYS_utimes, 0, 0), -EFAULT);
    expect("bad_path", syscall2(SYS_utimes, (void *)8, t), -EFAULT);
    expect("bad_path_null_times", syscall2(SYS_utimes, (void *)8, 0), -EFAULT);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
