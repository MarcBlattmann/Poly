/* lremovexattr only leaves the LAST component unfollowed: symbolic links
 * earlier in the path are followed (dlnk/f reaches dir/f), and a trailing
 * slash or "/." after a link makes it a directory lookup that follows the
 * link (dlnk/ and dlnk/. reach the directory). A link to a file with a
 * trailing slash is ENOTDIR, a dangling one ENOENT, a self-loop ELOOP.
 * Following counts: a chain of 40 links plus "/" resolves, 41 is ELOOP;
 * without the slash the 41st link itself is the object (EPERM). */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_lremovexattr 198
#define ENODATA 61
#define D "/tmp/a_lremovexattr_walk"

static char a[96], b[96], buf[16];

static i64 lrm(const char *p, const char *name) { return syscall2(SYS_lremovexattr, p, name); }
static i64 set(const char *p, const char *name) { return syscall5(SYS_setxattr, p, name, "v", 1, 0); }
static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }
static i64 sym(const char *target, const char *link) { return syscall3(SYS_symlinkat, target, AT_FDCWD, link); }

static char *link_name(char *out, int i, const char *suffix) {
    str_cpy(out, D "/c");
    char *p = out + str_len(out);
    a_utoa(p, i);
    str_cpy(p + str_len(p), suffix);
    return out;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_dir", a_mkdir(D "/dir", 0755), 0);
    expect("setup_f", a_make_file(D "/dir/f", "", 0644), 0);
    expect("setup_t", a_make_file(D "/t", "", 0644), 0);
    expect("setup_dlnk", sym(D "/dir", D "/dlnk"), 0);
    expect("setup_rdl", sym("dir", D "/rdl"), 0);
    expect("setup_lnk", sym("t", D "/lnk"), 0);
    expect("setup_dang", sym("nope", D "/dang"), 0);
    expect("setup_loop", sym("loop", D "/loop"), 0);
    int chain_ok = 1;
    for (int i = 1; i <= 41; i++) {
        if (i == 1) str_cpy(b, "dir");
        else {
            str_cpy(b, "c");
            a_utoa(b + 1, i - 1);
        }
        if (sym(b, link_name(a, i, "")) != 0) chain_ok = 0;
    }
    check("setup_chain", chain_ok, 0);
    expect("set_f", set(D "/dir/f", "user.f"), 0);
    expect("middle_link_followed", lrm(D "/dlnk/f", "user.f"), 0);
    expect("f_gone", get(D "/dir/f", "user.f"), -ENODATA);
    expect("middle_link_again", lrm(D "/rdl/f", "user.f"), -ENODATA);
    expect("set_dir", set(D "/dir", "user.d"), 0);
    expect("dir_link_itself_eperm", lrm(D "/dlnk", "user.d"), -EPERM);
    expect("dir_kept", get(D "/dir", "user.d"), 1);
    expect("trailing_slash_follows", lrm(D "/dlnk/", "user.d"), 0);
    expect("dir_gone", get(D "/dir", "user.d"), -ENODATA);
    expect("trailing_slash_again", lrm(D "/dlnk/", "user.d"), -ENODATA);
    expect("set_dir2", set(D "/dir", "user.d"), 0);
    expect("slash_dot_follows", lrm(D "/rdl/.", "user.d"), 0);
    expect("dir_gone2", get(D "/dir", "user.d"), -ENODATA);
    expect("file_link_slash_enotdir", lrm(D "/lnk/", "user.a"), -ENOTDIR);
    expect("file_link_slash_unknown_enotdir", lrm(D "/lnk/", "foo.bar"), -ENOTDIR);
    expect("dangling_slash_enoent", lrm(D "/dang/", "user.a"), -ENOENT);
    expect("loop_slash_eloop", lrm(D "/loop/", "user.a"), -ELOOP);
    expect("file_link_component_enotdir", lrm(D "/lnk/x", "user.a"), -ENOTDIR);
    expect("dangling_component_enoent", lrm(D "/dang/x", "user.a"), -ENOENT);
    expect("set_dir3", set(D "/dir", "user.q"), 0);
    expect("chain_41_itself_eperm", lrm(link_name(a, 41, ""), "user.q"), -EPERM);
    expect("chain_41_slash_eloop", lrm(link_name(a, 41, "/"), "user.q"), -ELOOP);
    expect("dir_kept_after_eloop", get(D "/dir", "user.q"), 1);
    expect("chain_40_slash_ok", lrm(link_name(a, 40, "/"), "user.q"), 0);
    expect("dir_gone3", get(D "/dir", "user.q"), -ENODATA);
    for (int i = 1; i <= 41; i++) a_unlink(link_name(a, i, ""));
    a_unlink(D "/dlnk");
    a_unlink(D "/rdl");
    a_unlink(D "/lnk");
    a_unlink(D "/dang");
    a_unlink(D "/loop");
    expect("cleanup_f", a_unlink(D "/dir/f"), 0);
    expect("cleanup_t", a_unlink(D "/t"), 0);
    expect("cleanup_dir", a_rmdir(D "/dir"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
