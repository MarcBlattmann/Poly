/* futimesat(fd, NULL, times) sets the times of the file fd refers to
 * (a regular file opened read-only, or a directory), like futimes. With
 * AT_FDCWD and a NULL path it is EFAULT; a bad descriptor EBADF. An
 * empty path "" is ENOENT (there is no AT_EMPTY_PATH).
 */
#include "a_sys.h"

#define SYS_futimesat 261
#define DIR "/tmp/ld_futimesat_null_path"

static struct a_stat st;
static struct a_timeval tv[2];

static i64 fut(i64 d, const char *p, i64 as, i64 ms) {
    tv[0].tv_sec = as;
    tv[0].tv_usec = 250;
    tv[1].tv_sec = ms;
    tv[1].tv_usec = 750;
    return syscall3(SYS_futimesat, d, p, tv);
}

static void cleanup(void) {
    a_unlink(DIR "/f");
    a_rmdir(DIR "/sub");
    a_rmdir(DIR);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    a_mkdir(DIR "/sub", 0755);
    expect("file", a_make_file(DIR "/f", "x", 0644), 0);

    i64 f = a_open(DIR "/f", O_RDONLY, 0);
    expect("rdonly_fd_3", f, 3);
    expect("null_path_file_fd", fut(f, 0, 111, 222), 0);
    syscall2(SYS_fstat, f, &st);
    expect("file_atime", st.st_atime_sec, 111);
    expect("file_atime_nsec", st.st_atime_nsec, 250000);
    expect("file_mtime", st.st_mtime_sec, 222);
    expect("file_mtime_nsec", st.st_mtime_nsec, 750000);

    i64 d = a_open(DIR "/sub", O_RDONLY | O_DIRECTORY, 0);
    expect("dir_fd_4", d, 4);
    expect("null_path_dir_fd", fut(d, 0, 333, 444), 0);
    a_stat_path(DIR "/sub", &st);
    expect("dir_atime", st.st_atime_sec, 333);
    expect("dir_mtime", st.st_mtime_sec, 444);

    expect("null_path_null_times", syscall3(SYS_futimesat, f, 0, 0), 0);
    syscall2(SYS_fstat, f, &st);
    check("null_times_now", st.st_mtime_sec > 1000000000, 0);

    expect("null_path_at_fdcwd_efault", fut(AT_FDCWD, 0, 1, 2), -EFAULT);
    expect("null_path_bad_fd_ebadf", fut(99, 0, 1, 2), -EBADF);
    expect("null_path_closed_fd_ebadf", fut(7, 0, 1, 2), -EBADF);
    expect("empty_path_enoent", fut(f, "", 1, 2), -ENOENT);
    expect("empty_path_dir_enoent", fut(d, "", 1, 2), -ENOENT);
    a_stat_path(DIR "/sub", &st);
    expect("dir_unchanged", st.st_mtime_sec, 444);

    a_close(f);
    a_close(d);
    cleanup();
    return finish();
}
