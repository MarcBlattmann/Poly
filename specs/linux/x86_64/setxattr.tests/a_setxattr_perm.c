/* setxattr permission checks, with all capabilities dropped first (so root
 * is checked like anyone else): user.* needs write permission on the
 * file, by its mode, not by how a descriptor was opened (EACCES on a 0444
 * file through a path and an O_RDONLY descriptor; a 0200 file works); a
 * 0555 directory is EACCES; a directory without search permission on the
 * way is EACCES. EACCES comes before EEXIST/ENODATA, before EINVAL for
 * "user." and before EOPNOTSUPP for an unknown prefix; unknown system.*
 * names are EOPNOTSUPP even then. The owner may set attributes on its own
 * sticky directory. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_fsetxattr 190
#define SYS_getxattr 191
#define ENODATA 61
#define D "/tmp/a_setxattr_perm"

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 set(const char *p, const char *name, i64 flags) {
    return syscall5(SYS_setxattr, p, name, "v", 1, flags);
}

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0444), 0);
    expect("setup_wo", a_make_file(D "/wo", "", 0200), 0);
    expect("setup_rodir", a_mkdir(D "/rodir", 0555), 0);
    expect("setup_nox", a_mkdir(D "/nox", 0755), 0);
    expect("setup_nox_f", a_make_file(D "/nox/f", "", 0644), 0);
    expect("setup_nox_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/nox", 0600), 0);
    expect("setup_sticky", a_mkdir(D "/sticky", 0755), 0);
    expect("setup_sticky_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/sticky", 01777), 0);
    expect("ro_file_eacces", set(D "/ro", "user.a", 0), -EACCES);
    i64 fd = a_open(D "/ro", O_RDONLY, 0);
    expect("ro_file_fd_eacces", syscall5(SYS_fsetxattr, fd, "user.a", "v", 1, 0), -EACCES);
    a_close(fd);
    expect("ro_not_created", syscall4(SYS_getxattr, D "/ro", "user.a", buf, sizeof buf), -ENODATA);
    expect("ro_replace_eacces_not_enodata", set(D "/ro", "user.a", 2), -EACCES);
    expect("ro_user_dot_eacces", set(D "/ro", "user.", 0), -EACCES);
    expect("ro_unknown_ns_eacces", set(D "/ro", "foo.a", 0), -EACCES);
    expect("ro_no_dot_eacces", set(D "/ro", "user", 0), -EACCES);
    expect("ro_system_eopnotsupp", set(D "/ro", "system.x", 0), -EOPNOTSUPP);
    expect("wo_file_ok", set(D "/wo", "user.a", 0), 0);
    expect("wo_create_existing_eexist", set(D "/wo", "user.a", 1), -EEXIST);
    expect("setup_wo_ro", syscall3(SYS_fchmodat, AT_FDCWD, D "/wo", 0444), 0);
    expect("ro_create_existing_eacces", set(D "/wo", "user.a", 1), -EACCES);
    expect("ro_dir_eacces", set(D "/rodir", "user.a", 0), -EACCES);
    expect("no_search_eacces", set(D "/nox/f", "user.a", 0), -EACCES);
    expect("own_sticky_dir_ok", set(D "/sticky", "user.a", 0), 0);
    expect("cleanup_nox_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/nox", 0755), 0);
    expect("cleanup_nox_f", a_unlink(D "/nox/f"), 0);
    a_rmdir(D "/nox");
    a_rmdir(D "/rodir");
    a_rmdir(D "/sticky");
    a_unlink(D "/ro");
    a_unlink(D "/wo");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
