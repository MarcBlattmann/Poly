/* getdirentries64(2), BSD 344: a directory of 150 files read through a
 * 1024-byte buffer takes several calls and gives every entry exactly once, "."
 * and ".." among them: 152 records of 32 bytes (4864 bytes), the last call 0; the
 * position (basep) after the first call is 0. Value: the number of calls. */
#include "poly_mtest.h"

static __attribute__((unused)) char buf[65536];
static __attribute__((unused)) void name3(char *out, int n) {
    out[0] = 'f';
    out[1] = (char)('0' + n / 100);
    out[2] = (char)('0' + (n / 10) % 10);
    out[3] = (char)('0' + n % 10);
    out[4] = 0;
}

int test_main(void) {
    bsd2(SYS_mkdir, "many", 0755);
    for (int i = 0; i < 150; i++) {
        char path[32] = "many/";
        name3(path + 5, i);
        bsd1(SYS_close, bsd3(SYS_open, path, O_CREAT | O_WRONLY, 0644).ret);
    }
    sysret d = bsd3(SYS_open, "many", O_RDONLY, 0);
    i64 base = -5, total = 0;
    int calls = 0, dots = 0, dup = 0, files = 0, bad = 0;
    int seen[150];
    memset(seen, 0, sizeof seen);
    for (;;) {
        sysret g = bsd4(SYS_getdirentries64, d.ret, buf, 1024, &base);
        if (calls == 0) check("position after the first call: 0", base == 0, base);
        calls++;
        if (g.err || g.ret == 0) break;
        total += g.ret;
        for (i64 off = 0; off < g.ret;) {
            struct poly_dirent64 *e = (struct poly_dirent64 *)(buf + off);
            if (e->d_reclen != 32) bad++;
            if (str_eq(e->d_name, ".") || str_eq(e->d_name, "..")) dots++;
            else if (e->d_name[0] == 'f' && e->d_namlen == 4) {
                int n = (e->d_name[1] - '0') * 100 + (e->d_name[2] - '0') * 10 + (e->d_name[3] - '0');
                if (n >= 0 && n < 150 && seen[n]++) dup++;
                files++;
            }
            off += e->d_reclen ? e->d_reclen : g.ret;
        }
    }
    int all = 1;
    for (int i = 0; i < 150; i++) all &= seen[i] == 1;
    check("every file once", all && dup == 0 && files == 150, files);
    check("\".\" and \"..\"", dots == 2, dots);
    check("152 records of 32 bytes: 4864 bytes in all", total == 4864 && bad == 0, total);
    check("it took more than one call", calls > 2, calls);
    value("calls (the last returned 0)", calls);
    return finish();
}
