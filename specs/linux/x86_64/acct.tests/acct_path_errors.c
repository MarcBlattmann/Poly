/* acct path errors for a privileged caller (pid 1 of a new pid namespace):
 * the file is never created (ENOENT (2) for a missing file, which stays
 * missing), an empty string is ENOENT, a path through a regular file is
 * ENOTDIR (20), a symlink loop ELOOP (40), a name of 4096 bytes before the
 * NUL ENAMETOOLONG (36), a bad pointer EFAULT (14), a directory EISDIR
 * (21). After all the failures accounting is still off: a child's exit
 * writes nothing anywhere, and acct(NULL) is 0.
 * poly-diff: needs=root (only root may switch process accounting)
 */
#include "poly_ipc.h"

#define SYS_acct 163
#define SYS_newfstatat 262
#define SYS_renameat 264
#define CLONE_NEWNS 0x00020000
#define CLONE_NEWPID 0x20000000
#define PR_SET_NAME 15
#define AFORK 0x01 /* linux/acct.h */
#define AXSIG 0x10 /* linux/acct.h */
#define REC 64     /* sizeof(struct acct_v3) */

/* struct acct_v3, linux/acct.h and acct(5) */
struct acct_v3 {
    unsigned char flag;
    unsigned char version;
    unsigned short tty;
    unsigned exitcode, uid, gid, pid, ppid, btime;
    float etime;
    unsigned short utime, stime, mem, io, rw, minflt, majflt, swaps;
    char comm[16];
};

struct k_stat {
    u64 dev, ino, nlink;
    unsigned mode, uid, gid, pad0;
    u64 rdev;
    i64 size, blksize, blocks;
    u64 times[6];
    i64 unused[3];
};

static __attribute__((unused)) i64 file_size(const char *path) {
    struct k_stat st;
    i64 r = syscall4(SYS_newfstatat, AT_FDCWD, path, &st, 0);
    return r < 0 ? r : st.size;
}

static __attribute__((unused)) void make_file(const char *path, unsigned mode) {
    syscall3(SYS_unlinkat, AT_FDCWD, path, 0);
    syscall1(SYS_close, syscall4(SYS_openat, AT_FDCWD, path, O_CREAT | O_WRONLY | O_TRUNC, mode));
}

/* Reads up to `max` records from offset `off` of `path`; returns the count. */
static __attribute__((unused)) int read_records(const char *path, i64 off, struct acct_v3 *out, int max) {
    i64 fd = syscall4(SYS_openat, AT_FDCWD, path, O_RDONLY, 0);
    if (fd < 0) return -1;
    syscall3(SYS_lseek, fd, off, 0);
    i64 n = read((int)fd, out, (u64)max * REC);
    close((int)fd);
    return n < 0 ? -1 : (int)(n / REC);
}

/* A child that names itself, leaves its session (no controlling tty) and
 * exits with `code`. Returns its pid after it has been reaped. */
static __attribute__((unused)) i64 run_child(const char *name, int code) {
    i64 pid = fork();
    if (pid == 0) {
        syscall0(SYS_setsid);
        syscall2(SYS_prctl, PR_SET_NAME, name);
        exit_group(code);
    }
    wait_status(pid);
    return pid;
}

/* Run `body` as pid 1 of a new pid namespace (plus `extra` clone flags),
 * so the accounting it switches on covers only its own namespace. The
 * child never calls body unless it really is pid 1 of a fresh namespace.
 * Returns the child's exit code. */
static __attribute__((unused)) int in_pid_ns(u64 extra, void (*body)(void)) {
    i64 pid = syscall5(SYS_clone, CLONE_NEWPID | extra | SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        i64 me = getpid();
        check("pid_1_in_new_ns", me == 1, me);
        if (me != 1) exit_group(1);
        body();
        exit_group(failed ? 1 : 0);
    }
    check("clone_newpid", pid > 0, pid);
    if (pid < 0) return -1;
    return wait_exit(pid);
}
#define EISDIR 21
#define D "/tmp/acct_path_errors"

static char longname[4200];

static void body(void) {
    i64 r = syscall1(SYS_acct, D "/missing");
    check("missing_enoent", r == -ENOENT, r);
    r = file_size(D "/missing");
    check("missing_not_created", r == -ENOENT, r);
    r = syscall1(SYS_acct, D "/nodir/file");
    check("missing_dir_enoent", r == -ENOENT, r);
    r = syscall1(SYS_acct, "");
    check("empty_enoent", r == -ENOENT, r);
    r = syscall1(SYS_acct, D "/file/x");
    check("through_file_enotdir", r == -ENOTDIR, r);
    r = syscall1(SYS_acct, D "/loop");
    check("symlink_loop_eloop", r == -ELOOP, r);
    r = syscall1(SYS_acct, longname);
    check("too_long_enametoolong", r == -ENAMETOOLONG, r);
    r = syscall1(SYS_acct, 8);
    check("bad_pointer_efault", r == -EFAULT, r);
    r = syscall1(SYS_acct, D);
    check("directory_eisdir", r == -EISDIR, r);
    r = syscall1(SYS_acct, "/");
    check("root_dir_eisdir", r == -EISDIR, r);

    i64 pid = fork();
    if (pid == 0) exit_group(0);
    wait_status(pid);
    r = file_size(D "/file");
    check("file_untouched", r == 0, r);
    r = syscall1(SYS_acct, 0);
    check("still_off", r == 0, r);
}

int test_main(void) {
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    make_file(D "/file", 0644);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/loop", 0);
    syscall3(SYS_symlinkat, D "/loop", AT_FDCWD, D "/loop");
    longname[0] = '/';
    for (int i = 1; i < 4096; i++) longname[i] = 'a';
    longname[4096] = 0;
    int code = in_pid_ns(0, body);
    check("child", code == 0, code);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/loop", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/file", 0);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
