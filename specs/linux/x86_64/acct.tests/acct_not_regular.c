/* acct only takes regular files: a character device node (made with
 * mknod, major 1 minor 3) is EACCES (13), and so is a symlink to it. A symlink is followed:
 * a symlink to a regular file works (0), and the record of a child that
 * exits lands in the file it points to; the symlink itself stays a
 * symlink. Run as pid 1 of a new pid namespace.
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
#define SYS_readlinkat 267
#define SYS_mknodat 259
#define S_IFCHR 0020000
#define D "/tmp/acct_not_regular"

static void body(void) {
    i64 r = syscall4(SYS_mknodat, AT_FDCWD, D "/dev", S_IFCHR | 0666, 0x103);
    check("mknod_chr", r == 0, r);
    r = syscall1(SYS_acct, D "/dev");
    check("char_device_eacces", r == -EACCES, r);
    r = syscall1(SYS_acct, D "/devlink");
    check("symlink_to_device_eacces", r == -EACCES, r);
    r = syscall1(SYS_acct, D "/link");
    check("symlink_to_file_ok", r == 0, r);
    run_child("viasymlink", 0);
    r = file_size(D "/file");
    check("record_in_target", r == REC, r);
    r = syscall1(SYS_acct, 0);
    check("off_ok", r == 0, r);
    struct acct_v3 rec[4];
    int n = read_records(D "/file", 0, rec, 4);
    check("first_record_is_child", n >= 1 && str_eq(rec[0].comm, "viasymlink"), n);
    char buf[64];
    r = syscall4(SYS_readlinkat, AT_FDCWD, D "/link", buf, sizeof buf);
    check("link_still_symlink", r == (i64)str_len(D "/file"), r);
}

int test_main(void) {
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    make_file(D "/file", 0644);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/link", 0);
    syscall3(SYS_symlinkat, D "/file", AT_FDCWD, D "/link");
    syscall3(SYS_unlinkat, AT_FDCWD, D "/dev", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/devlink", 0);
    syscall3(SYS_symlinkat, D "/dev", AT_FDCWD, D "/devlink");
    int code = in_pid_ns(0, body);
    check("child", code == 0, code);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/link", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/devlink", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/dev", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/file", 0);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
