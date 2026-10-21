/* access path handling: an empty path is ENOENT; a NULL or unmapped pointer
 * is EFAULT; a file followed by "/" or more components is ENOTDIR; a name of
 * 256 bytes is ENAMETOOLONG, 255 is fine; a path of 4096 bytes with its NUL
 * (4095 before it) is looked up, one byte more is ENAMETOOLONG; "." and ".."
 * work; repeated slashes are one. */
#include "a_sys.h"

#define D "/tmp/a_access_paths"

static char buf[8192];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("empty_enoent", syscall2(SYS_access, "", F_OK), -ENOENT);
    expect("null_efault", syscall2(SYS_access, 0, F_OK), -EFAULT);
    expect("bad_ptr_efault", syscall2(SYS_access, 1, F_OK), -EFAULT);
    expect("kernel_ptr_efault", syscall2(SYS_access, 0xffff800000000000L, F_OK), -EFAULT);
    expect("file_slash_enotdir", syscall2(SYS_access, D "/f/", F_OK), -ENOTDIR);
    expect("file_component_enotdir", syscall2(SYS_access, D "/f/x", F_OK), -ENOTDIR);
    expect("file_dot_enotdir", syscall2(SYS_access, D "/f/.", F_OK), -ENOTDIR);
    expect("dir_slash", syscall2(SYS_access, D "/", F_OK), 0);
    expect("dot", syscall2(SYS_access, D "/.", R_OK), 0);
    expect("dotdot", syscall2(SYS_access, D "/../a_access_paths/f", R_OK), 0);
    expect("double_slash", syscall2(SYS_access, "/tmp//a_access_paths///f", R_OK), 0);
    expect("root_dotdot", syscall2(SYS_access, "/../../tmp", X_OK), 0);

    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 'a';
    p[256] = 0;
    expect("name_256_enametoolong", syscall2(SYS_access, buf, F_OK), -ENAMETOOLONG);
    p[255] = 0;
    expect("name_255_enoent", syscall2(SYS_access, buf, F_OK), -ENOENT);

    /* "/" repeated: 4096 bytes plus NUL is too long, 4095 is looked up. */
    for (int i = 0; i < 4096; i++) buf[i] = '/';
    buf[4096] = 0;
    expect("len_4096_enametoolong", syscall2(SYS_access, buf, F_OK), -ENAMETOOLONG);
    buf[4095] = 0;
    expect("len_4095_ok", syscall2(SYS_access, buf, F_OK), 0);
    /* The same limit with real components: /tmp/./././... */
    char *q = str_cpy(buf, "/tmp");
    for (int i = 0; i < 2100; i++) { q[2 * i] = '/'; q[2 * i + 1] = '.'; }
    buf[4095] = 0;
    expect("dots_4095_ok", syscall2(SYS_access, buf, X_OK), 0);
    buf[4095] = '.';
    buf[4096] = 0;
    expect("dots_4096_enametoolong", syscall2(SYS_access, buf, X_OK), -ENAMETOOLONG);
    for (int i = 0; i < 8000; i++) buf[i] = '/';
    buf[8000] = 0;
    expect("len_8000_enametoolong", syscall2(SYS_access, buf, F_OK), -ENAMETOOLONG);

    expect("cleanup_unlink", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
