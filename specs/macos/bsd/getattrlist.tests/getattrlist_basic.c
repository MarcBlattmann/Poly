/* getattrlist, BSD 220 (path, attrlist, buffer, size, options): the buffer
 * starts with the total length (u32); then the requested attributes in
 * the order of the bitmap, common before file: ATTR_CMN_NAME is an
 * attrreference {offset from itself, length with the NUL} pointing at the
 * name after the fixed part, ATTR_CMN_OBJTYPE a u32 (VREG 1, VDIR 2, VLNK
 * 5), ATTR_FILE_DATALENGTH an 8-byte size, aligned to 8. With
 * ATTR_CMN_RETURNED_ATTRS the first attribute is the set of attributes
 * returned. */
#include "poly_mtest.h"

/* <sys/attr.h> */
struct attrlist_t {
    u16 bitmapcount;
    u16 reserved;
    u32 commonattr, volattr, dirattr, fileattr, forkattr;
};
#define ATTR_BIT_MAP_COUNT 5
#define ATTR_CMN_NAME 0x00000001u
#define ATTR_CMN_OBJTYPE 0x00000008u
#define ATTR_CMN_RETURNED_ATTRS 0x80000000u
#define ATTR_FILE_DATALENGTH 0x00000200u
#define FSOPT_NOFOLLOW 1
#define VREG 1
#define VDIR 2
#define VLNK 5

int test_main(void) {
    sysret f = bsd3(SYS_open, "file", O_CREAT | O_WRONLY, 0644);
    bsd3(SYS_write, f.ret, "0123456789", 10);
    bsd2(SYS_mkdir, "dir", 0755);
    struct attrlist_t al = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_NAME | ATTR_CMN_OBJTYPE, 0, 0, ATTR_FILE_DATALENGTH, 0};
    char buf[256];
    memset(buf, 0, sizeof buf);
    sysret r = bsd6(SYS_getattrlist, "file", &al, buf, sizeof buf, 0, 0);
    check("getattrlist returns 0", r.err == 0 && r.ret == 0, r.err);
    u32 *u = (u32 *)buf;
    check("the total length is 32", u[0] == 32, u[0]);
    check("the name: length 5 (\"file\" and NUL), reached by the offset", u[2] == 5 && str_eq(buf + 4 + (i32)u[1], "file"), u[2]);
    check("the object type: VREG", u[3] == VREG, u[3]);
    check("the data length: 10 (at offset 16)", *(i64 *)(buf + 16) == 10, *(i64 *)(buf + 16));
    al.fileattr = 0;
    al.commonattr = ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_NAME | ATTR_CMN_OBJTYPE;
    memset(buf, 0, sizeof buf);
    r = bsd6(SYS_getattrlist, "dir", &al, buf, sizeof buf, 0, 0);
    check_ok("with RETURNED_ATTRS, for a directory", r);
    check("the returned set: common attributes 0x80000009, nothing else", u[1] == 0x80000009u && u[2] == 0 && u[3] == 0 && u[4] == 0 && u[5] == 0, u[1]);
    check("the name \"dir\" (length 4)", u[7] == 4 && str_eq(buf + 24 + (i32)u[6], "dir"), u[7]);
    check("the object type: VDIR", u[8] == VDIR, u[8]);
    return finish();
}
