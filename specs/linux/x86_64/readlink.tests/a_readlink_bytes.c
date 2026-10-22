/* readlink: the text is any bytes but NUL and comes back byte for byte:
 * high bytes, newlines, spaces, repeated slashes, a lone "/". */
#include "a_sys.h"

#define D "/tmp/a_readlink_bytes"

static i64 rl(const char *p, char *buf, i64 n) { return syscall3(SYS_readlink, p, buf, n); }

int test_main(void) {
    char buf[300], text[256];
    int i;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    for (i = 0; i < 255; i++) text[i] = (char)(i + 1);
    text[255] = 0;
    expect("setup_all_bytes", syscall2(SYS_symlink, text, D "/all"), 0);
    memset(buf, 0, sizeof buf);
    expect("all_bytes_length", rl(D "/all", buf, sizeof buf), 255);
    check("all_bytes_same", mem_eq(buf, text, 255), buf[0]);
    expect("setup_slash", syscall2(SYS_symlink, "/", D "/root"), 0);
    expect("slash_length", rl(D "/root", buf, sizeof buf), 1);
    check("slash_text", buf[0] == '/', buf[0]);
    expect("setup_slashes", syscall2(SYS_symlink, "a///b/", D "/slashes"), 0);
    expect("slashes_length", rl(D "/slashes", buf, sizeof buf), 6);
    check("slashes_text", mem_eq(buf, "a///b/", 6), buf[0]);
    expect("setup_space", syscall2(SYS_symlink, " sp ace\n", D "/space"), 0);
    expect("space_length", rl(D "/space", buf, sizeof buf), 8);
    check("space_text", mem_eq(buf, " sp ace\n", 8), buf[0]);
    expect("setup_dots", syscall2(SYS_symlink, "..", D "/dots"), 0);
    expect("dots_length", rl(D "/dots", buf, sizeof buf), 2);
    expect("empty_text_enoent", syscall2(SYS_symlink, "", D "/empty"), -ENOENT);
    expect("cleanup_all", a_unlink(D "/all"), 0);
    expect("cleanup_root", a_unlink(D "/root"), 0);
    expect("cleanup_slashes", a_unlink(D "/slashes"), 0);
    expect("cleanup_space", a_unlink(D "/space"), 0);
    expect("cleanup_dots", a_unlink(D "/dots"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
