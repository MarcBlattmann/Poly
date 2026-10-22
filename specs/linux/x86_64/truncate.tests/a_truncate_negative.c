/* truncate with a negative length is EINVAL whatever the path is: the
 * length is checked before the path is looked at, so a missing file, a
 * directory, a FIFO, a bad pointer, an over-long path or a non-directory
 * component all give EINVAL. Nothing changes. */
#include "a_sys.h"

#define D "/tmp/a_truncate_negative"

/* asm/unistd_64.h */
#define SYS_mknodat 259

static char longpath[4200];

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0644), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/p", S_IFIFO | 0644, 0), 0);
    expect("file_minus1", syscall2(SYS_truncate, D "/f", -1), -EINVAL);
    expect("file_min", syscall2(SYS_truncate, D "/f", (i64)0x8000000000000000UL), -EINVAL);
    a_stat_path(D "/f", &st);
    expect("size_unchanged", st.st_size, 3);
    expect("missing", syscall2(SYS_truncate, D "/none", -1), -EINVAL);
    expect("missing_dir", syscall2(SYS_truncate, D "/none/x", -1), -EINVAL);
    expect("directory", syscall2(SYS_truncate, D, -1), -EINVAL);
    expect("fifo", syscall2(SYS_truncate, D "/p", -1), -EINVAL);
    expect("not_dir_component", syscall2(SYS_truncate, D "/f/x", -1), -EINVAL);
    expect("empty_path", syscall2(SYS_truncate, "", -1), -EINVAL);
    expect("null_path", syscall2(SYS_truncate, 0, -1), -EINVAL);
    expect("bad_pointer", syscall2(SYS_truncate, 8, -1), -EINVAL);
    longpath[0] = '/';
    for (int i = 1; i < 4200 - 1; i++) longpath[i] = (i % 128) ? 'n' : '/';
    expect("too_long_path", syscall2(SYS_truncate, longpath, -2), -EINVAL);
    /* With a valid length the same paths give their own errors. */
    expect("missing_valid", syscall2(SYS_truncate, D "/none", 0), -ENOENT);
    expect("directory_valid", syscall2(SYS_truncate, D, 0), -EISDIR);
    expect("cleanup_fifo", a_unlink(D "/p"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
