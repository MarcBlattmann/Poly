/* getdirentries64(2), BSD 344: records of struct dirent (64-bit inodes,
 * <sys/dirent.h>) one after another, d_reclen bytes each (a multiple of
 * 8: 32 for short names), "." and ".." included, in no particular order
 * otherwise; a second call at the end returns 0. */
#include "poly_mtest.h"

static void make_tree(void) {
    bsd2(SYS_mkdir, "d", 0755);
    bsd1(SYS_close, bsd3(SYS_open, "d/a", O_CREAT | O_WRONLY, 0644).ret);
    bsd1(SYS_close, bsd3(SYS_open, "d/bb", O_CREAT | O_WRONLY, 0644).ret);
    bsd2(SYS_mkdir, "d/sub", 0755);
}

static char buf[8192];

int test_main(void) {
    make_tree();
    sysret d = bsd3(SYS_open, "d", O_RDONLY | O_DIRECTORY, 0);
    i64 base = -5;
    sysret g = bsd4(SYS_getdirentries64, d.ret, buf, sizeof buf, &base);
    check("5 entries of 32 bytes: 160", g.err == 0 && g.ret == 160, g.err ? g.err : g.ret);
    int seen = 0, bad = 0, n = 0;
    for (i64 off = 0; off < g.ret && n < 16; n++) {
        struct poly_dirent64 *e = (struct poly_dirent64 *)(buf + off);
        if (e->d_reclen == 0 || e->d_reclen % 8 || e->d_namlen != str_len(e->d_name) || e->d_ino == 0) bad++;
        if (str_eq(e->d_name, ".") && e->d_type == DT_DIR) seen |= 1;
        else if (str_eq(e->d_name, "..") && e->d_type == DT_DIR) seen |= 2;
        else if (str_eq(e->d_name, "a") && e->d_type == DT_REG) seen |= 4;
        else if (str_eq(e->d_name, "bb") && e->d_type == DT_REG) seen |= 8;
        else if (str_eq(e->d_name, "sub") && e->d_type == DT_DIR) seen |= 16;
        else bad++;
        if (e->d_reclen != 32) bad++;
        off += e->d_reclen ? e->d_reclen : g.ret;
    }
    check("., .., a, bb, sub with their types", seen == 31, seen);
    check("each 32 bytes, name length, an inode", bad == 0, bad);
    check("5 records", n == 5, n);
    sysret e = bsd4(SYS_getdirentries64, d.ret, buf, sizeof buf, &base);
    check("at the end: 0", e.err == 0 && e.ret == 0, e.err ? e.err : e.ret);
    return finish();
}
