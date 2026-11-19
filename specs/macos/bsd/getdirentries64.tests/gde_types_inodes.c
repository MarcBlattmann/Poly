/* getdirentries64(2), BSD 344: d_type is DT_DIR (4), DT_REG (8), DT_LNK
 * (10), DT_FIFO (1); /dev/null is DT_CHR (2); d_ino is the inode lstat64 gives
 * for the same name; a name of 255 bytes has d_namlen 255 and d_reclen 280 (others
 * 32). Value: the type of /dev/fd (0, DT_UNKNOWN, on the oracle). */
#include "poly_mtest.h"

static __attribute__((unused)) char buf[65536];
static __attribute__((unused)) void name3(char *out, int n) {
    out[0] = 'f';
    out[1] = (char)('0' + n / 100);
    out[2] = (char)('0' + (n / 10) % 10);
    out[3] = (char)('0' + n % 10);
    out[4] = 0;
}

static int lookup(const char *dir, const char *name, struct poly_dirent64 *out) {
    sysret d = bsd3(SYS_open, dir, O_RDONLY, 0);
    i64 base;
    int found = 0;
    for (;;) {
        sysret g = bsd4(SYS_getdirentries64, d.ret, buf, sizeof buf, &base);
        if (g.err || g.ret == 0) break;
        for (i64 off = 0; off < g.ret;) {
            struct poly_dirent64 *e = (struct poly_dirent64 *)(buf + off);
            if (str_eq(e->d_name, name)) {
                memcpy(out, e, 24);
                memcpy(out->d_name, e->d_name, e->d_namlen + 1u);
                found = 1;
            }
            off += e->d_reclen;
        }
    }
    bsd1(SYS_close, d.ret);
    return found;
}

int test_main(void) {
    bsd2(SYS_mkdir, "t", 0755);
    bsd1(SYS_close, bsd3(SYS_open, "t/reg", O_CREAT | O_WRONLY, 0644).ret);
    bsd2(SYS_mkdir, "t/dir", 0755);
    bsd2(SYS_symlink, "reg", "t/lnk");
    check_ok("a FIFO (mknod)", bsd3(SYS_mknod, "t/fifo", S_IFIFO | 0644, 0));
    static char path[300], longname[256];
    path[0] = 't';
    path[1] = '/';
    for (int i = 0; i < 255; i++) longname[i] = 'n';
    longname[255] = 0;
    memcpy(path + 2, longname, 256);
    bsd1(SYS_close, bsd3(SYS_open, path, O_CREAT | O_WRONLY, 0644).ret);
    static struct poly_dirent64 e;
    struct poly_stat64 st;
    check("reg: DT_REG, its inode", lookup("t", "reg", &e) && e.d_type == DT_REG && bsd2(SYS_lstat64, "t/reg", &st).err == 0 && e.d_ino == st.st_ino, e.d_type);
    check("dir: DT_DIR, its inode", lookup("t", "dir", &e) && e.d_type == DT_DIR && bsd2(SYS_lstat64, "t/dir", &st).err == 0 && e.d_ino == st.st_ino, e.d_type);
    check("lnk: DT_LNK, its inode", lookup("t", "lnk", &e) && e.d_type == DT_LNK && bsd2(SYS_lstat64, "t/lnk", &st).err == 0 && e.d_ino == st.st_ino, e.d_type);
    check("fifo: DT_FIFO, its inode", lookup("t", "fifo", &e) && e.d_type == DT_FIFO && bsd2(SYS_lstat64, "t/fifo", &st).err == 0 && e.d_ino == st.st_ino, e.d_type);
    check("a name of 255 bytes: d_namlen 255, d_reclen 280", lookup("t", longname, &e) && e.d_namlen == 255 && e.d_reclen == 280, e.d_reclen);
    check("a short name: d_reclen 32", lookup("t", "reg", &e) && e.d_reclen == 32, e.d_reclen);
    check("/dev/null: DT_CHR", lookup("/dev", "null", &e) && e.d_type == DT_CHR, e.d_type);
    check("\".\" has the directory's own inode", lookup("t", ".", &e) && bsd2(SYS_lstat64, "t", &st).err == 0 && e.d_ino == st.st_ino, 0);
    if (lookup("/dev", "fd", &e)) value("the type of /dev/fd", e.d_type);
    return finish();
}
