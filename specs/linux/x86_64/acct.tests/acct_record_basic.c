/* With accounting on, a process that ends appends one 64-byte version-3
 * record (struct acct_v3, linux/acct.h) to the file, before its parent
 * can reap it. For a child of pid 1 in a new pid namespace that left its
 * session, named itself "acctchild" and called exit_group(7): ac_version
 * 3, AFORK (0x01) set in ac_flag (forked, never exec'd), AXSIG (0x10)
 * clear, ac_tty 0 (no controlling terminal), ac_exitcode 0x700 (the wait
 * status), ac_uid 0, ac_gid 0, ac_pid 2 and ac_ppid 1 (pids as seen in
 * the namespace that has accounting on), ac_comm "acctchild" padded with
 * NULs. The file was empty, so the record sits at offset 0.
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
#define F "/tmp/acct_record_basic"

static void body(void) {
    i64 r = syscall1(SYS_acct, F);
    check("on_ok", r == 0, r);
    i64 pid = run_child("acctchild", 7);
    check("child_pid_2", pid == 2, pid);
    r = file_size(F);
    check("one_record_after_reap", r == REC, r);
    struct acct_v3 rec[2];
    memset(rec, 0x55, sizeof rec);
    int n = read_records(F, 0, rec, 2);
    check("read_one_record", n == 1, n);
    check("version_3", rec[0].version == 3, rec[0].version);
    check("flag_afork", (rec[0].flag & AFORK) != 0, rec[0].flag);
    check("flag_no_axsig", (rec[0].flag & AXSIG) == 0, rec[0].flag);
    check("tty_0", rec[0].tty == 0, rec[0].tty);
    check("exitcode_wait_status", rec[0].exitcode == 0x700, rec[0].exitcode);
    check("uid_0", rec[0].uid == 0, rec[0].uid);
    check("gid_0", rec[0].gid == 0, rec[0].gid);
    check("pid_2", rec[0].pid == 2, rec[0].pid);
    check("ppid_1", rec[0].ppid == 1, rec[0].ppid);
    check("comm", str_eq(rec[0].comm, "acctchild"), 0);
    int pad = 1;
    for (int i = 9; i < 16; i++) pad &= rec[0].comm[i] == 0;
    check("comm_nul_padded", pad, 0);
    r = syscall1(SYS_acct, 0);
    check("off_ok", r == 0, r);
}

int test_main(void) {
    make_file(F, 0644);
    int code = in_pid_ns(0, body);
    check("child", code == 0, code);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, F, 0) == 0, 0);
    return finish();
}
