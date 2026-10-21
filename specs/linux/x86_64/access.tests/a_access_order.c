/* access: which error wins when a path has several problems: the mode first
 * (EINVAL), then the path string (EFAULT, ENAMETOOLONG for 4096+ bytes,
 * ENOENT for empty), then the components left to right as the walk reaches
 * them; the permission bits of the final name are checked last. */
#include "a_sys.h"

#define D "/tmp/a_access_order"

static char buf[8192];

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_ns", a_mkdir(D "/ns", 0755), 0);
    expect("chmod_ns", syscall2(SYS_chmod, D "/ns", 0600), 0);

    char *p = str_cpy(buf, D "/nope/");
    for (int i = 0; i < 256; i++) p[i] = 'b';
    p[256] = 0;
    expect("missing_then_long_enoent", syscall2(SYS_access, buf, F_OK), -ENOENT);
    p = str_cpy(buf, D "/f/");
    for (int i = 0; i < 256; i++) p[i] = 'b';
    p[256] = 0;
    expect("file_then_long_enotdir", syscall2(SYS_access, buf, F_OK), -ENOTDIR);
    p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 'b';
    str_cpy(p + 256, "/nope");
    expect("long_then_missing_enametoolong", syscall2(SYS_access, buf, F_OK), -ENAMETOOLONG);
    expect("missing_then_file", syscall2(SYS_access, D "/nope/../f", F_OK), -ENOENT);
    expect("file_then_missing", syscall2(SYS_access, D "/f/nope", F_OK), -ENOTDIR);
    expect("unsearchable_then_long", syscall2(SYS_access, D "/ns/nope/x", F_OK), root ? -ENOENT : -EACCES);
    p = str_cpy(buf, D "/ns/");
    for (int i = 0; i < 256; i++) p[i] = 'b';
    p[256] = 0;
    expect("unsearchable_long_name", syscall2(SYS_access, buf, F_OK), root ? -ENAMETOOLONG : -EACCES);
    expect("bad_mode_on_unsearchable", syscall2(SYS_access, D "/ns/x", 8), -EINVAL);
    expect("missing_with_x", syscall2(SYS_access, D "/nope", X_OK), -ENOENT);
    expect("enotdir_with_x", syscall2(SYS_access, D "/f/", X_OK), -ENOTDIR);

    for (int i = 0; i < 5000; i++) buf[i] = 'c';
    buf[0] = '/';
    buf[5000] = 0;
    expect("too_long_beats_missing", syscall2(SYS_access, buf, F_OK), -ENAMETOOLONG);

    syscall2(SYS_chmod, D "/ns", 0755);
    expect("cleanup_ns", a_rmdir(D "/ns"), 0);
    expect("cleanup_unlink", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
