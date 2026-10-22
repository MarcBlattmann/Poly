/* getdents: d_type, the last byte of each record, says what the entry
 * is: DT_DIR for ".", "..", and subdirectories, DT_REG for files (and
 * for a second hard link), DT_LNK for symbolic links (also dangling
 * ones), DT_FIFO for a FIFO, DT_SOCK for a bound Unix socket. Checked in
 * a directory under /tmp and one on the tmpfs at /dev/shm. */
#include "a_sys.h"

#define SYS_getdents 78
#define SYS_mknodat 259
#define SYS_socket 41
#define SYS_bind 49
#define AF_UNIX 1
#define SOCK_STREAM 1

struct g_dirent {
    u64 d_ino;
    u64 d_off;
    u16 d_reclen;
    char d_name[];
};

static char buf[4096] __attribute__((aligned(8)));

static const char *names[] = {".", "..", "file", "sub", "link", "dangle", "fifo", "sock", "hard"};
static const int types[] = {DT_DIR, DT_DIR, DT_REG, DT_DIR, DT_LNK, DT_LNK, DT_FIFO, DT_SOCK, DT_REG};
#define N 9

struct sockaddr_un_ {
    u16 sun_family;
    char sun_path[108];
};

static void run(const char *dir, const char *tag) {
    char p[160], c[64];
    struct sockaddr_un_ sa;
    expect(str_cat2(c, tag, "_setup_mkdir"), a_mkdir(dir, 0755), 0);
    expect(str_cat2(c, tag, "_setup_file"), a_make_file(str_cat2(p, dir, "/file"), "x", 0644), 0);
    expect(str_cat2(c, tag, "_setup_sub"), a_mkdir(str_cat2(p, dir, "/sub"), 0755), 0);
    expect(str_cat2(c, tag, "_setup_link"), syscall3(SYS_symlinkat, "file", AT_FDCWD, str_cat2(p, dir, "/link")), 0);
    expect(str_cat2(c, tag, "_setup_dangle"), syscall3(SYS_symlinkat, "nowhere", AT_FDCWD, str_cat2(p, dir, "/dangle")), 0);
    expect(str_cat2(c, tag, "_setup_fifo"), syscall4(SYS_mknodat, AT_FDCWD, str_cat2(p, dir, "/fifo"), S_IFIFO | 0644, 0), 0);
    i64 s = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    memset(&sa, 0, sizeof sa);
    sa.sun_family = AF_UNIX;
    str_cat2(sa.sun_path, dir, "/sock");
    expect(str_cat2(c, tag, "_setup_sock"), syscall3(SYS_bind, s, &sa, sizeof sa), 0);
    char q[160];
    expect(str_cat2(c, tag, "_setup_hard"), syscall5(SYS_linkat, AT_FDCWD, str_cat2(q, dir, "/file"), AT_FDCWD, str_cat2(p, dir, "/hard"), 0), 0);

    i64 fd = a_open(dir, O_RDONLY | O_DIRECTORY, 0);
    i64 n = syscall3(SYS_getdents, fd, buf, sizeof buf);
    check(str_cat2(c, tag, "_read"), n > 0, n);
    str_cat2(c, tag, "_type_");
    int seen[N] = {0}, got[N] = {0}, count = 0;
    for (i64 off = 0; off < n && count < 64;) {
        struct g_dirent *d = (struct g_dirent *)(buf + off);
        u8 type = (u8)buf[off + d->d_reclen - 1];
        count++;
        for (int i = 0; i < N; i++)
            if (str_eq(d->d_name, names[i])) {
                seen[i]++;
                got[i] = type;
            }
        if (d->d_reclen == 0) break;
        off += d->d_reclen;
    }
    /* Reported in a fixed order: the listing order differs between file systems. */
    for (int i = 0; i < N; i++) {
        char t[64];
        str_cat2(t, c, names[i]);
        expect(t, got[i], types[i]);
    }
    expect(str_cat2(c, tag, "_entry_count"), count, N);
    int once = 1;
    for (int i = 0; i < N; i++) once &= seen[i] == 1;
    check(str_cat2(c, tag, "_each_once"), once, 0);
    a_close(fd);
    a_close(s);

    const char *rm[] = {"/file", "/link", "/dangle", "/fifo", "/sock", "/hard"};
    for (int i = 0; i < 6; i++) expect(str_cat2(c, tag, "_cleanup"), a_unlink(str_cat2(p, dir, rm[i])), 0);
    expect(str_cat2(c, tag, "_cleanup_sub"), a_rmdir(str_cat2(p, dir, "/sub")), 0);
    expect(str_cat2(c, tag, "_cleanup_rmdir"), a_rmdir(dir), 0);
}

int test_main(void) {
    run("/tmp/a_getdents_types", "tmp");
    run("/dev/shm/a_getdents_types", "shm");
    return finish();
}
