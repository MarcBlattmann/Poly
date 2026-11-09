/*
 * Helpers for the mount, overlay-filesystem and mountinfo tests (the
 * `pk_overlayfs_*`, `pk_mount_*` and `pk_proc_mountinfo_*` programs).
 * Include after a_sys.h (which this header includes).
 *
 * Constants are ABI facts from the public Linux UAPI headers (linux/mount.h,
 * linux/fs.h, linux/sched.h, linux/stat.h, linux/xattr.h); the helpers are
 * written for Poly, no code is copied from anywhere.
 */
#ifndef P_MNT_H
#define P_MNT_H

#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_mknod 133
#define SYS_chroot 161
#define SYS_mount 165
#define SYS_umount2 166
#define SYS_setxattr 188
#define SYS_lsetxattr 189
#define SYS_getxattr 191
#define SYS_lgetxattr 192
#define SYS_listxattr 194
#define SYS_llistxattr 195
#define SYS_removexattr 197
#define SYS_lremovexattr 198
#define SYS_unshare 272
#define SYS_mknodat 259

/* errno values not in a_sys.h (asm-generic/errno.h) */
#define ENODATA 61

/* linux/mount.h */
#define MS_RDONLY 1
#define MS_NOSUID 2
#define MS_NODEV 4
#define MS_NOEXEC 8
#define MS_SYNCHRONOUS 16
#define MS_REMOUNT 32
#define MS_DIRSYNC 128
#define MS_NOSYMFOLLOW 256
#define MS_NOATIME 1024
#define MS_NODIRATIME 2048
#define MS_BIND 4096
#define MS_MOVE 8192
#define MS_REC 16384
#define MS_SILENT 32768
#define MS_UNBINDABLE (1 << 17)
#define MS_PRIVATE (1 << 18)
#define MS_SLAVE (1 << 19)
#define MS_SHARED (1 << 20)
#define MS_RELATIME (1 << 21)
#define MS_STRICTATIME (1 << 24)
#define MNT_FORCE 1
#define MNT_DETACH 2
#define MNT_EXPIRE 4
#define UMOUNT_NOFOLLOW 8

/* linux/statfs.h: f_flags */
#define ST_RDONLY 1
#define ST_NOSUID 2
#define ST_NODEV 4
#define ST_NOEXEC 8

#define CLONE_NEWNS 0x20000

/* linux/magic.h */
#define OVERLAYFS_SUPER_MAGIC 0x794c7630
#define TMPFS_MAGIC 0x01021994

static A_UNUSED i64 pm_mount(const char *src, const char *tgt, const char *fs, u64 flags, const void *data) {
    return syscall5(SYS_mount, src, tgt, fs, flags, data);
}
static A_UNUSED i64 pm_umount(const char *tgt, i64 flags) { return syscall2(SYS_umount2, tgt, flags); }

/* mkdir -p: every missing component, mode 0755; 0 or -errno. */
static A_UNUSED i64 pm_mkdirs(const char *path) {
    char buf[256];
    size_t n = str_len(path);
    if (n >= sizeof buf) return -ENAMETOOLONG;
    str_cpy(buf, path);
    for (size_t i = 1; i <= n; i++) {
        if (buf[i] == '/' || buf[i] == 0) {
            char c = buf[i];
            buf[i] = 0;
            i64 r = a_mkdir(buf, 0755);
            if (r < 0 && r != -EEXIST) return r;
            buf[i] = c;
        }
    }
    return 0;
}

/* Directory entry names of `path`, sorted, joined by ',' (no "." and ".."),
 * into out (NUL-terminated). Returns the number of names or -errno. */
static A_UNUSED i64 pm_list(const char *path, char *out, size_t cap) {
    i64 fd = a_open(path, O_RDONLY | O_DIRECTORY, 0);
    if (fd < 0) return fd;
    char names[48][40];
    int count = 0;
    char buf[2048];
    for (;;) {
        i64 n = syscall3(SYS_getdents64, fd, buf, sizeof buf);
        if (n <= 0) {
            a_close(fd);
            if (n < 0) return n;
            break;
        }
        for (i64 off = 0; off < n;) {
            struct a_dirent64 *d = (struct a_dirent64 *)(buf + off);
            off += d->d_reclen;
            if (str_eq(d->d_name, ".") || str_eq(d->d_name, "..")) continue;
            if (count < 48 && str_len(d->d_name) < 40) str_cpy(names[count++], d->d_name);
        }
    }
    for (int i = 1; i < count; i++) {
        char t[40];
        int j = i;
        str_cpy(t, names[i]);
        while (j > 0) {
            const char *a = names[j - 1], *b = t;
            while (*a && *a == *b) a++, b++;
            if ((unsigned char)*a <= (unsigned char)*b) break;
            str_cpy(names[j], names[j - 1]);
            j--;
        }
        str_cpy(names[j], t);
    }
    size_t o = 0;
    for (int i = 0; i < count; i++) {
        size_t l = str_len(names[i]);
        if (o + l + 2 >= cap) break;
        if (i) out[o++] = ',';
        str_cpy(out + o, names[i]);
        o += l;
    }
    out[o] = 0;
    return count;
}

/* The listing of `path` equals `want` (comma-joined sorted names)? */
static A_UNUSED void pm_expect_list(const char *name, const char *path, const char *want) {
    char out[512];
    i64 n = pm_list(path, out, sizeof out);
    check(name, n >= 0 && str_eq(out, want), n);
}

/* Read up to cap-1 bytes of a file as text. Returns the length or -errno. */
static A_UNUSED i64 pm_read_file(const char *path, char *buf, size_t cap) {
    i64 fd = a_open(path, O_RDONLY, 0);
    if (fd < 0) return fd;
    size_t got = 0;
    for (;;) {
        i64 n = a_read(fd, buf + got, cap - 1 - got);
        if (n <= 0) break;
        got += (size_t)n;
        if (got >= cap - 1) break;
    }
    a_close(fd);
    buf[got] = 0;
    return (i64)got;
}

/* The file's whole contents equal `want`? */
static A_UNUSED int pm_file_is(const char *path, const char *want) {
    char buf[1024];
    i64 n = pm_read_file(path, buf, sizeof buf);
    return n >= 0 && str_eq(buf, want);
}
static A_UNUSED void pm_expect_file(const char *name, const char *path, const char *want) {
    char buf[1024];
    i64 n = pm_read_file(path, buf, sizeof buf);
    check(name, n >= 0 && str_eq(buf, want), n);
}

/* 1 when lstat succeeds. */
static A_UNUSED int pm_exists(const char *path) {
    struct a_stat st;
    return a_lstat_path(path, &st) == 0;
}

static A_UNUSED i64 pm_mknod(const char *path, u32 mode, u32 major, u32 minor) {
    u64 dev = ((u64)(major & 0xfff) << 8) | (minor & 0xff) | ((u64)(minor & ~0xffu) << 12);
    return syscall3(SYS_mknod, path, mode, dev);
}

/* lstat mode (type + permission bits) or -errno. */
static A_UNUSED i64 pm_mode(const char *path) {
    struct a_stat st;
    i64 r = a_lstat_path(path, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

/* rm -rf below depth 6. */
static A_UNUSED void pm_rm_rf(const char *path) {
    struct a_stat st;
    if (a_lstat_path(path, &st) < 0) return;
    if ((st.st_mode & S_IFMT) != S_IFDIR) {
        a_unlink(path);
        return;
    }
    i64 fd = a_open(path, O_RDONLY | O_DIRECTORY, 0);
    if (fd >= 0) {
        char buf[1024];
        for (int round = 0; round < 8; round++) {
            i64 n = syscall3(SYS_getdents64, fd, buf, sizeof buf);
            if (n <= 0) break;
            for (i64 off = 0; off < n;) {
                struct a_dirent64 *d = (struct a_dirent64 *)(buf + off);
                off += d->d_reclen;
                if (str_eq(d->d_name, ".") || str_eq(d->d_name, "..")) continue;
                char child[256];
                str_cat2(child, path, "/");
                str_cpy(child + str_len(child), d->d_name);
                pm_rm_rf(child);
            }
            /* names removed: start over for the next batch */
            syscall3(SYS_lseek, fd, 0, SEEK_SET);
        }
        a_close(fd);
    }
    a_rmdir(path);
}

/* The calling process gets a mount namespace of its own whose mounts are
 * all private. 0 or the first failing -errno. */
static A_UNUSED i64 pm_private_ns(void) {
    i64 r = syscall1(SYS_unshare, CLONE_NEWNS);
    if (r < 0) return r;
    return pm_mount(0, "/", 0, MS_REC | MS_PRIVATE, 0);
}

/*
 * Run `body` in a child (parent suspended meanwhile) that is in a private
 * mount namespace of its own. The checks the child makes are added to the
 * parent's counts. Mounts made in the child vanish when it ends. Returns
 * the child's exit status (0 when the body returned 0).
 */
static A_UNUSED int pm_in_ns(int (*body)(void)) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        sh[0] = 0;
        sh[1] = 0;
        i64 r = pm_private_ns();
        if (r < 0) {
            check("child_private_ns", 0, r);
            sh[0] = passed;
            sh[1] = failed;
            syscall1(SYS_exit_group, 1);
        }
        int rc = body();
        sh[0] = passed;
        sh[1] = failed;
        syscall1(SYS_exit_group, rc);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    passed += sh[0];
    failed += sh[1];
    return A_WIFEXITED(status) ? A_WEXITSTATUS(status) : 255;
}

/* Mount a fresh tmpfs on `dir` (created if missing). */
static A_UNUSED i64 pm_tmpfs(const char *dir) {
    i64 r = pm_mkdirs(dir);
    if (r < 0) return r;
    return pm_mount("pktmpfs", dir, "tmpfs", 0, "mode=755");
}

/* Make `path` (text) from `a` + `b`. */
static A_UNUSED char *pm_path(char *out, const char *a, const char *b) { return str_cat2(out, a, b); }

/* Create file `base`+`name` with `text`. */
static A_UNUSED i64 pm_put(const char *base, const char *name, const char *text, i64 mode) {
    char p[256];
    pm_path(p, base, name);
    return a_make_file(p, text, mode);
}

/* Index of `needle` in `hay` or -1. */
static A_UNUSED int pm_find(const char *hay, const char *needle) {
    size_t n = str_len(needle);
    for (size_t i = 0; hay[i]; i++) {
        size_t j = 0;
        while (j < n && hay[i + j] == needle[j]) j++;
        if (j == n) return (int)i;
    }
    return -1;
}

/* Text of unsigned decimal into out. */
static A_UNUSED char *pm_num(char *out, u64 n) { return a_utoa(out, n); }


/* ---- /proc/self/mountinfo and /proc/self/mounts ----------------------- */

#define PM_MI_MAX (192 * 1024)
static A_UNUSED char pm_mi_text[PM_MI_MAX];

/* One parsed mountinfo line (fields as in proc(5); still escaped). */
struct pm_mi {
    char id[16], parent[16], dev[16];
    char root[200], mp[200], opts[160], optional[96], fstype[40], source[200], super[200];
    int nfields; /* whitespace separated fields of the line */
    int sep;     /* index of the "-" field, 0 when missing */
};

/* Load a mountinfo-like file into pm_mi_text. Returns its length or -errno. */
static A_UNUSED i64 pm_mi_load(const char *path) {
    i64 fd = a_open(path, O_RDONLY, 0);
    if (fd < 0) return fd;
    size_t got = 0;
    for (;;) {
        i64 n = a_read(fd, pm_mi_text + got, PM_MI_MAX - 1 - got);
        if (n <= 0) break;
        got += (size_t)n;
        if (got >= PM_MI_MAX - 1) break;
    }
    a_close(fd);
    pm_mi_text[got] = 0;
    return (i64)got;
}

static A_UNUSED void pm_copy_n(char *dst, const char *src, size_t cap) {
    size_t i = 0;
    while (src[i] && i + 1 < cap) {
        dst[i] = src[i];
        i++;
    }
    dst[i] = 0;
}

/* Parse the line starting at `line` (ending at '\n' or NUL) into *out.
 * Returns the pointer to the start of the next line. */
static A_UNUSED const char *pm_mi_parse(const char *line, struct pm_mi *out) {
    static char buf[4096];
    size_t n = 0;
    while (line[n] && line[n] != '\n' && n + 1 < sizeof buf) {
        buf[n] = line[n];
        n++;
    }
    buf[n] = 0;
    const char *next = line + n;
    while (*next && *next != '\n') next++;
    if (*next == '\n') next++;
    memset(out, 0, sizeof *out);
    char *tok[40];
    int nt = 0;
    for (size_t i = 0; i < n && nt < 40;) {
        while (i < n && buf[i] == ' ') buf[i++] = 0;
        if (i < n) {
            tok[nt++] = buf + i;
            while (i < n && buf[i] != ' ') i++;
        }
    }
    out->nfields = nt;
    if (nt < 6) return next;
    pm_copy_n(out->id, tok[0], sizeof out->id);
    pm_copy_n(out->parent, tok[1], sizeof out->parent);
    pm_copy_n(out->dev, tok[2], sizeof out->dev);
    pm_copy_n(out->root, tok[3], sizeof out->root);
    pm_copy_n(out->mp, tok[4], sizeof out->mp);
    pm_copy_n(out->opts, tok[5], sizeof out->opts);
    int sep = 0;
    for (int i = 6; i < nt; i++)
        if (str_eq(tok[i], "-")) {
            sep = i;
            break;
        }
    out->sep = sep;
    if (!sep) return next;
    char *o = out->optional;
    for (int i = 6; i < sep; i++) {
        if (i > 6 && o + 1 < out->optional + sizeof out->optional) *o++ = ' ';
        size_t l = str_len(tok[i]);
        if (o + l + 1 < out->optional + sizeof out->optional) {
            str_cpy(o, tok[i]);
            o += l;
        }
    }
    *o = 0;
    if (sep + 1 < nt) pm_copy_n(out->fstype, tok[sep + 1], sizeof out->fstype);
    if (sep + 2 < nt) pm_copy_n(out->source, tok[sep + 2], sizeof out->source);
    if (sep + 3 < nt) pm_copy_n(out->super, tok[sep + 3], sizeof out->super);
    return next;
}

/* Find the mountinfo line (in pm_mi_text) whose mount point field equals
 * `mp` (as written in the file, i.e. escaped). which = 0: the first such
 * line, -1: the last one (the mount on top). Returns 1 when found. */
static A_UNUSED int pm_mi_get(const char *mp, int which, struct pm_mi *out) {
    const char *p = pm_mi_text;
    int found = 0;
    struct pm_mi cur;
    while (*p) {
        p = pm_mi_parse(p, &cur);
        if (cur.nfields >= 6 && str_eq(cur.mp, mp)) {
            *out = cur;
            found = 1;
            if (which == 0) return 1;
        }
    }
    return found;
}

/* Number of lines of pm_mi_text and number that are malformed (fewer than 10
 * fields, no "-" at index >= 6, a non-numeric id or parent, a device field
 * without ':'). */
static A_UNUSED void pm_mi_count(int *lines, int *bad) {
    const char *p = pm_mi_text;
    struct pm_mi cur;
    *lines = *bad = 0;
    while (*p) {
        p = pm_mi_parse(p, &cur);
        (*lines)++;
        int ok = cur.nfields >= 10 && cur.sep >= 6;
        for (const char *c = cur.id; ok && *c; c++) ok = *c >= '0' && *c <= '9';
        for (const char *c = cur.parent; ok && *c; c++) ok = *c >= '0' && *c <= '9';
        if (ok) ok = cur.id[0] && cur.parent[0] && pm_find(cur.dev, ":") > 0;
        if (!ok) (*bad)++;
    }
}

/* Is `tok` one of the comma separated items of `list`? */
static A_UNUSED int pm_has_item(const char *list, const char *tok) {
    size_t n = str_len(tok);
    for (const char *p = list; *p;) {
        const char *q = p;
        while (*q && *q != ',') q++;
        if ((size_t)(q - p) == n) {
            size_t i = 0;
            while (i < n && p[i] == tok[i]) i++;
            if (i == n) return 1;
        }
        p = *q ? q + 1 : q;
    }
    return 0;
}

/* Does `s` start with `prefix`? */
static A_UNUSED int pm_starts(const char *s, const char *prefix) {
    while (*prefix && *s == *prefix) s++, prefix++;
    return *prefix == 0;
}

/* Text after "prefix" when s starts with it and the rest is only digits,
 * else NULL (the peer group number of "shared:N" or "master:N"). */
static A_UNUSED const char *pm_group(const char *s, const char *prefix) {
    if (!pm_starts(s, prefix)) return 0;
    s += str_len(prefix);
    if (!*s) return 0;
    for (const char *c = s; *c; c++)
        if (*c < '0' || *c > '9') return 0;
    return s;
}

/* One line of /proc/self/mounts: "source mountpoint fstype options 0 0". */
struct pm_mounts_line {
    char source[200], mp[200], fstype[40], opts[260], zero1[4], zero2[4];
    int nfields;
};
static A_UNUSED int pm_mounts_get(const char *mp, int which, struct pm_mounts_line *out) {
    const char *p = pm_mi_text;
    int found = 0;
    while (*p) {
        static char buf[4096];
        size_t n = 0;
        while (p[n] && p[n] != '\n' && n + 1 < sizeof buf) {
            buf[n] = p[n];
            n++;
        }
        buf[n] = 0;
        p += n;
        while (*p && *p != '\n') p++;
        if (*p == '\n') p++;
        char *tok[10];
        int nt = 0;
        for (size_t i = 0; i < n && nt < 10;) {
            while (i < n && buf[i] == ' ') buf[i++] = 0;
            if (i < n) {
                tok[nt++] = buf + i;
                while (i < n && buf[i] != ' ') i++;
            }
        }
        if (nt >= 6 && str_eq(tok[1], mp)) {
            memset(out, 0, sizeof *out);
            out->nfields = nt;
            pm_copy_n(out->source, tok[0], sizeof out->source);
            pm_copy_n(out->mp, tok[1], sizeof out->mp);
            pm_copy_n(out->fstype, tok[2], sizeof out->fstype);
            pm_copy_n(out->opts, tok[3], sizeof out->opts);
            pm_copy_n(out->zero1, tok[4], sizeof out->zero1);
            pm_copy_n(out->zero2, tok[5], sizeof out->zero2);
            found = 1;
            if (which == 0) return 1;
        }
    }
    return found;
}

#endif
