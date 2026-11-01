/* execveat failures, and the order of its checks. A failed execveat
 * returns -errno and the caller goes on in its own image.
 *   - the path is read first: NULL path EFAULT, even with bad flags;
 *   - flags (an int: bits 32..63 ignored) other than AT_EMPTY_PATH and
 *     AT_SYMLINK_NOFOLLOW: EINVAL, before dirfd and lookup;
 *   - lookup: bad dirfd EBADF, file dirfd with a relative path ENOTDIR,
 *     missing file or empty path without AT_EMPTY_PATH ENOENT, symlink
 *     with AT_SYMLINK_NOFOLLOW ELOOP (also an O_PATH descriptor of a
 *     symlink with AT_EMPTY_PATH);
 *   - the file: not executable or not a regular file (a directory, by
 *     name or as an AT_EMPTY_PATH descriptor) EACCES; open for writing
 *     ETXTBSY;
 *   - then argv and envp are read: EFAULT (a bad argv with a missing
 *     file is ENOENT, with a mode-0644 file EACCES, with bad flags EINVAL);
 *   - then the format: not ELF and no "#!" (also an empty file) ENOEXEC;
 *     a "#!" script whose interpreter does not exist ENOENT.
 */
#include "a_sys.h"

#define SYS_execveat 322
#define DIR "/tmp/ld_execveat_errors"

static char *av[] = {"prog", "x", 0};
static char *ev[] = {"A=1", 0};

static i64 xat(i64 fd, const void *path, const void *argv, const void *envp, u64 flags) {
    return syscall5(SYS_execveat, fd, path, argv, envp, flags);
}

static void cleanup(void) {
    static const char *names[] = {DIR "/noexec", DIR "/garbage", DIR "/empty", DIR "/badscript",
                                  DIR "/busy", DIR "/link", 0};
    for (int i = 0; names[i]; i++) a_unlink(names[i]);
    a_rmdir(DIR);
}

static void make_exec(const char *path, const char *text) {
    a_make_file(path, text, 0755);
    syscall2(SYS_chmod, path, 0755);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    expect("noexec", a_make_file(DIR "/noexec", "#!/bin/sh\n", 0644), 0);
    make_exec(DIR "/garbage", "not an executable\n");
    make_exec(DIR "/empty", "");
    make_exec(DIR "/badscript", "#!" DIR "/no-such-interpreter\n");
    make_exec(DIR "/busy", "#!/bin/sh\n");
    expect("symlink", syscall2(SYS_symlink, "garbage", DIR "/link"), 0);
    i64 d = a_open(DIR, O_RDONLY | O_DIRECTORY, 0);
    expect("dirfd_3", d, 3);

    /* Path, then flags, then descriptor. */
    expect("null_path_efault", xat(d, 0, av, ev, 0), -EFAULT);
    expect("null_path_before_flags", xat(d, 0, av, ev, 1), -EFAULT);
    expect("flag_1_einval", xat(d, "garbage", av, ev, 1), -EINVAL);
    expect("flag_removedir_einval", xat(d, "garbage", av, ev, AT_REMOVEDIR), -EINVAL);
    expect("flag_follow_einval", xat(d, "garbage", av, ev, AT_SYMLINK_FOLLOW), -EINVAL);
    expect("flags_before_badfd", xat(99, "garbage", av, ev, 1), -EINVAL);
    expect("flags_before_enoent", xat(d, "nope", av, ev, 1), -EINVAL);
    expect("flags_high_bits_ignored", xat(d, "nope", av, ev, 1UL << 32), -ENOENT);

    /* Lookup. */
    expect("badfd", xat(99, "garbage", av, ev, 0), -EBADF);
    expect("badfd_empty_path", xat(99, "", av, ev, AT_EMPTY_PATH), -EBADF);
    i64 f = a_open(DIR "/garbage", O_RDONLY, 0);
    expect("file_fd_4", f, 4);
    expect("file_dirfd_enotdir", xat(f, "garbage", av, ev, 0), -ENOTDIR);
    expect("missing_enoent", xat(d, "nope", av, ev, 0), -ENOENT);
    expect("empty_path_no_flag_enoent", xat(d, "", av, ev, 0), -ENOENT);
    expect("nofollow_symlink_eloop", xat(d, "link", av, ev, AT_SYMLINK_NOFOLLOW), -ELOOP);
    i64 pl = a_open(DIR "/link", O_PATH | O_NOFOLLOW, 0);
    expect("opath_symlink_fd_5", pl, 5);
    expect("opath_symlink_empty_path_eloop", xat(pl, "", av, ev, AT_EMPTY_PATH), -ELOOP);
    expect("nofollow_missing_enoent", xat(d, "nope", av, ev, AT_SYMLINK_NOFOLLOW), -ENOENT);

    /* The file itself. */
    expect("mode_0644_eacces", xat(d, "noexec", av, ev, 0), -EACCES);
    expect("directory_eacces", xat(AT_FDCWD, DIR, av, ev, 0), -EACCES);
    expect("dirfd_empty_path_eacces", xat(d, "", av, ev, AT_EMPTY_PATH), -EACCES);
    i64 w = a_open(DIR "/busy", O_WRONLY, 0);
    expect("writer_fd_6", w, 6);
    expect("open_for_write_etxtbsy", xat(d, "busy", av, ev, 0), -ETXTBSY);
    a_close(w);

    /* argv and envp after the file, before the format. */
    expect("bad_argv_efault", xat(d, "garbage", (void *)8, ev, 0), -EFAULT);
    expect("bad_envp_efault", xat(d, "garbage", av, (void *)8, 0), -EFAULT);
    static char *bad_elem[] = {"prog", (char *)8, 0};
    expect("bad_argv_element_efault", xat(d, "garbage", bad_elem, ev, 0), -EFAULT);
    expect("bad_argv_missing_file_enoent", xat(d, "nope", (void *)8, ev, 0), -ENOENT);
    expect("bad_argv_noexec_eacces", xat(d, "noexec", (void *)8, ev, 0), -EACCES);
    expect("bad_argv_bad_flags_einval", xat(d, "garbage", (void *)8, ev, 1), -EINVAL);
    expect("bad_argv_badfd_ebadf", xat(99, "garbage", (void *)8, ev, 0), -EBADF);

    /* Format. */
    expect("garbage_enoexec", xat(d, "garbage", av, ev, 0), -ENOEXEC);
    expect("empty_file_enoexec", xat(d, "empty", av, ev, 0), -ENOEXEC);
    expect("garbage_via_symlink_enoexec", xat(d, "link", av, ev, 0), -ENOEXEC);
    expect("garbage_fd_empty_path_enoexec", xat(f, "", av, ev, AT_EMPTY_PATH), -ENOEXEC);
    expect("script_missing_interpreter_enoent", xat(d, "badscript", av, ev, 0), -ENOENT);

    /* Still the same image, with the same descriptors. */
    expect("still_here_fd3", syscall2(SYS_fcntl, d, F_GETFD), 0);
    a_close(d);
    a_close(f);
    a_close(pl);
    cleanup();
    return finish();
}
