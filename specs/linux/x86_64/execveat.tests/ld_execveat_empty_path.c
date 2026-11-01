/* execveat(fd, "", argv, envp, AT_EMPTY_PATH) runs the file fd refers
 * to (fexecve): fd may be open O_RDONLY or O_PATH, and with FD_CLOEXEC
 * too for an ELF program (the descriptor is closed in the new image).
 * AT_EXECFN in the new image is "/dev/fd/<fd>". AT_EMPTY_PATH with a
 * non-empty path is ignored (the path is used). Descriptors without
 * FD_CLOEXEC stay open across each exec.
 *
 * The test copies its own binary to /tmp/ld_execveat_empty_path/prog and
 * goes through four images of it, carrying its check counts in argv.
 */
#include "a_sys.h"

#define SYS_execveat 322

static char buf[65536];
static char nb_pass[24], nb_fail[24];
#define DIR "/tmp/ld_execveat_empty_path"

static i64 copy_self(const char *dst) {
    i64 in = a_open(a_self_path(), O_RDONLY, 0);
    if (in < 0) return in;
    i64 out = a_open(dst, O_WRONLY | O_CREAT | O_TRUNC, 0755);
    if (out < 0) return out;
    i64 n;
    while ((n = a_read(in, buf, sizeof buf)) > 0) a_write(out, buf, n);
    a_close(in);
    a_close(out);
    return 0;
}

static char **next_argv(char *stage) {
    static char *v[5];
    a_utoa(nb_pass, passed);
    a_utoa(nb_fail, failed);
    v[0] = "prog";
    v[1] = stage;
    v[2] = nb_pass;
    v[3] = nb_fail;
    v[4] = 0;
    return v;
}

static int execfn_is(const char *want) {
    const char *got = (const char *)a_auxv(AT_EXECFN);
    return got && str_eq(got, want);
}

static void cleanup(void) {
    a_unlink(DIR "/prog");
    a_rmdir(DIR);
}

int test_main(void) {
    char **argv = a_argv();
    if (a_argc() == 4) {
        passed = (int)a_atou(argv[2]);
        failed = (int)a_atou(argv[3]);
    }
    const char *stage = a_argc() == 4 ? argv[1] : "";
    if (str_eq(stage, "s1")) {
        check("s1_execfn_dev_fd_3", execfn_is("/dev/fd/3"), 0);
        expect("s1_fd3_still_open", syscall2(SYS_fcntl, 3, F_GETFD), 0);
        a_close(3);
        i64 fd = a_open(DIR "/prog", O_PATH, 0);
        expect("s1_opath_fd_3", fd, 3);
        syscall5(SYS_execveat, fd, "", next_argv("s2"), 0, AT_EMPTY_PATH);
        check("s1_exec_opath_fd", 0, 0);
        return finish();
    }
    if (str_eq(stage, "s2")) {
        check("s2_execfn_dev_fd_3", execfn_is("/dev/fd/3"), 0);
        expect("s2_opath_fd_still_open", syscall2(SYS_fcntl, 3, F_GETFD), 0);
        expect("s2_opath_flags", syscall2(SYS_fcntl, 3, F_GETFL), O_PATH);
        a_close(3);
        i64 fd = a_open(DIR "/prog", O_RDONLY | O_CLOEXEC, 0);
        expect("s2_cloexec_fd_3", fd, 3);
        syscall5(SYS_execveat, fd, "", next_argv("s3"), 0, AT_EMPTY_PATH);
        check("s2_exec_cloexec_fd", 0, 0);
        return finish();
    }
    if (str_eq(stage, "s3")) {
        check("s3_execfn_dev_fd_3", execfn_is("/dev/fd/3"), 0);
        expect("s3_cloexec_fd_closed", syscall2(SYS_fcntl, 3, F_GETFD), -EBADF);
        /* AT_EMPTY_PATH is ignored when the path is not empty. */
        i64 d = a_open(DIR, O_RDONLY | O_DIRECTORY, 0);
        expect("s3_dirfd_3", d, 3);
        syscall5(SYS_execveat, d, "prog", next_argv("s4"), 0, AT_EMPTY_PATH);
        check("s3_exec_nonempty_path_with_flag", 0, 0);
        return finish();
    }
    if (str_eq(stage, "s4")) {
        check("s4_execfn_dev_fd_3_prog", execfn_is("/dev/fd/3/prog"), 0);
        cleanup();
        return finish();
    }

    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    expect("copy_self", copy_self(DIR "/prog"), 0);
    i64 fd = a_open(DIR "/prog", O_RDONLY, 0);
    expect("rdonly_fd_3", fd, 3);
    syscall5(SYS_execveat, fd, "", next_argv("s1"), 0, AT_EMPTY_PATH);
    check("exec_rdonly_fd", 0, 0);
    cleanup();
    return finish();
}
