/* execve: failing calls return an error to the caller; a successful one
 * replaces the program, which receives exactly the argv and envp strings
 * given (argv[0] need not be the path). This program runs itself again;
 * the second image continues the checks. */
#include "a_sys.h"

#define D "/tmp/a_execve_args"

static int phase2(void) {
    /* argv: "first-arg", "2", <passed so far>, "with space", "" */
    passed = (int)a_atou(a_argv()[2]);
    char **argv = a_argv(), **envp = a_envp();
    expect("argc_5", a_argc(), 5);
    check("argv0_as_given", str_eq(argv[0], "first-arg"), 0);
    check("argv_with_space", str_eq(argv[3], "with space"), 0);
    check("argv_empty_string", argv[4][0] == 0, argv[4][0]);
    check("argv_null_terminated", argv[5] == 0, 0);
    check("env_0", envp[0] && str_eq(envp[0], "KEY=value"), 0);
    check("env_1", envp[1] && str_eq(envp[1], "EMPTY="), 0);
    check("env_null_terminated", envp[2] == 0, 0);
    check("execfn_is_path", str_eq((const char *)a_auxv(AT_EXECFN), D "/prog"), 0);
    expect("pagesz_aux", a_auxv(AT_PAGESZ), A_PAGE);
    expect("cleanup_prog", a_unlink(D "/prog"), 0);
    expect("cleanup_plain", a_unlink(D "/plain"), 0);
    expect("cleanup_garbage", a_unlink(D "/garbage"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}

/* Copy this program to `to` with mode 0755. */
static i64 copy_self(const char *to) {
    static char buf[65536];
    i64 in = a_open(a_self_path(), O_RDONLY, 0), out = a_open(to, O_WRONLY | O_CREAT | O_TRUNC, 0755), n;
    if (in < 0 || out < 0) return -1;
    while ((n = a_read(in, buf, sizeof buf)) > 0) a_write(out, buf, n);
    a_close(in);
    a_close(out);
    return 0;
}

int test_main(void) {
    if (a_argc() > 1 && str_eq(a_argv()[1], "2")) return phase2();
    char *noargv[] = {"x", 0};
    char *noenv[] = {0};
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("copy_self", copy_self(D "/prog"), 0);
    expect("setup_plain", a_make_file(D "/plain", "not a program\n", 0644), 0);
    expect("setup_garbage", a_make_file(D "/garbage", "\x01\x02garbage bytes", 0755), 0);
    expect("missing_enoent", syscall3(SYS_execve, D "/none", noargv, noenv), -ENOENT);
    expect("no_exec_bit_eacces", syscall3(SYS_execve, D "/plain", noargv, noenv), -EACCES);
    expect("not_a_program_enoexec", syscall3(SYS_execve, D "/garbage", noargv, noenv), -ENOEXEC);
    expect("directory_eacces", syscall3(SYS_execve, D, noargv, noenv), -EACCES);
    expect("through_file_enotdir", syscall3(SYS_execve, D "/plain/x", noargv, noenv), -ENOTDIR);
    expect("bad_path_efault", syscall3(SYS_execve, (const char *)8, noargv, noenv), -EFAULT);
    expect("bad_argv_efault", syscall3(SYS_execve, D "/prog", (char **)8, noenv), -EFAULT);
    char *bad_entry[] = {"x", (char *)8, 0};
    expect("bad_argv_entry_efault", syscall3(SYS_execve, D "/prog", bad_entry, noenv), -EFAULT);
    char count[24];
    a_utoa(count, (u64)passed + 1);
    char *argv[] = {"first-arg", "2", count, "with space", "", 0};
    char *envp[] = {"KEY=value", "EMPTY=", 0};
    /* Failures so far would be lost across the execve: stop here. */
    if (failed) return finish();
    put("ok exec_now\n");
    i64 r = syscall3(SYS_execve, D "/prog", argv, envp);
    expect("execve_returned", r, 0);
    return finish();
}
