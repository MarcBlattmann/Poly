/* readlink: returns the link text without a trailing NUL and leaves the
 * rest of the buffer alone; absolute and relative texts alike. */
#include "a_sys.h"

#define D "/tmp/a_readlink_basic"

static i64 rl(const char *p, char *buf, i64 n) { return syscall3(SYS_readlink, p, buf, n); }

int test_main(void) {
    char buf[64];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_rel", syscall2(SYS_symlink, "some/target", D "/rel"), 0);
    expect("setup_abs", syscall2(SYS_symlink, "/etc/passwd", D "/abs"), 0);
    expect("setup_one", syscall2(SYS_symlink, "x", D "/one"), 0);
    memset(buf, 'z', sizeof buf);
    expect("rel_length", rl(D "/rel", buf, sizeof buf), 11);
    check("rel_text", mem_eq(buf, "some/target", 11), buf[0]);
    check("rel_no_nul", buf[11] == 'z', buf[11]);
    memset(buf, 'z', sizeof buf);
    expect("abs_length", rl(D "/abs", buf, sizeof buf), 11);
    check("abs_text", mem_eq(buf, "/etc/passwd", 11) && buf[11] == 'z', buf[11]);
    memset(buf, 'z', sizeof buf);
    expect("one_byte_text", rl(D "/one", buf, sizeof buf), 1);
    check("one_byte_value", buf[0] == 'x' && buf[1] == 'z', buf[1]);
    /* Exact fit: bufsiz equal to the length. */
    memset(buf, 'z', sizeof buf);
    expect("exact_fit", rl(D "/rel", buf, 11), 11);
    check("exact_fit_text", mem_eq(buf, "some/target", 11) && buf[11] == 'z', buf[11]);
    /* A second call gives the same answer (nothing is consumed). */
    expect("repeat", rl(D "/rel", buf, sizeof buf), 11);
    expect("cleanup_rel", a_unlink(D "/rel"), 0);
    expect("cleanup_abs", a_unlink(D "/abs"), 0);
    expect("cleanup_one", a_unlink(D "/one"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
