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
/* getattrlistbulk, BSD 461 (dirfd, attrlist, buffer, size, options): reads the attributes of every
 * entry of a directory, as many records as fit, returning their number (0 when the directory is
 * exhausted). "." and ".." are not listed. A record is: u32 total length (a multiple of 8), the
 * returned attribute set (ATTR_CMN_RETURNED_ATTRS, 20 bytes: 5 u32), then the attributes in bitmap
 * order packed without alignment padding: ATTR_CMN_NAME as an attrreference (i32 offset from itself,
 * u32 length with the NUL) whose characters follow the fixed part, ATTR_CMN_OBJTYPE u32 (VREG 1, VDIR 2,
 * VLNK 5), ATTR_FILE_DATALENGTH 8 bytes (present only for files and links: a directory's record is
 * shorter and its returned set has no file attribute). Links are not followed: a link's object type is
 * VLNK and its data length the length of its target path. */
int test_main(void) {
    bsd2(SYS_mkdir, "d", 0755);
    mkfile("d/aa", "12345");
    mkfile("d/bb", "");
    bsd2(SYS_mkdir, "d/sub", 0755);
    bsd2(SYS_symlink, "aa", "d/ln");
    int d = bsd3(SYS_open, "d", O_RDONLY, 0).ret;
    struct attrlist_t a = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_NAME | ATTR_CMN_OBJTYPE, 0, 0, ATTR_FILE_DATALENGTH, 0};
    static char buf[4096];
    memset(buf, 0, sizeof buf);
    sysret r = bsd6(SYS_getattrlistbulk, d, &a, buf, sizeof buf, 0, 0);
    check("four records (no . and ..)", r.err == 0 && r.ret == 4, r.err ? r.err : r.ret);
    int seen = 0, lens_ok = 1, files_ok = 1;
    char *p = buf;
    for (int i = 0; i < r.ret; i++) {
        u32 *u = (u32 *)p;
        u32 len = u[0];
        char *name = p + 24 + (i32)u[6];
        u32 nlen = u[7];
        u32 type = u[8];
        if (len % 8) lens_ok = 0;
        if (str_eq(name, "aa")) {
            seen |= 1;
            i64 dl; memcpy(&dl, p + 36, 8);
            if (!(len == 48 && type == VREG && dl == 5 && nlen == 3 && u[1] == 0x80000009u && u[4] == 0x200)) files_ok = 0;
        } else if (str_eq(name, "bb")) {
            seen |= 2;
            i64 dl; memcpy(&dl, p + 36, 8);
            if (!(len == 48 && type == VREG && dl == 0)) files_ok = 0;
        } else if (str_eq(name, "sub")) {
            seen |= 4;
            if (!(len == 40 && type == VDIR && nlen == 4 && u[1] == 0x80000009u && u[4] == 0)) files_ok = 0;
        } else if (str_eq(name, "ln")) {
            seen |= 8;
            i64 dl; memcpy(&dl, p + 36, 8);
            if (!(len == 48 && type == VLNK && dl == 2)) files_ok = 0;
        }
        p += len;
    }
    check("all four names are there: aa, bb, sub, ln", seen == 15, seen);
    check("every record length is a multiple of 8", lens_ok, 0);
    check("file records: length 48 (name 3 bytes), VREG, data length, returned set 0x80000009 / file 0x200", files_ok, 0);
    value("the total bytes of the four records", p - buf);
    r = bsd6(SYS_getattrlistbulk, d, &a, buf, sizeof buf, 0, 0);
    check("a second call: the directory is exhausted, 0", r.err == 0 && r.ret == 0, r.err ? r.err : r.ret);
    struct attrlist_t n = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_NAME, 0, 0, 0, 0};
    bsd1(SYS_close, d);
    d = bsd3(SYS_open, "d", O_RDONLY, 0).ret;
    r = bsd6(SYS_getattrlistbulk, d, &n, buf, sizeof buf, 0, 0);
    check("a new descriptor starts again; names only: four records of 40 bytes", r.err == 0 && r.ret == 4 && *(u32 *)buf == 40 && *(u32 *)(buf + 40) == 40, r.err ? r.err : r.ret);
    return finish();
}
