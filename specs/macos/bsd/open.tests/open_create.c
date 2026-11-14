/* open(2), BSD 5: O_CREAT makes a regular file with the mode less the
 * umask, on the lowest free descriptor; O_EXCL refuses an existing file;
 * O_TRUNC empties one. */
#include "poly_mtest.h"

int test_main(void) {
    bsd1(SYS_umask, 022);
    sysret f = bsd3(SYS_open, "new.txt", O_CREAT | O_WRONLY, 0666);
    check_ok("O_CREAT of a new file", f);
    check("the lowest free descriptor, 3", f.ret == 3, f.ret);
    struct poly_stat64 st;
    bsd2(SYS_fstat64, f.ret, &st);
    check("mode 0666 less umask 022: regular file 0644", st.st_mode == (S_IFREG | 0644), st.st_mode);
    bsd3(SYS_write, f.ret, "data", 4);
    bsd1(SYS_close, f.ret);
    check_err("O_CREAT|O_EXCL of an existing file: EEXIST", bsd3(SYS_open, "new.txt", O_CREAT | O_EXCL | O_WRONLY, 0644), EEXIST);
    bsd1(SYS_umask, 077);
    sysret g = bsd3(SYS_open, "private.txt", O_CREAT | O_EXCL | O_WRONLY, 0666);
    check_ok("O_CREAT|O_EXCL of a new file", g);
    bsd2(SYS_fstat64, g.ret, &st);
    check("umask 077: mode 0600", st.st_mode == (S_IFREG | 0600), st.st_mode);
    sysret t = bsd3(SYS_open, "new.txt", O_WRONLY | O_TRUNC, 0);
    check_ok("O_TRUNC of an existing file", t);
    bsd2(SYS_fstat64, t.ret, &st);
    check("it is empty afterwards", st.st_size == 0, st.st_size);
    sysret c = bsd3(SYS_open, "new.txt", O_RDONLY | O_CREAT, 0600);
    check_ok("O_CREAT of an existing file without O_EXCL opens it", c);
    return finish();
}
