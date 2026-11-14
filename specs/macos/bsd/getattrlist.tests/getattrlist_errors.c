/* getattrlist, BSD 220, errors: ENOENT for a missing path, EINVAL when the
 * bitmap count is not 5, EFAULT for an unreadable attrlist or buffer and for
 * the path, ENOTDIR for a file used as a directory. */
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
    bsd1(SYS_close, bsd3(SYS_open, "file", O_CREAT | O_WRONLY, 0644).ret);
    struct attrlist_t al = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_NAME, 0, 0, 0, 0};
    char buf[64];
    check_err("a missing path: ENOENT", bsd6(SYS_getattrlist, "missing", &al, buf, sizeof buf, 0, 0), ENOENT);
    check_err("a file used as a directory: ENOTDIR", bsd6(SYS_getattrlist, "file/x", &al, buf, sizeof buf, 0, 0), ENOTDIR);
    al.bitmapcount = 4;
    check_err("a bitmap count of 4: EINVAL", bsd6(SYS_getattrlist, "file", &al, buf, sizeof buf, 0, 0), EINVAL);
    al.bitmapcount = 6;
    value("a bitmap count of 6: errno", bsd6(SYS_getattrlist, "file", &al, buf, sizeof buf, 0, 0).err);
    al.bitmapcount = ATTR_BIT_MAP_COUNT;
    check_err("the attrlist at address 8: EFAULT", bsd6(SYS_getattrlist, "file", (void *)8, buf, sizeof buf, 0, 0), EFAULT);
    check_err("the buffer at address 8: EFAULT", bsd6(SYS_getattrlist, "file", &al, (void *)8, 64, 0, 0), EFAULT);
    check_err("the path at address 8: EFAULT", bsd6(SYS_getattrlist, (void *)8, &al, buf, sizeof buf, 0, 0), EFAULT);
    return finish();
}
