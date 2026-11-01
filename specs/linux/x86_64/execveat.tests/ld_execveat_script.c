/* execveat of a "#!" script: the interpreter named on the first line
 * runs with argv = [interpreter path, the optional argument from the "#!"
 * line, the script's name, then argv[1..] of the call]. The script's
 * name is "/dev/fd/<dirfd>/<path>" for a relative path with a dirfd and
 * "/dev/fd/<fd>" for AT_EMPTY_PATH; AT_EXECFN is that name too. When the
 * descriptor (the dirfd or the AT_EMPTY_PATH fd) has FD_CLOEXEC, the
 * script would be unreachable to the interpreter, and execveat fails with
 * ENOENT (an ELF program is still run from such a descriptor).
 *
 * The interpreter is a copy of this test's binary, under
 * /tmp/ld_execveat_script/; check counts travel in argv.
 */
#include "a_sys.h"

#define SYS_execveat 322

static char buf[65536];
static char nb_pass[24], nb_fail[24];
#define DIR "/tmp/ld_execveat_script"

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

/* Script argv: argv[0], the stage marker, the counts. The interpreter
 * receives [interp, "iarg", script, stage, passed, failed]. */
static char **next_argv(char *stage) {
    static char *v[5];
    a_utoa(nb_pass, passed);
    a_utoa(nb_fail, failed);
    v[0] = "ignored-argv0";
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
    a_unlink(DIR "/interp");
    a_unlink(DIR "/script");
    a_rmdir(DIR);
}

int test_main(void) {
    char **argv = a_argv();
    const char *stage = "";
    if (a_argc() == 6) {
        passed = (int)a_atou(argv[4]);
        failed = (int)a_atou(argv[5]);
        stage = argv[3];
    }
    if (str_eq(stage, "s1")) {
        check("s1_argv0_interpreter", str_eq(argv[0], DIR "/interp"), 0);
        check("s1_argv1_hashbang_arg", str_eq(argv[1], "iarg"), 0);
        check("s1_argv2_dev_fd_3_script", str_eq(argv[2], "/dev/fd/3/script"), 0);
        check("s1_execfn", execfn_is("/dev/fd/3/script"), 0);
        check("s1_env_kept", a_envp()[0] && str_eq(a_envp()[0], "E=1") && !a_envp()[1], 0);
        a_close(3);
        /* A script through a FD_CLOEXEC descriptor: ENOENT. */
        i64 c = a_open(DIR "/script", O_RDONLY | O_CLOEXEC, 0);
        expect("s1_cloexec_fd_3", c, 3);
        expect("s1_cloexec_fd_script_enoent", syscall5(SYS_execveat, c, "", next_argv("bad"), 0, AT_EMPTY_PATH),
               -ENOENT);
        a_close(c);
        i64 cd = a_open(DIR, O_RDONLY | O_DIRECTORY | O_CLOEXEC, 0);
        expect("s1_cloexec_dirfd_script_enoent", syscall5(SYS_execveat, cd, "script", next_argv("bad"), 0, 0),
               -ENOENT);
        /* Through a descriptor without FD_CLOEXEC it works. */
        a_close(cd);
        i64 fd = a_open(DIR "/script", O_RDONLY, 0);
        expect("s1_fd_3", fd, 3);
        syscall5(SYS_execveat, fd, "", next_argv("s2"), 0, AT_EMPTY_PATH);
        check("s1_exec_script_fd", 0, 0);
        return finish();
    }
    if (str_eq(stage, "s2")) {
        check("s2_argv0_interpreter", str_eq(argv[0], DIR "/interp"), 0);
        check("s2_argv1_hashbang_arg", str_eq(argv[1], "iarg"), 0);
        check("s2_argv2_dev_fd_3", str_eq(argv[2], "/dev/fd/3"), 0);
        check("s2_execfn", execfn_is("/dev/fd/3"), 0);
        check("s2_no_env", a_envp()[0] == 0, 0);
        cleanup();
        return finish();
    }

    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    expect("copy_self", copy_self(DIR "/interp"), 0);
    expect("script", a_make_file(DIR "/script", "#!" DIR "/interp iarg\n", 0755), 0);
    syscall2(SYS_chmod, DIR "/script", 0755);
    i64 d = a_open(DIR, O_RDONLY | O_DIRECTORY, 0);
    expect("dirfd_3", d, 3);
    static char *envp[] = {"E=1", 0};
    syscall5(SYS_execveat, d, "script", next_argv("s1"), envp, 0);
    check("exec_script_dirfd", 0, 0);
    cleanup();
    return finish();
}
