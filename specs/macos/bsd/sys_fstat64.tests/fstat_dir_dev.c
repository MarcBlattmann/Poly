/* fstat64(2), BSD 339: an empty directory (APFS: two links, size 64) and
 * the character device /dev/null (mode 0666, device 3,2, size 0). */
#include "poly_mtest.h"

int test_main(void) {
    bsd1(SYS_umask, 022);
    bsd2(SYS_mkdir, "empty", 0755);
    sysret d = bsd3(SYS_open, "empty", O_RDONLY, 0);
    struct poly_stat64 st;
    check_ok("fstat64 of a directory", bsd2(SYS_fstat64, d.ret, &st));
    check("a directory, mode 0755", st.st_mode == (S_IFDIR | 0755), st.st_mode);
    check("an empty directory has 2 links", st.st_nlink == 2, st.st_nlink);
    check("and size 64 on APFS", st.st_size == 64, st.st_size);
    sysret n = bsd3(SYS_open, "/dev/null", O_RDONLY, 0);
    check_ok("fstat64 of /dev/null", bsd2(SYS_fstat64, n.ret, &st));
    check("a character device, mode 0666", st.st_mode == (S_IFCHR | 0666), st.st_mode);
    check("device 3,2 (major in the top byte)", st.st_rdev == ((3 << 24) | 2), st.st_rdev);
    check("size 0", st.st_size == 0, st.st_size);
    return finish();
}
