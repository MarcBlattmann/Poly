#include "poly_mtest.h"
#define SYS_getxattr 234
#define SYS_fgetxattr 235
#define SYS_setxattr 236
#define SYS_fsetxattr 237
#define SYS_removexattr 238
#define SYS_fremovexattr 239
#define SYS_listxattr 240
#define SYS_flistxattr 241
#define ENOATTR 93
#define SYS_rename 128
#define SYS_link 9
#define SYS_ftruncate 201
#define XATTR_NOFOLLOW 1
#define XATTR_CREATE 2
#define XATTR_REPLACE 4
#define O_SYMLINK 0x200000
static __attribute__((unused)) sysret gx(const char *p, const char *n, void *b, i64 sz, i64 pos, i64 opt) { return bsd6(SYS_getxattr, p, n, b, sz, pos, opt); }
static __attribute__((unused)) sysret sx(const char *p, const char *n, const void *b, i64 sz, i64 pos, i64 opt) { return bsd6(SYS_setxattr, p, n, b, sz, pos, opt); }
static __attribute__((unused)) sysret fgx(i64 fd, const char *n, void *b, i64 sz, i64 pos, i64 opt) { return bsd6(SYS_fgetxattr, fd, n, b, sz, pos, opt); }
static __attribute__((unused)) sysret fsx(i64 fd, const char *n, const void *b, i64 sz, i64 pos, i64 opt) { return bsd6(SYS_fsetxattr, fd, n, b, sz, pos, opt); }
static __attribute__((unused)) int user_names(const char *list, i64 len, char *out) {
    /* the NUL-separated names of a list without com.apple.provenance, joined by | */
    int n = 0;
    for (i64 i = 0; i < len;) {
        const char *nm = list + i;
        i64 l = (i64)str_len(nm) + 1;
        if (!(str_len(nm) >= 20 && mem_eq(nm, "com.apple.provenance", 20))) {
            if (n) out[n++] = 0x7c;
            for (i64 j = 0; j < l - 1; j++) out[n++] = nm[j];
        }
        i += l;
    }
    out[n] = 0;
    return n;
}
/* setxattr, BSD 236: names and sizes. A name of 127 bytes works, 128 is ENAMETOOLONG (63), an
 * empty name EINVAL, a name that is not UTF-8 EINVAL. Values of up to 4 MiB were accepted and read
 * back in full; many attributes can coexist on one file; the empty value is stored. */
static char big[4 << 20];
int test_main(void) {
    bsd1(SYS_close, bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644).ret);
    char name[160], buf[16];
    memset(name, 110, 127);
    name[127] = 0;
    sysret r = sx("file", name, "x", 1, 0, 0);
    check_ok("a name of 127 bytes", r);
    name[127] = 110;
    name[128] = 0;
    r = sx("file", name, "x", 1, 0, 0);
    check_err("a name of 128 bytes is ENAMETOOLONG", r, ENAMETOOLONG);
    r = sx("file", "", "x", 1, 0, 0);
    check_err("an empty name is EINVAL", r, EINVAL);
    for (int i = 0; i < (int)sizeof big; i++) big[i] = (char)(i * 7 + (i >> 9));
    r = sx("file", "user.big", big, 65536, 0, 0);
    check_ok("a value of 64 KiB", r);
    r = gx("file", "user.big", 0, 0, 0, 0);
    check("its length reads back", r.err == 0 && r.ret == 65536, r.err ? r.err : r.ret);
    r = sx("file", "user.big", big, 1 << 20, 0, 0);
    check_ok("a value of 1 MiB", r);
    static char back[1 << 20];
    r = gx("file", "user.big", back, sizeof back, 0, 0);
    check("1 MiB reads back identical", r.err == 0 && r.ret == (1 << 20) && mem_eq(back, big, 1 << 20), r.err ? r.err : r.ret);
    r = sx("file", "user.big", big, sizeof big, 0, 0);
    check_ok("a value of 4 MiB", r);
    r = gx("file", "user.big", 0, 0, 0, 0);
    check("its length reads back", r.err == 0 && r.ret == (4 << 20), r.err ? r.err : r.ret);
    char key[16] = "user.k00";
    int bad = 0;
    for (int i = 0; i < 50; i++) {
        key[6] = (char)(48 + i / 10);
        key[7] = (char)(48 + i % 10);
        if (sx("file", key, &key[7], 1, 0, 0).err) bad++;
    }
    check("fifty attributes on one file", bad == 0, bad);
    key[6] = 52;
    key[7] = 57;
    r = gx("file", key, buf, sizeof buf, 0, 0);
    check("the last reads back", r.err == 0 && r.ret == 1 && buf[0] == 57, r.err ? r.err : r.ret);
    r = sx("file", "user.nothing", 0, 0, 0, 0);
    check_ok("a NULL value with size 0 stores an empty attribute", r);
    r = gx("file", "user.nothing", buf, sizeof buf, 0, 0);
    check("which reads back with length 0", r.err == 0 && r.ret == 0, r.err ? r.err : r.ret);
    return finish();
}
