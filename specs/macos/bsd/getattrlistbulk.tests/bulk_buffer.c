#include "poly_mtest.h"
#define SYS_rename 128
#define SYS_renameat 465
#define SYS_renameatx_np 488
#define SYS_link 9
#define SYS_ftruncate 201
#define SYS_getattrlist 220
#define SYS_setattrlist 221
#define SYS_fgetattrlist 228
#define SYS_getattrlistbulk 461
#define SYS_clonefileat 462
#define SYS_fclonefileat 517
#define SYS_getxattr 234
#define SYS_setxattr 236
#define RENAME_SWAP 2
#define RENAME_EXCL 4
#define CLONE_NOFOLLOW 1
#define ATTR_BIT_MAP_COUNT 5
#define ATTR_CMN_RETURNED_ATTRS 0x80000000u
#define ATTR_CMN_NAME 0x1u
#define ATTR_CMN_OBJTYPE 0x8u
#define ATTR_FILE_DATALENGTH 0x200u
#define VREG 1
#define VDIR 2
#define VLNK 5
struct attrlist_t { u16 bitmapcount; u16 reserved; u32 commonattr, volattr, dirattr, fileattr, forkattr; };
static __attribute__((unused)) void mkfile(const char *path, const char *text) {
    sysret f = bsd3(SYS_open, path, O_CREAT | O_TRUNC | O_RDWR, 0644);
    if (!f.err) { bsd3(SYS_write, f.ret, text, str_len(text)); bsd1(SYS_close, f.ret); }
}
static __attribute__((unused)) i64 size_of(const char *path) {
    struct poly_stat64 st;
    return bsd2(SYS_stat64, path, &st).err ? -1 : st.st_size;
}
static __attribute__((unused)) i64 ino_of(const char *path) {
    struct poly_stat64 st;
    return bsd2(SYS_stat64, path, &st).err ? -1 : (i64)st.st_ino;
}
static __attribute__((unused)) int exists(const char *path) {
    struct poly_stat64 st;
    return bsd2(SYS_lstat64, path, &st).err == 0;
}
static __attribute__((unused)) int content_is(const char *path, const char *text) {
    char buf[256];
    memset(buf, 0, sizeof buf);
    sysret f = bsd3(SYS_open, path, O_RDONLY, 0);
    if (f.err) return 0;
    sysret r = bsd3(SYS_read, f.ret, buf, sizeof buf - 1);
    bsd1(SYS_close, f.ret);
    return r.err == 0 && (size_t)r.ret == str_len(text) && mem_eq(buf, text, str_len(text));
}
/* getattrlistbulk, BSD 461: a small buffer. A buffer smaller than the first record is ERANGE (34); a
 * buffer for two records returns two and the next call continues where it stopped, until the call
 * returns 0; reading a directory of 100 files through 512-byte buffers lists every name exactly once. */
int test_main(void) {
    bsd2(SYS_mkdir, "d", 0755);
    mkfile("d/aa", "x");
    mkfile("d/bb", "x");
    mkfile("d/cc", "x");
    int d = bsd3(SYS_open, "d", O_RDONLY, 0).ret;
    struct attrlist_t a = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_NAME, 0, 0, 0, 0};
    static char buf[8192];
    sysret r = bsd6(SYS_getattrlistbulk, d, &a, buf, 20, 0, 0);
    check_err("a buffer smaller than one record is ERANGE", r, ERANGE);
    r = bsd6(SYS_getattrlistbulk, d, &a, buf, 39, 0, 0);
    value("a buffer of 39 bytes: errno", r.err);
    value("its result", r.ret);
    r = bsd6(SYS_getattrlistbulk, d, &a, buf, 40, 0, 0);
    check("a buffer of 40 bytes holds one record of 40 bytes", r.err == 0 && r.ret == 1 && *(u32 *)buf == 40, r.err ? r.err : r.ret);
    int d2 = bsd3(SYS_open, "d", O_RDONLY, 0).ret;
    int total = 0, calls = 0;
    for (;;) {
        r = bsd6(SYS_getattrlistbulk, d2, &a, buf, 90, 0, 0);
        if (r.err || r.ret == 0) break;
        total += r.ret;
        calls++;
    }
    check("three entries through 90-byte buffers: 3 entries in total", total == 3, total);
    check("in two calls (two records of 40 fit in 90, then one)", calls == 2, calls);
    bsd2(SYS_mkdir, "big", 0755);
    char name[16] = "big/f000";
    for (int i = 0; i < 100; i++) {
        name[5] = 'f';
        name[6] = (char)(48 + i / 10);
        name[7] = (char)(48 + i % 10);
        name[8] = 0;
        char path[16] = "big/";
        path[4] = 'f'; path[5] = name[6]; path[6] = name[7]; path[7] = 0;
        mkfile(path, "");
    }
    int bd = bsd3(SYS_open, "big", O_RDONLY, 0).ret;
    static unsigned char seen[100];
    int count = 0, dup = 0, bad = 0;
    for (;;) {
        r = bsd6(SYS_getattrlistbulk, bd, &a, buf, 512, 0, 0);
        if (r.err || r.ret == 0) break;
        char *p = buf;
        for (int i = 0; i < r.ret; i++) {
            u32 *u = (u32 *)p;
            char *nm = p + 24 + (i32)u[6];
            if (nm[0] == 'f' && nm[1] >= '0' && nm[1] <= '9' && nm[2] >= '0' && nm[2] <= '9') {
                int k = (nm[1] - '0') * 10 + (nm[2] - '0');
                if (seen[k]++) dup++;
            } else bad++;
            count++;
            p += u[0];
        }
    }
    int missing = 0;
    for (int i = 0; i < 100; i++) if (!seen[i]) missing++;
    check("100 files through 512-byte buffers: 100 records", count == 100 && bad == 0, count);
    check("each name once", dup == 0 && missing == 0, dup + missing);
    return finish();
}
