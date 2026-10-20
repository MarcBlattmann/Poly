/* umask: returns the previous mask; only the permission bits (0777) are
 * kept; the mask applies to new files and directories. */
#include "a_sys.h"

#define D "/tmp/a_umask_basic"

static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)(st.st_mode & 07777);
}

int test_main(void) {
    i64 initial = syscall1(SYS_umask, 022);
    expect("returns_previous", syscall1(SYS_umask, 077), 022);
    expect("returns_077", syscall1(SYS_umask, 0), 077);
    expect("returns_0", syscall1(SYS_umask, 0777), 0);
    expect("returns_0777", syscall1(SYS_umask, 07777), 0777);
    expect("high_bits_dropped", syscall1(SYS_umask, 022), 0777);
    expect("only_low_bits_kept", syscall1(SYS_umask, 0xfffff000 | 022), 022);
    expect("after_garbage_022", syscall1(SYS_umask, 022), 022);
    expect("setup_mkdir", a_mkdir(D, 0777), 0);
    expect("dir_0755", mode_of(D), 0755);
    syscall1(SYS_umask, 027);
    expect("file_0640", a_make_file(D "/f", "", 0666) == 0 ? mode_of(D "/f") : -1, 0640);
    expect("dir_0750", a_mkdir(D "/d", 0777) == 0 ? mode_of(D "/d") : -1, 0750);
    syscall1(SYS_umask, 0777);
    expect("file_all_masked", a_make_file(D "/g", "", 0666) == 0 ? mode_of(D "/g") : -1, 0);
    syscall1(SYS_umask, 0);
    expect("file_unmasked", a_make_file(D "/h", "", 0666) == 0 ? mode_of(D "/h") : -1, 0666);
    /* O_CREAT on an existing file ignores the mode and the mask. */
    syscall1(SYS_umask, 0777);
    i64 fd = a_open(D "/h", O_RDWR | O_CREAT, 0);
    a_close(fd);
    expect("existing_file_mode_kept", mode_of(D "/h"), 0666);
    syscall1(SYS_umask, initial);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_h", a_unlink(D "/h"), 0);
    expect("cleanup_d", a_rmdir(D "/d"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
