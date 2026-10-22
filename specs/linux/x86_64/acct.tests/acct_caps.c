/* The privilege acct checks is CAP_SYS_PACCT (20) in the effective set:
 * root that removes only that bit from its effective set gets EPERM (1),
 * also for acct(NULL); with it back the same call works. Root that keeps
 * only CAP_SYS_PACCT is subject to normal file permissions: a 0444 file
 * it owns is EACCES (13), a file under a 0600 directory (no search
 * permission) is EACCES, a 0644 file it owns works. Run as pid 1 of a new
 * pid namespace.
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
#define D "/tmp/acct_caps"
#define CAP_SYS_PACCT 20 /* linux/capability.h */
#define SYS_fchmodat 268

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

static void body(void) {
    struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    struct cap_data d[2];
    syscall2(SYS_capget, &h, d);
    d[0].effective &= ~(1U << CAP_SYS_PACCT);
    i64 r = syscall2(SYS_capset, &h, d);
    check("drop_pacct_effective", r == 0, r);
    r = syscall1(SYS_acct, D "/rw");
    check("without_pacct_eperm", r == -EPERM, r);
    r = syscall1(SYS_acct, 0);
    check("without_pacct_null_eperm", r == -EPERM, r);
    d[0].effective |= 1U << CAP_SYS_PACCT;
    r = syscall2(SYS_capset, &h, d);
    check("restore_pacct", r == 0, r);
    r = syscall1(SYS_acct, D "/rw");
    check("with_pacct_ok", r == 0, r);
    r = syscall1(SYS_acct, 0);
    check("with_pacct_off_ok", r == 0, r);

    memset(d, 0, sizeof d);
    d[0].effective = d[0].permitted = 1U << CAP_SYS_PACCT;
    r = syscall2(SYS_capset, &h, d);
    check("keep_only_pacct", r == 0, r);
    r = syscall1(SYS_acct, D "/ro");
    check("readonly_file_eacces", r == -EACCES, r);
    r = syscall1(SYS_acct, D "/nosearch/file");
    check("no_search_eacces", r == -EACCES, r);
    r = syscall1(SYS_acct, D "/rw");
    check("writable_file_ok", r == 0, r);
    r = syscall1(SYS_acct, 0);
    check("off_ok", r == 0, r);
}

int test_main(void) {
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    syscall3(SYS_mkdirat, AT_FDCWD, D "/nosearch", 0700);
    make_file(D "/nosearch/file", 0644);
    syscall3(SYS_fchmodat, AT_FDCWD, D "/nosearch", 0600);
    make_file(D "/ro", 0444);
    make_file(D "/rw", 0644);
    int code = in_pid_ns(0, body);
    check("child", code == 0, code);
    syscall3(SYS_fchmodat, AT_FDCWD, D "/nosearch", 0700);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/nosearch/file", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/nosearch", AT_REMOVEDIR);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/ro", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/rw", 0);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
