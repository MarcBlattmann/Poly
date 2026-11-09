/*
 * Helpers shared by the namespace and sandbox tests of the package-format
 * batch (`pk_userns_*`, `pk_pidns_*`, `pk_netns_*`, `pk_nsfs_*`,
 * `pk_sandbox_*`). Include after a_sys.h.
 *
 * Constants come from the public Linux UAPI headers (linux/sched.h,
 * linux/mount.h, linux/capability.h, linux/nsfs.h, linux/prctl.h,
 * asm/unistd_64.h); the helpers are written for Poly.
 */
#ifndef P_NS_H
#define P_NS_H

#include "a_sys.h"

/* asm/unistd_64.h */
#ifndef SYS_ioctl
#define SYS_ioctl 16
#endif
#define SYS_socket 41
#define SYS_connect 42
#define SYS_accept 43
#define SYS_sendto 44
#define SYS_recvfrom 45
#define SYS_bind 49
#define SYS_listen 50
#define SYS_getsockname 51
#define SYS_socketpair 53
#define SYS_setsockopt 54
#define SYS_mknod 133
#define SYS_pivot_root 155
#define SYS_prctl 157
#define SYS_chroot 161
#define SYS_mount 165
#define SYS_umount2 166
#define SYS_sethostname 170
#define SYS_setdomainname 171
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_getresuid 118
#define SYS_setresgid 119
#define SYS_getresgid 120
#define SYS_capget 125
#define SYS_capset 126
#define SYS_unshare 272
#define SYS_setns 308
#define SYS_eventfd2 290
#define SYS_pidfd_open 434

/* asm-generic/errno.h */
#ifndef EADDRNOTAVAIL
#define EADDRNOTAVAIL 99
#endif
#ifndef ENETUNREACH
#define ENETUNREACH 101
#endif
#ifndef ECONNREFUSED
#define ECONNREFUSED 111
#endif
#ifndef EAFNOSUPPORT
#define EAFNOSUPPORT 97
#endif
#ifndef EADDRINUSE
#define EADDRINUSE 98
#endif
#ifndef EPROTONOSUPPORT
#define EPROTONOSUPPORT 93
#endif
#ifndef ENOTCONN
#define ENOTCONN 107
#endif
#ifndef ENOTSOCK
#define ENOTSOCK 88
#endif

/* linux/sched.h */
#define CLONE_NEWTIME 0x80
#define CLONE_NEWNS 0x20000
#define CLONE_NEWCGROUP 0x02000000
#define CLONE_NEWUTS 0x04000000
#define CLONE_NEWIPC 0x08000000
#define CLONE_NEWUSER 0x10000000
#define CLONE_NEWPID 0x20000000
#define CLONE_NEWNET 0x40000000

/* linux/mount.h */
#define MS_RDONLY 1
#define MS_NOSUID 2
#define MS_NODEV 4
#define MS_NOEXEC 8
#define MS_REMOUNT 32
#define MS_BIND 4096
#define MS_REC 16384
#define MS_SILENT 32768
#define MS_UNBINDABLE (1 << 17)
#define MS_PRIVATE (1 << 18)
#define MS_SLAVE (1 << 19)
#define MS_SHARED (1 << 20)
#define MS_RELATIME (1 << 21)
#define MS_NOSYMFOLLOW (1 << 8)
#define MNT_DETACH 2

/* linux/prctl.h */
#define PR_SET_PDEATHSIG 1
#define PR_GET_PDEATHSIG 2
#define PR_SET_DUMPABLE 4
#define PR_SET_CHILD_SUBREAPER 36
#define PR_GET_CHILD_SUBREAPER 37
#define PR_SET_NO_NEW_PRIVS 38
#define PR_GET_NO_NEW_PRIVS 39

/* linux/nsfs.h: _IO(0xb7, n) */
#define NS_GET_USERNS 0xb701
#define NS_GET_PARENT 0xb702
#define NS_GET_NSTYPE 0xb703
#define NS_GET_OWNER_UID 0xb704

/* socket constants: sys/socket.h values on x86-64 Linux */
#define AF_UNIX 1
#define AF_INET 2
#define AF_NETLINK 16
#define AF_PACKET 17
#define SOCK_STREAM 1
#define SOCK_DGRAM 2
#define SOCK_RAW 3
#define SOCK_CLOEXEC 02000000

#define P_NULL ((void *)0)

static A_UNUSED i64 p_write_file(const char *path, const char *s) {
    i64 fd = a_open(path, O_WRONLY, 0);
    if (fd < 0) return fd;
    i64 n = a_write(fd, s, str_len(s));
    a_close(fd);
    return n;
}

/* Read a whole (small) file into buf, NUL-terminated; length or -errno. */
static A_UNUSED i64 p_read_file(const char *path, char *buf, size_t cap) {
    i64 fd = a_open(path, O_RDONLY, 0);
    if (fd < 0) return fd;
    size_t n = 0;
    for (;;) {
        i64 r = a_read(fd, buf + n, cap - 1 - n);
        if (r <= 0) break;
        n += (size_t)r;
        if (n >= cap - 1) break;
    }
    a_close(fd);
    buf[n] = 0;
    return (i64)n;
}

/* Position of needle in hay, NULL if none. */
static A_UNUSED const char *p_find(const char *hay, const char *needle) {
    size_t nl = str_len(needle);
    for (; *hay; hay++) {
        size_t i = 0;
        while (i < nl && hay[i] == needle[i]) i++;
        if (i == nl) return hay;
    }
    return P_NULL;
}

static A_UNUSED int p_starts(const char *s, const char *prefix) {
    while (*prefix) if (*s++ != *prefix++) return 0;
    return 1;
}

/* Decimal text -> number (leading blanks skipped). */
static A_UNUSED u64 p_num(const char *s) {
    while (*s == ' ' || *s == '\t') s++;
    return a_atou(s);
}

/* Value of the "Key:" line of a /proc status file (pointer after the colon
 * and blanks), NULL if the line does not exist. */
static A_UNUSED const char *p_status_field(const char *text, const char *key) {
    size_t kl = str_len(key);
    for (const char *p = text; *p;) {
        if (p_starts(p, key) && p[kl] == ':') {
            p += kl + 1;
            while (*p == ' ' || *p == '\t') p++;
            return p;
        }
        while (*p && *p != '\n') p++;
        if (*p) p++;
    }
    return P_NULL;
}

/* Number of fields (separated by blanks or tabs) of the line starting at p. */
static A_UNUSED int p_fields(const char *p) {
    int n = 0;
    while (*p && *p != '\n') {
        while (*p == ' ' || *p == '\t') p++;
        if (*p && *p != '\n') n++;
        while (*p && *p != '\n' && *p != ' ' && *p != '\t') p++;
    }
    return n;
}

/* Last field of the line at p, as a number. */
static A_UNUSED u64 p_last_field(const char *p) {
    const char *last = p;
    while (*p && *p != '\n') {
        if (*p == ' ' || *p == '\t') last = p + 1;
        p++;
    }
    return a_atou(last);
}

static A_UNUSED i64 p_mount(const char *src, const char *tgt, const char *fs, u64 flags, const char *data) {
    return syscall5(SYS_mount, src, tgt, fs, flags, data);
}

static A_UNUSED i64 p_chdir(const char *p) { return syscall1(SYS_chdir, p); }

/* readlink into buf (NUL-terminated); length or -errno. */
static A_UNUSED i64 p_readlink(const char *path, char *buf, size_t cap) {
    i64 n = syscall3(SYS_readlink, path, buf, cap - 1);
    if (n >= 0) buf[n] = 0;
    return n;
}

/* Plain fork: the child gets a copy of memory and runs at once with the
 * parent (no suspension). 0 in the child. */
static A_UNUSED i64 p_fork(void) { return syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0); }

/* clone with extra flags, child on a copy of the stack (no CLONE_VM). */
static A_UNUSED i64 p_clone(u64 flags) { return syscall5(SYS_clone, flags | SIGCHLD, 0, 0, 0, 0); }

static A_UNUSED int p_wait(i64 pid) {
    i32 st = -1;
    i64 r = syscall4(SYS_wait4, pid, &st, 0, 0);
    return r == pid ? st : -1000 - (int)(r < 0 ? -r : r);
}

static __attribute__((noreturn, unused)) void p_exit(int code) {
    for (;;) syscall1(SYS_exit_group, code);
}

/* Wait status of a normal exit with `code`. */
#define P_EXITED(code) ((code) << 8)

/*
 * Run `fn` in a child started with `flags` (clone flags added to SIGCHLD),
 * suspended parent not used: the parent waits. The child reports its
 * passed/failed counters through a shared page (set P_REPORT at its end);
 * the parent adds them and checks the child's exit status is 0. Returns the
 * wait status.
 */
static A_UNUSED void p_report(void) {
    a_shared()[0] = passed;
    a_shared()[1] = failed;
}

static A_UNUSED int p_run(const char *name, u64 flags, int (*fn)(void)) {
    i32 p = passed, f = failed;
    a_shared()[0] = 0;
    a_shared()[1] = 0;
    i64 pid = p_clone(flags);
    if (pid == 0) {
        passed = failed = 0;
        int rc = fn();
        p_report();
        p_exit(rc);
    }
    int st = pid < 0 ? (int)pid : p_wait(pid);
    passed = p + a_shared()[0];
    failed = f + a_shared()[1];
    expect(name, st, 0);
    return st;
}

/* Hand-shake of a sandbox helper: the child (started with clone `flags`)
 * runs `pre` (may be NULL; e.g. change ids, unshare), tells the parent it
 * is ready, and waits; the parent runs `setup(child_pid)` (e.g. writes the
 * child's uid_map; may be NULL) and releases it; the child then runs `fn`.
 * Counters are summed as for p_run; the child must exit 0. */
static A_UNUSED int p_run_staged(const char *name, u64 flags, int (*pre)(void), int (*fn)(void),
                                 void (*setup)(i64 pid)) {
    i32 p = passed, f = failed;
    i32 go[2], ready[2];
    a_shared()[0] = 0;
    a_shared()[1] = 0;
    if (syscall2(SYS_pipe2, go, 0) < 0 || syscall2(SYS_pipe2, ready, 0) < 0) {
        check(name, 0, -1);
        return -1;
    }
    i64 pid = p_clone(flags);
    if (pid == 0) {
        a_close(go[1]);
        a_close(ready[0]);
        passed = failed = 0;
        int rc = pre ? pre() : 0;
        a_write(ready[1], "r", 1);
        a_close(ready[1]);
        char c = 0;
        i64 got = a_read(go[0], &c, 1);
        a_close(go[0]);
        if (rc == 0) rc = (got == 1 && c == 'g') ? fn() : 99;
        p_report();
        p_exit(rc);
    }
    a_close(go[0]);
    a_close(ready[1]);
    char c = 0;
    a_read(ready[0], &c, 1);
    a_close(ready[0]);
    if (pid > 0 && setup) setup(pid);
    a_write(go[1], "g", 1);
    a_close(go[1]);
    int st = pid < 0 ? (int)pid : p_wait(pid);
    passed = p + a_shared()[0];
    failed = f + a_shared()[1];
    expect(name, st, 0);
    return st;
}

static A_UNUSED int p_run_with(const char *name, u64 flags, int (*fn)(void), void (*setup)(i64 pid)) {
    return p_run_staged(name, flags, 0, fn, setup);
}
/* Fork a child (new session not needed) and give it the ids of nobody. */
static A_UNUSED i64 p_drop_to_nobody(void) {
    i64 r = syscall2(SYS_setgroups, 0, 0);
    if (r < 0) return r;
    r = syscall3(SYS_setresgid, 65534, 65534, 65534);
    if (r < 0) return r;
    return syscall3(SYS_setresuid, 65534, 65534, 65534);
}

/* Write "0 <outer> 1\n"-style maps: inside outside count. */
static A_UNUSED void p_map_text(char *out, u64 inside, u64 outside, u64 count) {
    char n[24];
    char *p = out;
    p = str_cpy(p, a_utoa(n, inside));
    *p++ = ' ';
    p = str_cpy(p, a_utoa(n, outside));
    *p++ = ' ';
    p = str_cpy(p, a_utoa(n, count));
    *p++ = '\n';
    *p = 0;
}

/* Path "/proc/<pid>/<what>" into buf. */
static A_UNUSED char *p_proc_path(char *buf, i64 pid, const char *what) {
    char n[24];
    char *p = str_cpy(buf, "/proc/");
    p = str_cpy(p, a_utoa(n, (u64)pid));
    *p++ = '/';
    str_cpy(p, what);
    return buf;
}

/* Write the uid_map, setgroups ("deny") and gid_map of process `pid`
 * ("self" if pid <= 0) mapping inside 0 to outer ids. Returns 0 or the
 * first -errno. */
static A_UNUSED i64 p_write_maps(i64 pid, u64 uid_out, u64 gid_out, int deny_setgroups) {
    char path[48], map[64];
    i64 r;
    if (pid <= 0) str_cpy(path, "/proc/self/setgroups");
    else p_proc_path(path, pid, "setgroups");
    if (deny_setgroups) {
        r = p_write_file(path, "deny");
        if (r < 0) return r;
    }
    p_map_text(map, 0, uid_out, 1);
    if (pid <= 0) str_cpy(path, "/proc/self/uid_map");
    else p_proc_path(path, pid, "uid_map");
    r = p_write_file(path, map);
    if (r < 0) return r;
    p_map_text(map, 0, gid_out, 1);
    if (pid <= 0) str_cpy(path, "/proc/self/gid_map");
    else p_proc_path(path, pid, "gid_map");
    r = p_write_file(path, map);
    return r < 0 ? r : 0;
}

/* A pipe pair for parent/child hand-shakes (cloexec off). */
static A_UNUSED i64 p_pipe(i32 fds[2]) { return syscall2(SYS_pipe2, fds, 0); }

/* Block until one byte arrives (returns it) or the pipe is closed (-1). */
static A_UNUSED int p_wait_byte(i64 fd) {
    char c;
    return a_read(fd, &c, 1) == 1 ? (unsigned char)c : -1;
}
static A_UNUSED void p_send_byte(i64 fd, char c) { a_write(fd, &c, 1); }

/* Number of entries of directory `path` whose name satisfies `numeric`
 * (all digits) -> *num, and all entries -> *all (including . and ..).
 * Returns 0 or -errno. */
static A_UNUSED i64 p_count_dir(const char *path, int *num, int *all) {
    static char buf[4096];
    i64 fd = a_open(path, O_RDONLY | O_DIRECTORY, 0);
    if (fd < 0) return fd;
    *num = *all = 0;
    for (;;) {
        i64 n = syscall3(SYS_getdents64, fd, buf, sizeof buf);
        if (n <= 0) {
            a_close(fd);
            return n < 0 ? n : 0;
        }
        for (i64 off = 0; off < n;) {
            struct a_dirent64 *d = (struct a_dirent64 *)(buf + off);
            (*all)++;
            int digits = d->d_name[0] != 0;
            for (const char *c = d->d_name; *c; c++)
                if (*c < '0' || *c > '9') digits = 0;
            if (digits) (*num)++;
            off += d->d_reclen;
        }
    }
}

/* True if directory `path` has an entry called `name`. */
static A_UNUSED int p_dir_has(const char *path, const char *name) {
    static char buf[4096];
    i64 fd = a_open(path, O_RDONLY | O_DIRECTORY, 0);
    if (fd < 0) return 0;
    int found = 0;
    for (;;) {
        i64 n = syscall3(SYS_getdents64, fd, buf, sizeof buf);
        if (n <= 0) break;
        for (i64 off = 0; off < n;) {
            struct a_dirent64 *d = (struct a_dirent64 *)(buf + off);
            if (str_eq(d->d_name, name)) found = 1;
            off += d->d_reclen;
        }
    }
    a_close(fd);
    return found;
}

/* The inode number of a namespace file. */
static A_UNUSED i64 p_ns_ino(const char *path) {
    struct a_stat st;
    i64 r = a_stat_path(path, &st);
    return r < 0 ? r : (i64)st.st_ino;
}


/* rt_sigaction with the restorer the kernel needs (x86-64 has no default). */
void p_restorer(void);
__asm__(".text\n"
        ".global p_restorer\n"
        "p_restorer:\n"
        "    mov $15, %eax\n"
        "    syscall\n");
static A_UNUSED i64 p_handler(int sig, void (*fn)(int)) {
    struct a_sigaction sa = {(u64)fn, SA_RESTORER, (u64)p_restorer, 0};
    return syscall4(SYS_rt_sigaction, sig, &sa, 0, 8);
}

#endif
