/* The start-up of a Chrome-style namespace sandbox, by an unprivileged
 * user (uid/gid 65534, no capabilities), without a private mount
 * namespace. One clone(CLONE_NEWUSER | CLONE_NEWPID | CLONE_NEWNET) makes
 * the sandboxed process: pid 1 with parent 0, ids unmapped (65534) until
 * it writes setgroups "deny", gid_map and uid_map "0 65534 1" itself;
 * then it is uid 0 with all capabilities in the new user namespace. It
 * chroot()s into "/proc/self/fdinfo" (a directory that holds one entry
 * per open descriptor and nothing else; chroot works because the
 * namespace's root has CAP_SYS_CHROOT) and does chdir("/"): from then on
 * "/etc", "/proc" and "/tmp" are ENOENT, open("/etc/passwd") is ENOENT,
 * "/" lists the descriptor "0", and ".." of "/" is "/". The network
 * namespace has only a down loopback: a TCP connect to 127.0.0.1 is
 * ENETUNREACH while AF_UNIX socketpair works. The process then empties
 * its bounding, permitted, effective and inheritable capability sets: a
 * second chroot("..") is EPERM although uid is still 0, sets
 * no_new_privs, installs a SIGSYS handler (SA_SIGINFO) and a filter that
 * answers openat with SECCOMP_RET_TRAP (data 0x42) and allows the rest:
 * openat("/x") does not run, the handler sees si_signo 31, si_code 1
 * (SYS_SECCOMP), si_errno 0x42, si_syscall 257 and si_arch 0xc000003e, and
 * by storing -EACCES (13) in the saved RAX makes the call return -13 ("trap
 * and emulate"); other calls (getpid) are unaffected. A fork child under
 * a filter that answers ptrace with KILL_PROCESS is killed by SIGSYS when
 * it calls ptrace.
 * poly-diff: needs=root (the test starts as root and becomes uid 65534; creating user namespaces is a system setting)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"
#include "p_seccomp.h"

#define SYS_openat 257
#define SYS_ptrace 101
#define SA_SIGINFO 4

struct cap_hdr {
    u32 version;
    i32 pid;
};
struct cap_data {
    u32 effective, permitted, inheritable;
};

static volatile i32 seen_signo, seen_code, seen_errno, seen_syscall, seen_calls;
static volatile u32 seen_arch;

static void on_sigsys(int sig, void *info, void *uctx) {
    (void)sig;
    i32 *si = (i32 *)info;
    seen_signo = si[0];
    seen_errno = si[1];
    seen_code = si[2];
    seen_syscall = si[6];
    seen_arch = (u32)si[7];
    seen_calls++;
    *(i64 *)((char *)uctx + 144) = -EACCES;
}

static int sandbox(void) {
    struct a_stat st;
    expect("pid_1", syscall0(SYS_getpid), 1);
    expect("ppid_0", syscall0(SYS_getppid), 0);
    expect("uid_unmapped", syscall0(SYS_getuid), 65534);
    expect("write_maps", p_write_maps(0, 65534, 65534, 1), 0);
    expect("uid_0", syscall0(SYS_getuid), 0);

    expect("chroot_fdinfo", syscall1(SYS_chroot, "/proc/self/fdinfo"), 0);
    expect("chdir_root", p_chdir("/"), 0);
    expect("etc_gone", a_stat_path("/etc", &st), -ENOENT);
    expect("proc_gone", a_stat_path("/proc", &st), -ENOENT);
    expect("tmp_gone", a_stat_path("/tmp", &st), -ENOENT);
    expect("passwd_gone", a_open("/etc/passwd", O_RDONLY, 0), -ENOENT);
    check("lists_fd_0", p_dir_has("/", "0"), 0);
    struct a_stat s1, s2;
    expect("stat_root", a_stat_path("/", &s1), 0);
    expect("stat_dotdot", a_stat_path("/..", &s2), 0);
    check("dotdot_is_root", s1.st_ino == s2.st_ino && s1.st_dev == s2.st_dev, 0);

    i64 s = syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    check("tcp_socket", s >= 0, s);
    u8 addr[16] = {2, 0, 0, 80, 127, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0};
    expect("connect_lo_down", syscall3(SYS_connect, s, addr, 16), -ENETUNREACH);
    a_close(s);
    i32 sv[2];
    expect("unix_socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, sv), 0);

    int dropped = 1;
    for (i64 c = 0; syscall5(SYS_prctl, PR_CAPBSET_READ, c, 0, 0, 0) >= 0; c++)
        if (syscall5(SYS_prctl, PR_CAPBSET_DROP, c, 0, 0, 0) != 0) dropped = 0;
    check("bounding_dropped", dropped, 0);
    struct cap_hdr hdr = {0x20080522, 0};
    struct cap_data d[2] = {{0, 0, 0}, {0, 0, 0}};
    expect("capset_empty", syscall2(SYS_capset, &hdr, d), 0);
    expect("chroot_again_eperm", syscall1(SYS_chroot, ".."), -EPERM);
    expect("still_uid_0", syscall0(SYS_getuid), 0);
    expect("set_nnp", syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);

    struct a_sigaction sa = {(u64)on_sigsys, SA_SIGINFO | SA_RESTORER, (u64)p_restorer, 0};
    expect("sigaction", syscall4(SYS_rt_sigaction, SIGSYS, &sa, 0, 8), 0);
    struct pk_insn f[] = {
        PK_LDABS(SD_NR),
        PK_JEQ(SYS_openat, 0, 1),
        PK_RET(SECCOMP_RET_TRAP | 0x42),
        PK_ALLOW,
    };
    struct pk_prog prog = {PK_N(f), f};
    expect("filter", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog), 0);
    i64 r = syscall4(SYS_openat, -100, "/x", O_RDONLY, 0);
    expect("emulated_result", r, -EACCES);
    expect("handler_ran_once", seen_calls, 1);
    expect("si_signo", seen_signo, 31);
    expect("si_code", seen_code, 1);
    expect("si_errno", seen_errno, 0x42);
    expect("si_syscall", seen_syscall, 257);
    expect("si_arch", (i64)seen_arch, 0xc000003e);
    expect("getpid_unaffected", syscall0(SYS_getpid), 1);

    i64 kid = p_fork();
    if (kid == 0) {
        struct pk_insn k[] = {
            PK_LDABS(SD_NR),
            PK_JEQ(SYS_ptrace, 0, 1),
            PK_RET(SECCOMP_RET_KILL_PROCESS),
            PK_ALLOW,
        };
        struct pk_prog kp = {PK_N(k), k};
        if (syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &kp) != 0) p_exit(2);
        syscall4(SYS_ptrace, 0, 0, 0, 0);
        p_exit(3);
    }
    int st2 = p_wait(kid);
    expect("kill_process_signal", st2 & 0x7f, 31);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    i64 pid = p_fork();
    if (pid == 0) {
        passed = failed = 0;
        int rc = 1;
        if (p_drop_to_nobody() == 0) {
            syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
            i64 sb = p_clone(CLONE_NEWUSER | CLONE_NEWPID | CLONE_NEWNET);
            if (sb == 0) {
                passed = failed = 0;
                rc = sandbox();
                p_report();
                p_exit(rc);
            }
            int st = sb < 0 ? (int)sb : p_wait(sb);
            expect("sandbox_exit_0", st, 0);
            passed += a_shared()[0];
            failed += a_shared()[1];
            rc = failed ? 1 : 0;
        }
        p_report();
        p_exit(rc);
    }
    int st = p_wait(pid);
    expect("helper_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    return finish();
}
