/* open(2) and permissions: the file's mode against the access mode, search
 * permission on directories, write permission for O_CREAT and O_TRUNC, and
 * O_NOATIME on a file one does not own. As root (CAP_DAC_OVERRIDE,
 * CAP_FOWNER) the checks pass; as any other user they fail as below. */
#include "a_sys.h"

#define SYS_open 2
#define D "/tmp/a_open_perm"

static i64 op(const char *p, i64 flags, i64 mode) { return syscall3(SYS_open, p, flags, mode); }

static int root;

/* Root expects success, everyone else `err`; closes what opened. */
static void denied(const char *name, i64 got, i64 err) {
    check(name, root ? got >= 0 : got == err, got);
    if (got >= 0) a_close(got);
}

int test_main(void) {
    root = syscall0(SYS_geteuid) == 0;
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_r", a_make_file(D "/r", "data", 0400), 0);
    expect("setup_w", a_make_file(D "/w", "data", 0200), 0);
    expect("setup_none", a_make_file(D "/none", "data", 0), 0);
    expect("setup_x", a_make_file(D "/x", "data", 0100), 0);

    i64 fd = op(D "/r", O_RDONLY, 0);
    expect("read_only_file_rdonly", fd, 3);
    a_close(fd);
    denied("read_only_file_wronly", op(D "/r", O_WRONLY, 0), -EACCES);
    denied("read_only_file_rdwr", op(D "/r", O_RDWR, 0), -EACCES);
    denied("read_only_file_trunc", op(D "/r", O_RDONLY | O_TRUNC, 0), -EACCES);
    expect("read_only_file_not_truncated", a_stat_path(D "/r", &st) == 0 ? st.st_size : -1, root ? 0 : 4);
    denied("read_only_file_accmode_3", op(D "/r", 3, 0), -EACCES);

    fd = op(D "/w", O_WRONLY, 0);
    expect("write_only_file_wronly", fd, 3);
    a_close(fd);
    denied("write_only_file_rdonly", op(D "/w", O_RDONLY, 0), -EACCES);
    fd = op(D "/w", O_WRONLY | O_APPEND, 0);
    expect("write_only_file_append", fd, 3);
    a_close(fd);

    denied("mode_0_rdonly", op(D "/none", O_RDONLY, 0), -EACCES);
    denied("mode_0_wronly", op(D "/none", O_WRONLY, 0), -EACCES);
    denied("exec_only_rdonly", op(D "/x", O_RDONLY, 0), -EACCES);
    /* O_PATH needs no permission on the file itself. */
    fd = op(D "/none", O_PATH, 0);
    expect("mode_0_opath", fd, 3);
    a_close(fd);
    /* O_CREAT on an existing file checks the access mode, not creation. */
    denied("mode_0_creat_existing", op(D "/none", O_RDONLY | O_CREAT, 0600), -EACCES);
    expect("mode_0_creat_excl_eexist", op(D "/none", O_RDONLY | O_CREAT | O_EXCL, 0600), -EEXIST);

    /* A directory without write permission: no new names in it. */
    expect("setup_ro_dir", a_mkdir(D "/rodir", 0500), 0);
    denied("creat_in_ro_dir", op(D "/rodir/new", O_RDWR | O_CREAT, 0600), -EACCES);
    if (root) a_unlink(D "/rodir/new");
    expect("missing_in_ro_dir_enoent", op(D "/rodir/nope", O_RDONLY, 0), -ENOENT);

    /* A directory without search permission: nothing below it. */
    expect("setup_nox_dir", a_mkdir(D "/noxdir", 0700), 0);
    expect("setup_nox_file", a_make_file(D "/noxdir/f", "z", 0600), 0);
    expect("chmod_nox_dir", syscall2(SYS_chmod, D "/noxdir", 0600), 0);
    denied("open_below_nox_dir", op(D "/noxdir/f", O_RDONLY, 0), -EACCES);
    denied("missing_below_nox_dir", op(D "/noxdir/nope", O_RDONLY, 0), root ? -ENOENT : -EACCES);
    fd = op(D "/noxdir", O_RDONLY, 0);
    expect("nox_dir_itself_opens", fd, 3);
    a_close(fd);

    /* O_NOATIME needs ownership: fine on one's own file. */
    fd = op(D "/r", O_RDONLY | O_NOATIME, 0);
    expect("noatime_own_file", fd, 3);
    expect("noatime_getfl", syscall3(SYS_fcntl, fd, F_GETFL, 0), O_RDONLY | O_NOATIME | O_LARGEFILE);
    a_close(fd);

    syscall2(SYS_chmod, D "/noxdir", 0700);
    a_unlink(D "/noxdir/f");
    expect("cleanup_rmdir_nox", a_rmdir(D "/noxdir"), 0);
    expect("cleanup_rmdir_ro", a_rmdir(D "/rodir"), 0);
    const char *names[] = {D "/r", D "/w", D "/none", D "/x"};
    for (int i = 0; i < 4; i++) expect("cleanup_unlink", a_unlink(names[i]), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
