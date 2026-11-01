/* readlink(2) copies a symbolic link's contents into buf without a
 * terminating NUL and returns the number of bytes; a buffer shorter than
 * the contents gets the first bufsiz bytes (no error); bytes after the
 * copied ones are not touched. The last component is not followed, links
 * before it are. */
#include "a_sys.h"

#define D "/tmp/lb_readlink_basic"

static i64 readlink_(const char *p, char *b, i64 n) { return syscall3(SYS_readlink, p, b, n); }

static char big[5000], target[4200];

int test_main(void) {
    char b[64];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_l", syscall2(SYS_symlink, "target/of/link", D "/l"), 0);
    expect("setup_dl", syscall2(SYS_symlink, "sub", D "/dl"), 0);
    expect("setup_inner", syscall2(SYS_symlink, "../l", D "/sub/inner"), 0);
    expect("setup_chain", syscall2(SYS_symlink, "l", D "/chain"), 0);

    for (int i = 0; i < 64; i++) b[i] = '#';
    expect("full", readlink_(D "/l", b, 64), 14);
    expect("full_bytes", mem_eq(b, "target/of/link", 14), 1);
    expect("no_nul_appended", b[14], '#');
    /* exact size, short buffers */
    for (int i = 0; i < 64; i++) b[i] = '#';
    expect("exact", readlink_(D "/l", b, 14), 14);
    expect("exact_no_nul", b[14], '#');
    for (int i = 0; i < 64; i++) b[i] = '#';
    expect("short_6", readlink_(D "/l", b, 6), 6);
    expect("short_bytes", mem_eq(b, "target", 6), 1);
    expect("short_untouched", b[6], '#');
    expect("short_1", readlink_(D "/l", b, 1), 1);
    expect("short_1_byte", b[0], 't');
    /* a huge bufsiz is fine */
    expect("huge_bufsiz", readlink_(D "/l", big, sizeof big), 14);
    /* the link itself, not its target; a chain gives the first link */
    expect("chain_first_link", readlink_(D "/chain", b, 64), 1);
    expect("chain_bytes", b[0], 'l');
    /* links before the last component are followed */
    expect("through_dirlink", readlink_(D "/dl/inner", b, 64), 4);
    expect("through_dirlink_bytes", mem_eq(b, "../l", 4), 1);
    /* a dangling link and a looping link can be read */
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);
    expect("loop_read", readlink_(D "/loop", b, 64), 4);
    expect("dangling_read", readlink_(D "/l", b, 64), 14);
    /* relative paths */
    expect("chdir", syscall1(SYS_chdir, D "/sub"), 0);
    expect("relative", readlink_("inner", b, 64), 4);
    expect("relative_dotdot", readlink_("../l", b, 64), 14);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    /* the longest contents: 4095 bytes */
    for (int i = 0; i < 4095; i++) target[i] = (char)('A' + i % 26);
    target[4095] = 0;
    expect("setup_long", syscall2(SYS_symlink, target, D "/long"), 0);
    expect("long_read", readlink_(D "/long", big, sizeof big), 4095);
    expect("long_bytes", mem_eq(big, target, 4095), 1);
    expect("long_short_buf", readlink_(D "/long", big, 100), 100);
    /* bufsiz is an int: the upper half of the register is ignored */
    expect("bufsiz_upper_half", readlink_(D "/l", b, (i64)0x100000000L | 64), 14);

    static const char *rm[] = {D "/long", D "/loop", D "/chain", D "/sub/inner", D "/dl", D "/l"};
    for (unsigned i = 0; i < sizeof rm / sizeof rm[0]; i++) expect("cleanup_unlink", a_unlink(rm[i]), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
