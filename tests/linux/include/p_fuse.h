/*
 * Helpers for the FUSE tests (pk_fuse_*): a tiny FUSE server that runs in a
 * forked child and serves a small in-memory file system over /dev/fuse,
 * while the test process (the client) uses the mounted tree and inspects
 * what the kernel asked the server. Include after a_sys.h.
 *
 * Opcodes, flags and struct layouts are ABI facts from the public UAPI
 * header linux/fuse.h; the protocol is described in the kernel's public
 * documentation. Everything else is written for Poly; behaviour only.
 *
 * Roles: the test process is the only one that prints. The server child
 * never prints; it records every request in a shared log (struct
 * fz_shared) before it answers, so after a client call returns the log
 * already holds what the kernel sent for it. Requests that arrive
 * asynchronously (FORGET, BATCH_FORGET, INTERRUPT) are logged too: tests
 * count only the opcodes they care about.
 *
 * Served tree (node id = number; all owned by root, mtime 1700000000):
 *   1 /            directory 0755
 *   2 /hello       file 0644, "hello from fuse\n" (16 bytes)
 *   3 /dir         directory 0755
 *   4 /link        symlink to "hello"
 *   5 /big         file 0644, 300000 bytes, byte i = (i*7 + 3 + (i>>8)) & 255
 *   6 /dir/inner   file 0644, "inner\n"
 *   7 /many        directory with 300 entries "entry_0000" .. "entry_0299"
 *                  (all names of node 2; only for listing)
 * New nodes get the lowest free number from 8.
 */
#ifndef P_FUSE_H
#define P_FUSE_H

#include "a_sys.h"

#ifndef SYS_poll
#define SYS_poll 7
#endif
#ifndef SYS_mknod
#define SYS_mknod 133
#endif
#ifndef SYS_mount
#define SYS_mount 165
#endif
#ifndef SYS_umount2
#define SYS_umount2 166
#endif
#ifndef SYS_unshare
#define SYS_unshare 272
#endif
#define CLONE_NEWNS 0x20000 /* linux/sched.h */
/* linux/mount.h */
#define MS_RDONLY 1
#define MS_NOSUID 2
#define MS_NODEV 4
#define MS_NOEXEC 8
#define MS_REMOUNT 32
#define MS_REC 0x4000
#define MS_PRIVATE 0x40000
#define MNT_FORCE 1
#define MNT_DETACH 2
#define ENOTCONN 107
#define ECONNREFUSED 111
#define ENODEV_ 19
#define EFUSE_PERM 1
#define FUSE_SUPER_MAGIC 0x65735546 /* linux/magic.h */

/* linux/fuse.h: enum fuse_opcode */
enum {
    FZ_LOOKUP = 1, FZ_FORGET = 2, FZ_GETATTR = 3, FZ_SETATTR = 4, FZ_READLINK = 5, FZ_SYMLINK = 6,
    FZ_MKNOD = 8, FZ_MKDIR = 9, FZ_UNLINK = 10, FZ_RMDIR = 11, FZ_RENAME = 12, FZ_LINK = 13, FZ_OPEN = 14,
    FZ_READ = 15, FZ_WRITE = 16, FZ_STATFS = 17, FZ_RELEASE = 18, FZ_FSYNC = 20, FZ_SETXATTR = 21,
    FZ_GETXATTR = 22, FZ_LISTXATTR = 23, FZ_REMOVEXATTR = 24, FZ_FLUSH = 25, FZ_INIT = 26, FZ_OPENDIR = 27,
    FZ_READDIR = 28, FZ_RELEASEDIR = 29, FZ_FSYNCDIR = 30, FZ_GETLK = 31, FZ_SETLK = 32, FZ_SETLKW = 33,
    FZ_ACCESS = 34, FZ_CREATE = 35, FZ_INTERRUPT = 36, FZ_BMAP = 37, FZ_DESTROY = 38, FZ_IOCTL = 39,
    FZ_POLL = 40, FZ_NOTIFY_REPLY = 41, FZ_BATCH_FORGET = 42, FZ_FALLOCATE = 43, FZ_READDIRPLUS = 44,
    FZ_RENAME2 = 45, FZ_LSEEK = 46, FZ_COPY_FILE_RANGE = 47
};
/* linux/fuse.h: FATTR_*, init flags, open flags */
#define FZ_FATTR_MODE (1 << 0)
#define FZ_FATTR_UID (1 << 1)
#define FZ_FATTR_GID (1 << 2)
#define FZ_FATTR_SIZE (1 << 3)
#define FZ_FATTR_ATIME (1 << 4)
#define FZ_FATTR_MTIME (1 << 5)
#define FZ_FATTR_FH (1 << 6)
#define FZ_FATTR_ATIME_NOW (1 << 7)
#define FZ_FATTR_MTIME_NOW (1 << 8)
#define FZ_ASYNC_READ (1 << 0)
#define FZ_ATOMIC_O_TRUNC (1 << 3)
#define FZ_BIG_WRITES (1 << 5)
#define FZ_WRITEBACK_CACHE (1 << 16)
#define FZ_MAX_PAGES (1 << 22)

struct fz_in { /* struct fuse_in_header */
    u32 len, opcode;
    u64 unique, nodeid;
    u32 uid, gid, pid;
    u16 total_extlen, padding;
};
struct fz_out { /* struct fuse_out_header */
    u32 len;
    i32 error;
    u64 unique;
};
struct fz_attr { /* struct fuse_attr */
    u64 ino, size, blocks, atime, mtime, ctime;
    u32 atimensec, mtimensec, ctimensec, mode, nlink, uid, gid, rdev, blksize, flags;
};
struct fz_entry { /* struct fuse_entry_out */
    u64 nodeid, generation, entry_valid, attr_valid;
    u32 entry_valid_nsec, attr_valid_nsec;
    struct fz_attr attr;
};
struct fz_attrout { /* struct fuse_attr_out */
    u64 attr_valid;
    u32 attr_valid_nsec, dummy;
    struct fz_attr attr;
};
struct fz_init_in { /* struct fuse_init_in */
    u32 major, minor, max_readahead, flags, flags2, unused[11];
};
struct fz_init_out { /* struct fuse_init_out */
    u32 major, minor, max_readahead, flags;
    u16 max_background, congestion_threshold;
    u32 max_write, time_gran;
    u16 max_pages, map_alignment;
    u32 flags2, unused[7];
};
struct fz_open_in { u32 flags, open_flags; };
struct fz_open_out { u64 fh; u32 open_flags, padding; };
struct fz_create_in { u32 flags, mode, umask, open_flags; };
struct fz_mknod_in { u32 mode, rdev, umask, padding; };
struct fz_mkdir_in { u32 mode, umask; };
struct fz_read_in { u64 fh, offset; u32 size, read_flags; u64 lock_owner; u32 flags, padding; };
struct fz_write_in { u64 fh, offset; u32 size, write_flags; u64 lock_owner; u32 flags, padding; };
struct fz_write_out { u32 size, padding; };
struct fz_release_in { u64 fh; u32 flags, release_flags; u64 lock_owner; };
struct fz_flush_in { u64 fh; u32 unused, padding; u64 lock_owner; };
struct fz_fsync_in { u64 fh; u32 fsync_flags, padding; };
struct fz_getattr_in { u32 getattr_flags, dummy; u64 fh; };
struct fz_setattr_in {
    u32 valid, padding;
    u64 fh, size, lock_owner, atime, mtime, ctime;
    u32 atimensec, mtimensec, ctimensec, mode, unused4, uid, gid, unused5;
};
struct fz_rename_in { u64 newdir; };
struct fz_rename2_in { u64 newdir; u32 flags, padding; };
struct fz_link_in { u64 oldnodeid; };
struct fz_forget_in { u64 nlookup; };
struct fz_access_in { u32 mask, padding; };
struct fz_kstatfs {
    u64 blocks, bfree, bavail, files, ffree;
    u32 bsize, namelen, frsize, padding, spare[6];
};
struct fz_dirent { u64 ino, off; u32 namelen, type; };

/* ---- the shared log and the server's configuration ------------------- */

struct fz_rec {
    u32 op, len;
    u64 nodeid;
    u32 uid, gid;
    u64 a[4];
    u64 sum; /* WRITE: sum of the payload bytes weighted by position (fz_sum) */
    char name[24];
    char aux[24]; /* second name: symlink target, rename destination */
    i32 reply_err; /* error the server answered with (0 = ok, 1000 = none) */
};

#define FZ_RECS 600
struct fz_shared {
    /* configuration: set by the test before fz_start() (or at any time) */
    i32 init_major, init_minor, init_error; /* 0 = 7 / 31 / no error */
    u32 init_flags, max_write, max_pages;   /* 0 = none / 65536 / unset */
    i32 init_hold;                          /* 1: do not answer INIT until go */
    i32 init_junk_first;                    /* 1: before the real reply, write some bad replies */
    i32 init_reply_twice;                   /* 1: after the real reply, write it again */
    volatile i32 go;
    u64 entry_valid, attr_valid;            /* seconds in entry/attr replies */
    u64 neg_valid;                          /* nonzero: ENOENT lookups answer nodeid 0 with this validity */
    i32 fail_op, fail_err;                  /* answer this opcode with -fail_err */
    u32 write_short;                        /* nonzero: WRITE replies at most this many bytes */
    u32 read_short;                         /* nonzero: READ replies at most this many bytes */
    i32 open_flags_reply;                   /* open_flags field of OPEN/OPENDIR/CREATE replies */
    i32 skip_reply_op;                      /* never answer this opcode (the client hangs unless killed) */
    /* observations */
    volatile i32 n;
    volatile i32 init_count;
    volatile i32 destroy_seen;
    volatile i64 exit_read_err;
    volatile i64 reply_write_res; /* result of the last write() of a reply */
    volatile i64 junk_res[5];     /* results of the bad replies of init_junk_first */
    struct fz_in init_hdr;
    struct fz_init_in init_in;
    struct fz_rec rec[FZ_RECS];
};

static struct fz_shared *fz;
static i64 fz_fd = -1;
static i64 fz_child;
static const char *fz_dir;
/* A test may install a hook: called for every request but INIT, returns 1 if it answered. */
static int (*fz_hook)(const struct fz_in *h, const u8 *payload);

#define FZ_NONODE 0xffffffffu
#define FZ_NODES 24
#define FZ_ENTS 520
#define FZ_DATA 131072
#define FZ_BIGSIZE 300000UL
struct fz_node { u32 used, mode, nlink, rdev, uid, gid; u64 size, mtime; };
struct fz_ent { u32 used, dir, node; char name[40]; };
static struct fz_node fz_nodes[FZ_NODES];
static struct fz_ent fz_ents[FZ_ENTS];
static u8 fz_data[FZ_NODES][FZ_DATA];
static u8 fz_rbuf[1 << 19];
static u8 fz_wbuf[1 << 19];
static u8 fz_dbuf[1 << 18];
static u32 fz_fh_counter = 100;

static A_UNUSED u8 fz_bigbyte(u64 i) { return (u8)(i * 7 + 3 + (i >> 8)); }

static A_UNUSED void fz_strcpy_n(char *d, const char *s, size_t cap) {
    size_t i = 0;
    while (s[i] && i + 1 < cap) { d[i] = s[i]; i++; }
    d[i] = 0;
}

static A_UNUSED u32 fz_newnode(u32 mode, const char *text) {
    for (u32 i = 1; i < FZ_NODES; i++) {
        if (fz_nodes[i].used) continue;
        memset(&fz_nodes[i], 0, sizeof fz_nodes[i]);
        fz_nodes[i].used = 1;
        fz_nodes[i].mode = mode;
        fz_nodes[i].nlink = (mode & S_IFMT) == S_IFDIR ? 2 : 1;
        fz_nodes[i].mtime = 1700000000;
        if (text) {
            size_t l = str_len(text);
            memcpy(fz_data[i], text, l);
            fz_nodes[i].size = l;
        }
        return i;
    }
    return FZ_NONODE;
}

static A_UNUSED u32 fz_addent(u32 dir, const char *name, u32 node) {
    for (u32 i = 0; i < FZ_ENTS; i++) {
        if (fz_ents[i].used) continue;
        fz_ents[i].used = 1;
        fz_ents[i].dir = dir;
        fz_ents[i].node = node;
        fz_strcpy_n(fz_ents[i].name, name, sizeof fz_ents[i].name);
        return i;
    }
    return FZ_NONODE;
}

static A_UNUSED u32 fz_findent(u32 dir, const char *name) {
    for (u32 i = 0; i < FZ_ENTS; i++)
        if (fz_ents[i].used && fz_ents[i].dir == dir && str_eq(fz_ents[i].name, name)) return i;
    return FZ_NONODE;
}

static A_UNUSED void fz_tree_init(void) {
    fz_nodes[0].used = 1;                        /* node 0 is never used */
    fz_newnode(S_IFDIR | 0755, 0);               /* 1 */
    fz_newnode(S_IFREG | 0644, "hello from fuse\n"); /* 2 */
    fz_newnode(S_IFDIR | 0755, 0);               /* 3 */
    fz_newnode(S_IFLNK | 0777, "hello");         /* 4 */
    u32 big = fz_newnode(S_IFREG | 0644, 0);     /* 5 */
    fz_nodes[big].size = FZ_BIGSIZE;
    fz_newnode(S_IFREG | 0644, "inner\n");       /* 6 */
    fz_newnode(S_IFDIR | 0755, 0);               /* 7 */
    fz_addent(1, "hello", 2);
    fz_addent(1, "dir", 3);
    fz_addent(1, "link", 4);
    fz_addent(1, "big", 5);
    fz_addent(1, "many", 7);
    fz_addent(3, "inner", 6);
    for (int i = 0; i < 300; i++) {
        char nm[16] = "entry_0000";
        nm[6] = (char)('0' + i / 1000 % 10);
        nm[7] = (char)('0' + i / 100 % 10);
        nm[8] = (char)('0' + i / 10 % 10);
        nm[9] = (char)('0' + i % 10);
        fz_addent(7, nm, 2);
    }
}

static A_UNUSED void fz_fill_attr(struct fz_attr *a, u32 id) {
    struct fz_node *n = &fz_nodes[id];
    memset(a, 0, sizeof *a);
    a->ino = id;
    a->size = n->size;
    a->blocks = (n->size + 511) / 512;
    a->atime = a->mtime = a->ctime = n->mtime;
    a->mode = n->mode;
    a->nlink = n->nlink;
    a->uid = n->uid;
    a->gid = n->gid;
    a->rdev = n->rdev;
    a->blksize = 4096;
}

static A_UNUSED void fz_reply(u64 unique, int err, const void *body, u32 len) {
    struct fz_out *h = (struct fz_out *)fz_wbuf;
    h->len = 16 + len;
    h->error = err;
    h->unique = unique;
    if (len) memcpy(fz_wbuf + 16, body, len);
    fz->reply_write_res = syscall3(SYS_write, fz_fd, fz_wbuf, 16 + len);
}

static A_UNUSED void fz_entry_reply(u64 unique, u32 id, const struct fz_open_out *open) {
    struct {
        struct fz_entry e;
        struct fz_open_out o;
    } r;
    memset(&r, 0, sizeof r);
    r.e.nodeid = id;
    r.e.entry_valid = fz->entry_valid;
    r.e.attr_valid = fz->attr_valid;
    fz_fill_attr(&r.e.attr, id);
    if (open) r.o = *open;
    fz_reply(unique, 0, &r, open ? sizeof r : sizeof r.e);
}

static A_UNUSED void fz_attr_reply(u64 unique, u32 id) {
    struct fz_attrout o;
    memset(&o, 0, sizeof o);
    o.attr_valid = fz->attr_valid;
    fz_fill_attr(&o.attr, id);
    fz_reply(unique, 0, &o, sizeof o);
}

static A_UNUSED u32 fz_dtype(u32 mode) {
    switch (mode & S_IFMT) {
    case S_IFDIR: return DT_DIR;
    case S_IFLNK: return DT_LNK;
    case S_IFCHR: return DT_CHR;
    case S_IFBLK: return DT_BLK;
    case S_IFIFO: return DT_FIFO;
    default: return DT_REG;
    }
}

/* Directory listing: position p is 0 ".", 1 "..", then the entries in table order. */
static A_UNUSED void fz_readdir(const struct fz_in *h, const struct fz_read_in *r) {
    u8 out[16384];
    u32 used = 0, maxn = r->size < sizeof out ? r->size : (u32)sizeof out;
    u32 pos = 0;
    for (u32 pass = 0; pass < 2 + FZ_ENTS; pass++) {
        const char *name;
        u32 ino, type;
        if (pass == 0) { name = "."; ino = (u32)h->nodeid; type = DT_DIR; }
        else if (pass == 1) { name = ".."; ino = 1; type = DT_DIR; }
        else {
            u32 e = pass - 2;
            if (!fz_ents[e].used || fz_ents[e].dir != h->nodeid) continue;
            name = fz_ents[e].name;
            ino = fz_ents[e].node;
            type = fz_dtype(fz_nodes[ino].mode);
            if (h->nodeid == 7) ino = 1000 + e;
        }
        pos++;
        if (pos <= r->offset) continue;
        u32 nl = (u32)str_len(name);
        u32 sz = (u32)((sizeof(struct fz_dirent) + nl + 7) & ~7u);
        if (used + sz > maxn) break;
        struct fz_dirent *d = (struct fz_dirent *)(out + used);
        d->ino = ino;
        d->off = pos;
        d->namelen = nl;
        d->type = type;
        memset(out + used + sizeof *d, 0, sz - sizeof *d);
        memcpy(out + used + sizeof *d, name, nl);
        used += sz;
    }
    fz_reply(h->unique, 0, out, used);
}

/* Checksum of n bytes, as if they started at byte position `pos` of a stream. */
static A_UNUSED u64 fz_sum(const u8 *p, u64 n, u64 pos) {
    u64 s = 0;
    for (u64 i = 0; i < n; i++) s += (u64)p[i] * (((pos + i) % 251) + 1);
    return s;
}

static A_UNUSED void fz_rec_name(struct fz_rec *r, const u8 *p, u32 avail) {
    u32 i = 0;
    while (i < avail && i < sizeof r->name - 1 && p[i]) { r->name[i] = (char)p[i]; i++; }
    r->name[i] = 0;
    /* the string after the first NUL, if any */
    u32 j = 0;
    while (i < avail && p[i]) i++;
    i++;
    while (i < avail && j < sizeof r->aux - 1 && p[i]) r->aux[j++] = (char)p[i++];
    r->aux[j] = 0;
}

static A_UNUSED void fz_log(const struct fz_in *h, const u8 *pl) {
    struct fz_rec *r = &fz->rec[fz->n < FZ_RECS ? fz->n : FZ_RECS - 1];
    u32 avail = h->len > 40 ? h->len - 40 : 0;
    memset(r, 0, sizeof *r);
    r->op = h->opcode;
    r->len = h->len;
    r->nodeid = h->nodeid;
    r->uid = h->uid;
    r->gid = h->gid;
    r->reply_err = 1000;
    switch (h->opcode) {
    case FZ_READ: case FZ_READDIR: case FZ_READDIRPLUS: {
        const struct fz_read_in *x = (const void *)pl;
        r->a[0] = x->offset; r->a[1] = x->size; r->a[2] = x->read_flags; r->a[3] = x->fh;
        break;
    }
    case FZ_WRITE: {
        const struct fz_write_in *x = (const void *)pl;
        r->a[0] = x->offset; r->a[1] = x->size; r->a[2] = x->write_flags; r->a[3] = x->fh;
        r->sum = fz_sum(pl + sizeof *x, avail > sizeof *x ? avail - (u32)sizeof *x : 0, 0);
        break;
    }
    case FZ_OPEN: case FZ_OPENDIR: r->a[0] = ((const struct fz_open_in *)(const void *)pl)->flags; break;
    case FZ_CREATE: {
        const struct fz_create_in *x = (const void *)pl;
        r->a[0] = x->flags; r->a[1] = x->mode; r->a[2] = x->umask;
        fz_rec_name(r, pl + sizeof *x, avail - sizeof *x);
        break;
    }
    case FZ_MKDIR: {
        const struct fz_mkdir_in *x = (const void *)pl;
        r->a[0] = x->mode; r->a[1] = x->umask;
        fz_rec_name(r, pl + sizeof *x, avail - sizeof *x);
        break;
    }
    case FZ_MKNOD: {
        const struct fz_mknod_in *x = (const void *)pl;
        r->a[0] = x->mode; r->a[1] = x->rdev; r->a[2] = x->umask;
        fz_rec_name(r, pl + sizeof *x, avail - sizeof *x);
        break;
    }
    case FZ_SETATTR: {
        const struct fz_setattr_in *x = (const void *)pl;
        r->a[0] = x->valid; r->a[1] = x->size; r->a[2] = x->mode; r->a[3] = x->fh;
        break;
    }
    case FZ_RELEASE: case FZ_RELEASEDIR: {
        const struct fz_release_in *x = (const void *)pl;
        r->a[0] = x->fh; r->a[1] = x->flags; r->a[2] = x->release_flags;
        break;
    }
    case FZ_FLUSH: r->a[0] = ((const struct fz_flush_in *)(const void *)pl)->fh; break;
    case FZ_FSYNC: case FZ_FSYNCDIR: {
        const struct fz_fsync_in *x = (const void *)pl;
        r->a[0] = x->fh; r->a[1] = x->fsync_flags;
        break;
    }
    case FZ_GETATTR: {
        const struct fz_getattr_in *x = (const void *)pl;
        r->a[0] = x->getattr_flags; r->a[1] = x->fh;
        break;
    }
    case FZ_RENAME: r->a[0] = ((const struct fz_rename_in *)(const void *)pl)->newdir; fz_rec_name(r, pl + 8, avail - 8); break;
    case FZ_RENAME2: {
        const struct fz_rename2_in *x = (const void *)pl;
        r->a[0] = x->newdir; r->a[1] = x->flags;
        fz_rec_name(r, pl + sizeof *x, avail - sizeof *x);
        break;
    }
    case FZ_LINK: r->a[0] = ((const struct fz_link_in *)(const void *)pl)->oldnodeid; fz_rec_name(r, pl + 8, avail - 8); break;
    case FZ_FORGET: r->a[0] = ((const struct fz_forget_in *)(const void *)pl)->nlookup; break;
    case FZ_ACCESS: r->a[0] = ((const struct fz_access_in *)(const void *)pl)->mask; break;
    case FZ_LOOKUP: case FZ_UNLINK: case FZ_RMDIR: case FZ_SYMLINK: fz_rec_name(r, pl, avail); break;
    default: break;
    }
    r->reply_err = 1000;
    if (fz->n < FZ_RECS) fz->n = fz->n + 1;
}

static A_UNUSED void fz_setrec_err(int err) {
    if (fz->n > 0) fz->rec[fz->n - 1].reply_err = err;
}

/* Answer `err` and remember it in the log's last record. */
static A_UNUSED void fz_fail(const struct fz_in *h, int err) {
    fz_setrec_err(err);
    fz_reply(h->unique, -err, 0, 0);
}

static A_UNUSED void fz_handle_init(const struct fz_in *h, const u8 *pl) {
    const struct fz_init_in *in = (const void *)pl;
    fz->init_hdr = *h;
    fz->init_in = *in;
    fz->init_count = fz->init_count + 1;
    while (fz->init_hold && !fz->go) {
        struct a_timespec ts = {0, 2000000};
        syscall2(SYS_nanosleep, &ts, 0);
    }
    if (fz->init_junk_first) {
        /* Replies the kernel must refuse: too short, and one for an unknown request. */
        struct fz_out bad = {10, 0, h->unique};
        fz->junk_res[0] = syscall3(SYS_write, fz_fd, &bad, 10);
        struct fz_out other = {16, 0, h->unique + 2 * 1000000};
        fz->junk_res[1] = syscall3(SYS_write, fz_fd, &other, 16);
        struct fz_out longer = {64, 0, h->unique};
        fz->junk_res[2] = syscall3(SYS_write, fz_fd, &longer, 16);
    }
    if (fz->init_error) {
        fz_reply(h->unique, -fz->init_error, 0, 0);
        return;
    }
    struct fz_init_out o;
    memset(&o, 0, sizeof o);
    o.major = fz->init_major ? (u32)fz->init_major : 7;
    o.minor = fz->init_minor ? (u32)fz->init_minor : 31;
    o.max_readahead = in->max_readahead;
    o.flags = fz->init_flags;
    o.max_background = 16;
    o.congestion_threshold = 12;
    o.max_write = fz->max_write ? fz->max_write : 65536;
    o.time_gran = 1;
    o.max_pages = (u16)fz->max_pages;
    fz_reply(h->unique, 0, &o, sizeof o);
    if (fz->init_reply_twice) {
        struct fz_out again = {16, 0, h->unique};
        fz->junk_res[3] = syscall3(SYS_write, fz_fd, &again, 16);
        struct fz_out positive = {16, 5, h->unique + 2 * 1000000};
        fz->junk_res[4] = syscall3(SYS_write, fz_fd, &positive, 16);
    }
}

static A_UNUSED void fz_handle(const struct fz_in *h, const u8 *pl) {
    u32 avail = h->len > 40 ? h->len - 40 : 0;
    u32 id = (u32)h->nodeid;
    const char *nm = (const char *)pl;
    if (fz->fail_op && (u32)fz->fail_op == h->opcode && h->opcode != FZ_FORGET) {
        fz_fail(h, fz->fail_err);
        return;
    }
    if (fz->skip_reply_op && (u32)fz->skip_reply_op == h->opcode) return;
    if (fz_hook && fz_hook(h, pl)) return;
    switch (h->opcode) {
    case FZ_FORGET: case FZ_BATCH_FORGET: case FZ_INTERRUPT: return; /* no reply */
    case FZ_LOOKUP: {
        u32 e = id < FZ_NODES ? fz_findent(id, nm) : FZ_NONODE;
        if (e == FZ_NONODE) {
            if (fz->neg_valid) {
                struct fz_entry z;
                memset(&z, 0, sizeof z);
                z.entry_valid = fz->neg_valid;
                fz_setrec_err(0);
                fz_reply(h->unique, 0, &z, sizeof z);
            } else {
                fz_fail(h, ENOENT);
            }
            return;
        }
        fz_setrec_err(0);
        fz_entry_reply(h->unique, fz_ents[e].node, 0);
        return;
    }
    case FZ_GETATTR:
        if (id >= FZ_NODES || !fz_nodes[id].used) { fz_fail(h, ENOENT); return; }
        fz_setrec_err(0);
        fz_attr_reply(h->unique, id);
        return;
    case FZ_SETATTR: {
        const struct fz_setattr_in *s = (const void *)pl;
        if (id >= FZ_NODES || !fz_nodes[id].used) { fz_fail(h, ENOENT); return; }
        struct fz_node *n = &fz_nodes[id];
        if (s->valid & FZ_FATTR_MODE) n->mode = (n->mode & S_IFMT) | (s->mode & 07777);
        if (s->valid & FZ_FATTR_UID) n->uid = s->uid;
        if (s->valid & FZ_FATTR_GID) n->gid = s->gid;
        if (s->valid & FZ_FATTR_SIZE) {
            if (s->size > FZ_DATA && id != 5) { fz_fail(h, EFBIG); return; }
            if (s->size > n->size && id != 5) memset(fz_data[id] + n->size, 0, s->size - n->size);
            n->size = s->size;
        }
        if (s->valid & FZ_FATTR_MTIME) n->mtime = s->mtime;
        fz_setrec_err(0);
        fz_attr_reply(h->unique, id);
        return;
    }
    case FZ_READLINK:
        fz_setrec_err(0);
        fz_reply(h->unique, 0, fz_data[id], (u32)fz_nodes[id].size);
        return;
    case FZ_OPEN: case FZ_OPENDIR: {
        struct fz_open_out o;
        if (h->opcode == FZ_OPEN && id < FZ_NODES && (((const struct fz_open_in *)(const void *)pl)->flags & O_TRUNC)) {
            fz_nodes[id].size = 0; /* an OPEN carrying O_TRUNC truncates (FUSE_ATOMIC_O_TRUNC) */
        }
        memset(&o, 0, sizeof o);
        o.fh = ++fz_fh_counter;
        o.open_flags = (u32)fz->open_flags_reply;
        fz_setrec_err(0);
        fz_reply(h->unique, 0, &o, sizeof o);
        return;
    }
    case FZ_READ: {
        const struct fz_read_in *r = (const void *)pl;
        u64 size = fz_nodes[id].size, n = 0;
        u32 want = r->size;
        if (fz->read_short && want > fz->read_short) want = fz->read_short;
        if (r->offset < size) n = size - r->offset < want ? size - r->offset : want;
        if (n > sizeof fz_dbuf) n = sizeof fz_dbuf;
        if (id == 5) for (u64 i = 0; i < n; i++) fz_dbuf[i] = fz_bigbyte(r->offset + i);
        else if (n) memcpy(fz_dbuf, fz_data[id] + r->offset, n);
        fz_setrec_err(0);
        fz_reply(h->unique, 0, fz_dbuf, (u32)n);
        return;
    }
    case FZ_READDIR:
        fz_setrec_err(0);
        fz_readdir(h, (const void *)pl);
        return;
    case FZ_WRITE: {
        const struct fz_write_in *w = (const void *)pl;
        const u8 *d = pl + sizeof *w;
        u32 n = w->size < avail - sizeof *w ? w->size : avail - (u32)sizeof *w;
        if (w->offset + n > FZ_DATA) { fz_fail(h, EFBIG); return; }
        memcpy(fz_data[id] + w->offset, d, n);
        if (w->offset + n > fz_nodes[id].size) fz_nodes[id].size = w->offset + n;
        struct fz_write_out o = {fz->write_short && n > fz->write_short ? fz->write_short : n, 0};
        fz_setrec_err(0);
        fz_reply(h->unique, 0, &o, sizeof o);
        return;
    }
    case FZ_CREATE: {
        const struct fz_create_in *c = (const void *)pl;
        const char *name = (const char *)pl + sizeof *c;
        if (fz_findent(id, name) != FZ_NONODE) { fz_fail(h, EEXIST); return; }
        u32 nn = fz_newnode(S_IFREG | (c->mode & 07777 & ~c->umask), 0);
        if (nn == FZ_NONODE) { fz_fail(h, ENOSPC); return; }
        fz_addent(id, name, nn);
        struct fz_open_out o;
        memset(&o, 0, sizeof o);
        o.fh = ++fz_fh_counter;
        o.open_flags = (u32)fz->open_flags_reply;
        fz_setrec_err(0);
        fz_entry_reply(h->unique, nn, &o);
        return;
    }
    case FZ_MKNOD: case FZ_MKDIR: case FZ_SYMLINK: {
        const char *name;
        u32 mode;
        const char *text = 0;
        if (h->opcode == FZ_MKNOD) {
            const struct fz_mknod_in *m = (const void *)pl;
            name = (const char *)pl + sizeof *m;
            mode = m->mode & ~m->umask;
        } else if (h->opcode == FZ_MKDIR) {
            const struct fz_mkdir_in *m = (const void *)pl;
            name = (const char *)pl + sizeof *m;
            mode = S_IFDIR | (m->mode & 07777 & ~m->umask);
        } else {
            name = nm;
            text = nm + str_len(nm) + 1;
            mode = S_IFLNK | 0777;
        }
        if (fz_findent(id, name) != FZ_NONODE) { fz_fail(h, EEXIST); return; }
        u32 nn = fz_newnode(mode, text);
        if (nn == FZ_NONODE) { fz_fail(h, ENOSPC); return; }
        if (h->opcode == FZ_MKNOD) fz_nodes[nn].rdev = ((const struct fz_mknod_in *)(const void *)pl)->rdev;
        fz_addent(id, name, nn);
        fz_setrec_err(0);
        fz_entry_reply(h->unique, nn, 0);
        return;
    }
    case FZ_UNLINK: case FZ_RMDIR: {
        u32 e = fz_findent(id, nm);
        if (e == FZ_NONODE) { fz_fail(h, ENOENT); return; }
        struct fz_node *n = &fz_nodes[fz_ents[e].node];
        if (h->opcode == FZ_RMDIR) {
            for (u32 i = 0; i < FZ_ENTS; i++)
                if (fz_ents[i].used && fz_ents[i].dir == fz_ents[e].node) { fz_fail(h, ENOTEMPTY); return; }
        }
        if (n->nlink) n->nlink--;
        fz_ents[e].used = 0;
        fz_setrec_err(0);
        fz_reply(h->unique, 0, 0, 0);
        return;
    }
    case FZ_RENAME: case FZ_RENAME2: {
        u64 newdir;
        const char *oldn;
        if (h->opcode == FZ_RENAME) { newdir = ((const struct fz_rename_in *)(const void *)pl)->newdir; oldn = (const char *)pl + 8; }
        else { newdir = ((const struct fz_rename2_in *)(const void *)pl)->newdir; oldn = (const char *)pl + 16; }
        const char *newn = oldn + str_len(oldn) + 1;
        u32 e = fz_findent(id, oldn);
        if (e == FZ_NONODE) { fz_fail(h, ENOENT); return; }
        u32 t = fz_findent((u32)newdir, newn);
        if (h->opcode == FZ_RENAME2 && (((const struct fz_rename2_in *)(const void *)pl)->flags & 2)) {
            if (t == FZ_NONODE) { fz_fail(h, ENOENT); return; }
            u32 tmp = fz_ents[e].node;
            fz_ents[e].node = fz_ents[t].node;
            fz_ents[t].node = tmp;
            fz_setrec_err(0);
            fz_reply(h->unique, 0, 0, 0);
            return;
        }
        if (t != FZ_NONODE) fz_ents[t].used = 0;
        fz_ents[e].dir = (u32)newdir;
        fz_strcpy_n(fz_ents[e].name, newn, sizeof fz_ents[e].name);
        fz_setrec_err(0);
        fz_reply(h->unique, 0, 0, 0);
        return;
    }
    case FZ_LINK: {
        const struct fz_link_in *l = (const void *)pl;
        const char *name = (const char *)pl + sizeof *l;
        u32 target = (u32)l->oldnodeid;
        if (target >= FZ_NODES || !fz_nodes[target].used) { fz_fail(h, ENOENT); return; }
        fz_nodes[target].nlink++;
        fz_addent(id, name, target);
        fz_setrec_err(0);
        fz_entry_reply(h->unique, target, 0);
        return;
    }
    case FZ_STATFS: {
        struct fz_kstatfs s;
        memset(&s, 0, sizeof s);
        s.blocks = 1000; s.bfree = 500; s.bavail = 400; s.files = 100; s.ffree = 50;
        s.bsize = 4096; s.namelen = 255; s.frsize = 4096;
        fz_setrec_err(0);
        fz_reply(h->unique, 0, &s, sizeof s);
        return;
    }
    case FZ_DESTROY:
        fz->destroy_seen = 1;
        fz_setrec_err(0);
        fz_reply(h->unique, 0, 0, 0);
        return;
    case FZ_FLUSH: case FZ_RELEASE: case FZ_RELEASEDIR: case FZ_FSYNC: case FZ_FSYNCDIR: case FZ_ACCESS:
        fz_setrec_err(0);
        fz_reply(h->unique, 0, 0, 0);
        return;
    default:
        fz_fail(h, ENOSYS);
        return;
    }
}

/* The server's main loop (in the child). Ends when /dev/fuse says the connection is gone. */
static A_UNUSED __attribute__((noreturn)) void fz_serve(void) {
    syscall1(SYS_alarm, 25);
    fz_tree_init();
    for (;;) {
        i64 n = syscall3(SYS_read, fz_fd, fz_rbuf, sizeof fz_rbuf);
        if (n < 0) {
            if (n == -EINTR || n == -ENOENT) continue;
            fz->exit_read_err = n;
            exit_group(0);
        }
        if ((size_t)n < sizeof(struct fz_in)) continue;
        const struct fz_in *h = (const void *)fz_rbuf;
        if (h->opcode == FZ_INIT) {
            fz_log(h, fz_rbuf + 40);
            fz_handle_init(h, fz_rbuf + 40);
            fz_setrec_err(fz->init_error);
            continue;
        }
        fz_log(h, fz_rbuf + 40);
        fz_handle(h, fz_rbuf + 40);
    }
}

/* ---- client side ------------------------------------------------------ */

#define FZ_PATH_MAX 128
/* dir + "/" + rel into a static buffer (two alternating). */
static A_UNUSED const char *fz_path(const char *rel) {
    static char bufs[4][FZ_PATH_MAX];
    static int k;
    char *b = bufs[k++ & 3];
    char *p = str_cpy(b, fz_dir);
    if (*rel) { *p++ = '/'; str_cpy(p, rel); }
    return b;
}

/* Common set-up: timeout guard, SIGCHLD blocked, shared log, private mount
 * namespace, an empty mount point `dir`. Returns 0 or -errno of the failing step. */
static A_UNUSED i64 fz_setup(const char *dir) {
    syscall1(SYS_alarm, 25);
    a_block_sigchld();
    fz = (struct fz_shared *)syscall6(SYS_mmap, 0, sizeof *fz, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    if ((i64)fz < 0 && (i64)fz > -4096) return (i64)fz;
    fz_dir = dir;
    i64 r = syscall1(SYS_unshare, CLONE_NEWNS);
    if (r < 0) return r;
    r = syscall5(SYS_mount, 0, "/", 0, MS_REC | MS_PRIVATE, 0);
    if (r < 0) return r;
    a_rmdir(dir);
    return a_mkdir(dir, 0755);
}

/* Open /dev/fuse; if the node is missing make one (character device 10:229) in /tmp. */
static A_UNUSED i64 fz_open(void) {
    i64 fd = a_open("/dev/fuse", O_RDWR, 0);
    if (fd == -ENOENT) {
        syscall3(SYS_mknod, "/tmp/pk_fuse_node", S_IFCHR | 0666, (10 << 8) | 229);
        fd = a_open("/tmp/pk_fuse_node", O_RDWR, 0);
        a_unlink("/tmp/pk_fuse_node");
    }
    fz_fd = fd;
    return fd;
}

/* mount(2) of a "fuse" file system on `dir` with the usual options for descriptor `fd`; `extra`
 * (or NULL) is appended to the option string, `fstype` defaults to "fuse". */
static A_UNUSED i64 fz_mount_fd(i64 fd, const char *fstype, u64 flags, const char *extra) {
    char opts[160];
    char *p = str_cpy(opts, "fd=");
    a_utoa(p, (u64)fd);
    p += str_len(p);
    p = str_cpy(p, ",rootmode=40000,user_id=0,group_id=0");
    p += str_len(p);
    if (extra) { *p++ = ','; str_cpy(p, extra); }
    return syscall5(SYS_mount, "pk", fz_dir, fstype ? fstype : "fuse", flags, opts);
}
static A_UNUSED i64 fz_mount(const char *extra) { return fz_mount_fd(fz_fd, 0, MS_NOSUID | MS_NODEV, extra); }

/* Fork the server. The parent closes its descriptor so that the connection ends when the server does. */
static A_UNUSED i64 fz_start(void) {
    i64 pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) fz_serve();
    fz_child = pid;
    if (pid > 0) { a_close(fz_fd); fz_fd = -1; }
    return pid;
}

/* Kill the server (if still running) and collect it; returns its wait status. */
static A_UNUSED i32 fz_stop(void) {
    i32 st = -1;
    if (fz_child > 0) {
        syscall2(SYS_kill, fz_child, SIGKILL);
        syscall4(SYS_wait4, fz_child, &st, 0, 0);
        fz_child = 0;
    }
    return st;
}

/* Wait for the server to end by itself (it does after the connection is gone). */
static A_UNUSED i32 fz_wait(void) {
    i32 st = -1;
    if (fz_child > 0) {
        syscall4(SYS_wait4, fz_child, &st, 0, 0);
        fz_child = 0;
    }
    return st;
}

/* Unmount lazily and remove the mount point. */
static A_UNUSED void fz_cleanup(void) {
    fz_stop();
    syscall2(SYS_umount2, fz_dir, MNT_DETACH);
    a_rmdir(fz_dir);
}

/* Log queries. */
static A_UNUSED int fz_count(u32 op) {
    int c = 0;
    for (int i = 0; i < fz->n; i++) if (fz->rec[i].op == op) c++;
    return c;
}
/* The nth (from 0) record of an opcode, or NULL. */
static A_UNUSED const struct fz_rec *fz_nth(u32 op, int nth) {
    for (int i = 0; i < fz->n; i++)
        if (fz->rec[i].op == op && nth-- == 0) return &fz->rec[i];
    return 0;
}
static A_UNUSED const struct fz_rec *fz_last(u32 op) {
    for (int i = fz->n - 1; i >= 0; i--) if (fz->rec[i].op == op) return &fz->rec[i];
    return 0;
}
/* The last record of an opcode whose name is `name`. */
static A_UNUSED const struct fz_rec *fz_last_named(u32 op, const char *name) {
    for (int i = fz->n - 1; i >= 0; i--)
        if (fz->rec[i].op == op && str_eq(fz->rec[i].name, name)) return &fz->rec[i];
    return 0;
}
static A_UNUSED int fz_count_named(u32 op, const char *name) {
    int c = 0;
    for (int i = 0; i < fz->n; i++) if (fz->rec[i].op == op && str_eq(fz->rec[i].name, name)) c++;
    return c;
}
/* Forget what was logged so far (counts restart from here). */
static A_UNUSED int fz_mark(void) { return fz->n; }
static A_UNUSED int fz_count_since(int mark, u32 op) {
    int c = 0;
    for (int i = mark; i < fz->n; i++) if (fz->rec[i].op == op) c++;
    return c;
}
static A_UNUSED const struct fz_rec *fz_nth_since(int mark, u32 op, int nth) {
    for (int i = mark; i < fz->n; i++)
        if (fz->rec[i].op == op && nth-- == 0) return &fz->rec[i];
    return 0;
}

/* Wait (up to ms milliseconds) until the log holds at least `want` records of `op`; returns the count. */
static A_UNUSED int fz_wait_count(u32 op, int want, int ms) {
    for (int i = 0; i < ms / 5 + 1 && fz_count(op) < want; i++) {
        struct a_timespec ts = {0, 5000000};
        syscall2(SYS_nanosleep, &ts, 0);
    }
    return fz_count(op);
}

static A_UNUSED void fz_msleep(int ms) {
    struct a_timespec ts = {ms / 1000, (ms % 1000) * 1000000L};
    syscall2(SYS_nanosleep, &ts, 0);
}

/* Read a whole file into buf (at most cap bytes); returns the byte count or -errno. */
static A_UNUSED i64 fz_slurp(const char *path, void *buf, size_t cap) {
    i64 fd = a_open(path, O_RDONLY, 0);
    if (fd < 0) return fd;
    size_t got = 0;
    for (;;) {
        i64 n = a_read(fd, (char *)buf + got, cap - got);
        if (n < 0) { a_close(fd); return n; }
        if (n == 0) break;
        got += (size_t)n;
        if (got == cap) break;
    }
    a_close(fd);
    return (i64)got;
}

#endif
