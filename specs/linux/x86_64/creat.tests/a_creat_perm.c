/* creat(2) and permissions: write permission on an existing file, write
 * permission on the directory for a new name, search permission on every
 * directory on the way. As root the checks pass; as any other user they
 * fail with EACCES and nothing is created or truncated. */
#include "a_sys.h"

#define SYS_creat 85
#define D "/tmp/a_creat_perm"

static i64 cr(const char *p, i64 mode) { return syscall2(SYS_creat, p, mode); }

static int root;

static void denied(const char *name, i64 got) {
    check(name, root ? got >= 0 : got == -EACCES, got);
    if (got >= 0) a_close(got);
}

int test_main(void) {
    root = syscall0(SYS_geteuid) == 0;
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_ro_file", a_make_file(D "/ro", "data", 0444), 0);
    expect("setup_wo_file", a_make_file(D "/wo", "data", 0200), 0);
    expect("setup_rodir", a_mkdir(D "/rodir", 0700), 0);
    expect("setup_rodir_file", a_make_file(D "/rodir/f", "data", 0600), 0);
    expect("setup_noxdir", a_mkdir(D "/noxdir", 0700), 0);
    expect("setup_noxdir_file", a_make_file(D "/noxdir/f", "data", 0600), 0);
    expect("chmod_rodir", syscall2(SYS_chmod, D "/rodir", 0500), 0);
    expect("chmod_noxdir", syscall2(SYS_chmod, D "/noxdir", 0600), 0);

    denied("readonly_file_eacces", cr(D "/ro", 0644));
    expect("readonly_file_size", a_stat_path(D "/ro", &st) == 0 ? (i64)st.st_size : -1, root ? 0 : 4);

    /* Write permission alone suffices: creat does not read. */
    i64 fd = cr(D "/wo", 0644);
    expect("writeonly_file_ok", fd, 3);
    a_close(fd);
    expect("writeonly_file_truncated", a_stat_path(D "/wo", &st) == 0 ? (i64)st.st_size : -1, 0);

    denied("new_in_readonly_dir_eacces", cr(D "/rodir/new", 0644));
    expect("new_in_readonly_dir_made", a_stat_path(D "/rodir/new", &st), root ? 0 : -ENOENT);
    /* An existing writable file in a read-only directory can be truncated. */
    fd = cr(D "/rodir/f", 0644);
    expect("existing_in_readonly_dir_ok", fd, 3);
    a_close(fd);
    expect("existing_in_readonly_dir_truncated", a_stat_path(D "/rodir/f", &st) == 0 ? (i64)st.st_size : -1, 0);

    denied("existing_in_nosearch_dir_eacces", cr(D "/noxdir/f", 0644));
    denied("new_in_nosearch_dir_eacces", cr(D "/noxdir/new", 0644));
    expect("missing_below_nosearch", cr(D "/noxdir/nothere/x", 0644), root ? -ENOENT : -EACCES);

    expect("restore_rodir", syscall2(SYS_chmod, D "/rodir", 0700), 0);
    expect("restore_noxdir", syscall2(SYS_chmod, D "/noxdir", 0700), 0);
    a_unlink(D "/rodir/new");
    a_unlink(D "/noxdir/new");
    expect("cleanup_rodir_f", a_unlink(D "/rodir/f"), 0);
    expect("cleanup_noxdir_f", a_unlink(D "/noxdir/f"), 0);
    expect("cleanup_rodir", a_rmdir(D "/rodir"), 0);
    expect("cleanup_noxdir", a_rmdir(D "/noxdir"), 0);
    expect("cleanup_ro", a_unlink(D "/ro"), 0);
    expect("cleanup_wo", a_unlink(D "/wo"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
