/* mkdir path spelling: trailing slashes on the new name are allowed (one
 * or several), repeated slashes anywhere count as one, a leading "//" is
 * the root, "." components stay put and ".." goes to the parent (at "/"
 * it stays at "/"). A new name followed by "/." or "/.." is ENOENT: the
 * name would have to exist already. */
#include "a_sys.h"

#define D "/tmp/c_mkdir_slashes_dots"

static i64 mk(const char *p, i64 m) { return syscall2(SYS_mkdir, p, m); }
static int is_dir(const char *p) {
    struct a_stat st;
    return a_lstat_path(p, &st) == 0 && (st.st_mode & S_IFMT) == S_IFDIR;
}

int test_main(void) {
    struct a_stat st;
    expect("setup", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);

    expect("trailing_slash", mk(D "/a/", 0700), 0);
    expect("a_made", is_dir(D "/a"), 1);
    expect("trailing_slashes", mk(D "/b///", 0700), 0);
    expect("b_made", is_dir(D "/b"), 1);
    expect("double_slashes_inside", mk("/tmp//c_mkdir_slashes_dots///c", 0700), 0);
    expect("c_made", is_dir(D "/c"), 1);
    expect("leading_double_slash", mk("//tmp/c_mkdir_slashes_dots/d", 0700), 0);
    expect("d_made", is_dir(D "/d"), 1);
    expect("leading_triple_slash", mk("///tmp/c_mkdir_slashes_dots/e", 0700), 0);
    expect("e_made", is_dir(D "/e"), 1);
    expect("dot_components", mk("/./tmp/./c_mkdir_slashes_dots/./f", 0700), 0);
    expect("f_made", is_dir(D "/f"), 1);
    expect("dotdot_component", mk(D "/sub/../g", 0700), 0);
    expect("g_made_in_D", is_dir(D "/g"), 1);
    expect("g_not_in_sub", a_lstat_path(D "/sub/g", &st), -ENOENT);
    expect("dotdot_dotdot", mk(D "/sub/../sub/../h", 0700), 0);
    expect("h_made", is_dir(D "/h"), 1);
    expect("dotdot_at_root", mk("/../../tmp/c_mkdir_slashes_dots/i", 0700), 0);
    expect("i_made", is_dir(D "/i"), 1);
    expect("dot_slash_trailing", mk(D "/./j/", 0700), 0);
    expect("j_made", is_dir(D "/j"), 1);

    expect("new_then_dot", mk(D "/k/.", 0700), -ENOENT);
    expect("new_then_dotdot", mk(D "/k/..", 0700), -ENOENT);
    expect("new_then_dot_slash", mk(D "/k/./", 0700), -ENOENT);
    expect("k_not_made", a_lstat_path(D "/k", &st), -ENOENT);

    expect("rm_a", a_rmdir(D "/a"), 0);
    expect("rm_b", a_rmdir(D "/b"), 0);
    expect("rm_c", a_rmdir(D "/c"), 0);
    expect("rm_d", a_rmdir(D "/d"), 0);
    expect("rm_e", a_rmdir(D "/e"), 0);
    expect("rm_f", a_rmdir(D "/f"), 0);
    expect("rm_g", a_rmdir(D "/g"), 0);
    expect("rm_h", a_rmdir(D "/h"), 0);
    expect("rm_i", a_rmdir(D "/i"), 0);
    expect("rm_j", a_rmdir(D "/j"), 0);
    expect("rm_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
