/* faccessat: follows symbolic links (the raw call has no flags argument);
 * relative to a directory descriptor. */
#include "a_sys.h"

#define D "/tmp/a_faccessat_links"

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("setup_dangling", syscall3(SYS_symlinkat, "gone", AT_FDCWD, D "/d"), 0);
    expect("setup_exec", a_make_file(D "/e", "", 0755), 0);
    expect("setup_exec_link", syscall3(SYS_symlinkat, "e", AT_FDCWD, D "/el"), 0);
    expect("link_exists", syscall3(SYS_faccessat, AT_FDCWD, D "/l", F_OK), 0);
    expect("dangling_enoent", syscall3(SYS_faccessat, AT_FDCWD, D "/d", F_OK), -ENOENT);
    expect("link_takes_target_mode", syscall3(SYS_faccessat, AT_FDCWD, D "/l", X_OK), -EACCES);
    expect("link_to_exec", syscall3(SYS_faccessat, AT_FDCWD, D "/el", X_OK), 0);
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("relative", syscall3(SYS_faccessat, dfd, "f", R_OK), 0);
    expect("relative_missing", syscall3(SYS_faccessat, dfd, "nope", F_OK), -ENOENT);
    expect("bad_dirfd_ebadf", syscall3(SYS_faccessat, 99, "f", F_OK), -EBADF);
    i64 ffd = a_open(D "/f", O_RDONLY, 0);
    expect("file_dirfd_enotdir", syscall3(SYS_faccessat, ffd, "x", F_OK), -ENOTDIR);
    a_close(ffd);
    a_close(dfd);
    const char *names[] = {D "/f", D "/l", D "/d", D "/e", D "/el"};
    int ok = 1;
    for (int i = 0; i < 5; i++) ok &= a_unlink(names[i]) == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
