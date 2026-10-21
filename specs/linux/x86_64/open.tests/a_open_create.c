/* open(2) with O_CREAT and O_EXCL: the mode of a new file under the umask,
 * mode bits outside 07777, special bits, and existing names. */
#include "a_sys.h"

#define SYS_open 2
#define D "/tmp/a_open_create"

static i64 op(const char *p, i64 flags, i64 mode) { return syscall3(SYS_open, p, flags, mode); }

static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_stat_path(p, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);

    i64 fd = op(D "/a", O_WRONLY | O_CREAT, 0777);
    expect("create_fd_3", fd, 3);
    expect("create_umask_022_applied", mode_of(D "/a"), S_IFREG | 0755);
    expect("fstat_new", syscall2(SYS_fstat, fd, &st), 0);
    expect("new_size_0", st.st_size, 0);
    expect("new_nlink_1", st.st_nlink, 1);
    expect("new_owner_uid", st.st_uid, syscall0(SYS_geteuid));
    expect("new_owner_gid", st.st_gid, syscall0(SYS_getegid));
    a_close(fd);

    /* With O_CREAT and an existing name, the file is just opened. */
    a_make_file(D "/a", "keep", 0600);
    fd = op(D "/a", O_RDWR | O_CREAT, 0600);
    expect("existing_opens", fd, 3);
    expect("existing_keeps_mode", mode_of(D "/a"), S_IFREG | 0755);
    expect("existing_keeps_size", syscall2(SYS_fstat, fd, &st) == 0 ? st.st_size : -1, 4);
    a_close(fd);
    expect("excl_existing_eexist", op(D "/a", O_RDWR | O_CREAT | O_EXCL, 0600), -EEXIST);
    expect("excl_existing_rdonly_eexist", op(D "/a", O_RDONLY | O_CREAT | O_EXCL, 0600), -EEXIST);

    /* O_EXCL without O_CREAT on a regular file is ignored. */
    fd = op(D "/a", O_RDONLY | O_EXCL, 0);
    expect("excl_without_creat_opens", fd, 3);
    a_close(fd);

    /* O_CREAT|O_RDONLY makes the file even if the mode forbids writing. */
    fd = op(D "/ro", O_RDONLY | O_CREAT | O_EXCL, 0444);
    expect("create_rdonly_mode_0444", fd, 3);
    expect("create_rdonly_mode_is_0444", mode_of(D "/ro"), S_IFREG | 0444);
    a_close(fd);
    /* The new descriptor may write even though the mode it set says no. */
    fd = op(D "/ro2", O_WRONLY | O_CREAT | O_EXCL, 0);
    expect("create_mode_0_fd", fd, 3);
    expect("create_mode_0_write_ok", a_write(fd, "z", 1), 1);
    expect("create_mode_0_is_0", mode_of(D "/ro2"), S_IFREG | 0);
    a_close(fd);

    /* Bits above 07777 in mode are ignored; setuid, setgid, sticky kept. */
    fd = op(D "/hi", O_WRONLY | O_CREAT, 0170644);
    expect("high_mode_bits_fd", fd, 3);
    expect("high_mode_bits_ignored", mode_of(D "/hi"), S_IFREG | 0644);
    a_close(fd);
    fd = op(D "/sp", O_WRONLY | O_CREAT, 07755);
    expect("special_bits_fd", fd, 3);
    expect("special_bits_kept", mode_of(D "/sp") & 07777, 07755);
    a_close(fd);

    /* The umask is read at the time of the call. */
    i64 old = syscall1(SYS_umask, 0);
    expect("umask_was_022", old, 022);
    fd = op(D "/u0", O_WRONLY | O_CREAT, 0666);
    a_close(fd);
    expect("umask_0_mode_0666", mode_of(D "/u0"), S_IFREG | 0666);
    syscall1(SYS_umask, 0777);
    fd = op(D "/u7", O_RDWR | O_CREAT, 0666);
    expect("umask_0777_still_opens_rdwr", fd, 3);
    expect("umask_0777_write_ok", a_write(fd, "q", 1), 1);
    a_close(fd);
    expect("umask_0777_mode_0", mode_of(D "/u7"), S_IFREG | 0);
    syscall1(SYS_umask, 022);

    /* Without O_CREAT, the mode argument is ignored entirely. */
    expect("no_creat_missing_enoent", op(D "/m", O_RDWR, 0644), -ENOENT);
    expect("no_creat_not_created", mode_of(D "/m"), -ENOENT);
    expect("creat_missing_parent_enoent", op(D "/no/x", O_RDWR | O_CREAT, 0644), -ENOENT);

    const char *names[] = {D "/a", D "/ro", D "/ro2", D "/hi", D "/sp", D "/u0", D "/u7"};
    for (int i = 0; i < 7; i++) expect("cleanup_unlink", a_unlink(names[i]), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
