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
/* fgetattrlist, BSD 228 (fd, attrlist, buffer, size, options): like getattrlist for an open
 * descriptor. The buffer starts with the total length; ATTR_CMN_RETURNED_ATTRS first when asked; the
 * name attribute is the name stored in the directory (as created), also when the descriptor was opened
 * with another case; ATTR_CMN_OBJTYPE VREG 1 / VDIR 2; ATTR_FILE_DATALENGTH the size, which follows
 * writes through the descriptor. A descriptor that is not open is EBADF (9). */
int test_main(void) {
    int fd = bsd3(SYS_open, "Mixed.Txt", O_CREAT | O_RDWR, 0644).ret;
    bsd3(SYS_write, fd, "0123456789", 10);
    struct attrlist_t al = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_NAME | ATTR_CMN_OBJTYPE, 0, 0, ATTR_FILE_DATALENGTH, 0};
    char buf[256];
    memset(buf, 0, sizeof buf);
    sysret r = bsd6(SYS_fgetattrlist, fd, &al, buf, sizeof buf, 0, 0);
    check("fgetattrlist returns 0", r.err == 0 && r.ret == 0, r.err);
    u32 *u = (u32 *)buf;
    check("the returned set: common 0x80000009, file 0x200", u[1] == 0x80000009u && u[4] == 0x200 && u[2] == 0 && u[3] == 0, u[1]);
    check("the name \"Mixed.Txt\" (10 bytes with the NUL)", u[7] == 10 && str_eq(buf + 24 + (i32)u[6], "Mixed.Txt"), u[7]);
    check("the object type VREG", u[8] == VREG, u[8]);
    i64 dl;
    char *after_name = buf + 24 + (i32)u[6] + u[7];
    (void)after_name;
    memcpy(&dl, buf + 36, 8);
    check("the data length is 10", dl == 10, dl);
    bsd3(SYS_write, fd, "abc", 3);
    memset(buf, 0, sizeof buf);
    bsd6(SYS_fgetattrlist, fd, &al, buf, sizeof buf, 0, 0);
    memcpy(&dl, buf + 36, 8);
    check("after a write the length follows: 13", dl == 13, dl);
    int other = bsd3(SYS_open, "MIXED.txt", O_RDONLY, 0).ret;
    memset(buf, 0, sizeof buf);
    r = bsd6(SYS_fgetattrlist, other, &al, buf, sizeof buf, 0, 0);
    check("a descriptor opened with another case still reports the stored name", r.err == 0 && str_eq(buf + 24 + (i32)u[6], "Mixed.Txt"), r.err);
    check_err("a descriptor that is not open is EBADF", bsd6(SYS_fgetattrlist, 99, &al, buf, sizeof buf, 0, 0), EBADF);
    bsd2(SYS_mkdir, "Dir", 0755);
    int d = bsd3(SYS_open, "dir", O_RDONLY, 0).ret;
    struct attrlist_t dal = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_NAME | ATTR_CMN_OBJTYPE, 0, 0, 0, 0};
    memset(buf, 0, sizeof buf);
    r = bsd6(SYS_fgetattrlist, d, &dal, buf, sizeof buf, 0, 0);
    check("a directory: stored name \"Dir\" and VDIR", r.err == 0 && str_eq(buf + 24 + (i32)u[6], "Dir") && u[8] == VDIR, r.err);
    return finish();
}
