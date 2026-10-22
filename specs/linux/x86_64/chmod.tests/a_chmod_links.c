/* chmod follows symbolic links, also in the last component: the target's
 * mode changes and the link's own mode stays 0777. A dangling link is
 * ENOENT, a loop ELOOP. */
#include "a_sys.h"

#define D "/tmp/a_chmod_links"

static i64 lmode(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    expect("setup_link_f", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/lf"), 0);
    expect("setup_link_d", syscall3(SYS_symlinkat, D "/d", AT_FDCWD, D "/ld"), 0);
    expect("setup_link_link", syscall3(SYS_symlinkat, "lf", AT_FDCWD, D "/llf"), 0);
    expect("setup_dangling", syscall3(SYS_symlinkat, "gone", AT_FDCWD, D "/dang"), 0);
    expect("setup_loop_a", syscall3(SYS_symlinkat, "loopb", AT_FDCWD, D "/loopa"), 0);
    expect("setup_loop_b", syscall3(SYS_symlinkat, "loopa", AT_FDCWD, D "/loopb"), 0);
    expect("setup_self", syscall3(SYS_symlinkat, "self", AT_FDCWD, D "/self"), 0);

    expect("chmod_via_link", syscall2(SYS_chmod, D "/lf", 0600), 0);
    expect("target_changed", lmode(D "/f"), S_IFREG | 0600);
    expect("link_unchanged", lmode(D "/lf"), S_IFLNK | 0777);
    expect("chmod_via_link_chain", syscall2(SYS_chmod, D "/llf", 0440), 0);
    expect("chain_target_changed", lmode(D "/f"), S_IFREG | 0440);
    expect("chain_links_unchanged", lmode(D "/llf") == (S_IFLNK | 0777) && lmode(D "/lf") == (S_IFLNK | 0777), 1);
    expect("chmod_via_dir_link", syscall2(SYS_chmod, D "/ld", 0700), 0);
    expect("dir_target_changed", lmode(D "/d"), S_IFDIR | 0700);
    expect("chmod_via_dir_link_slash", syscall2(SYS_chmod, D "/ld/", 0750), 0);
    expect("dir_target_changed_2", lmode(D "/d"), S_IFDIR | 0750);
    expect("setup_inner", a_make_file(D "/d/x", "", 0644), 0);
    expect("chmod_through_dir_link", syscall2(SYS_chmod, D "/ld/x", 0604), 0);
    expect("inner_changed", lmode(D "/d/x"), S_IFREG | 0604);
    expect("dangling_enoent", syscall2(SYS_chmod, D "/dang", 0600), -ENOENT);
    expect("dangling_still_link", lmode(D "/dang"), S_IFLNK | 0777);
    expect("loop_eloop", syscall2(SYS_chmod, D "/loopa", 0600), -ELOOP);
    expect("self_loop_eloop", syscall2(SYS_chmod, D "/self", 0600), -ELOOP);
    expect("loop_in_middle_eloop", syscall2(SYS_chmod, D "/loopa/x", 0600), -ELOOP);
    expect("link_to_file_then_more_enotdir", syscall2(SYS_chmod, D "/lf/x", 0600), -ENOTDIR);
    expect("link_to_file_slash_enotdir", syscall2(SYS_chmod, D "/lf/", 0600), -ENOTDIR);
    expect("dangling_slash_enoent", syscall2(SYS_chmod, D "/dang/", 0600), -ENOENT);

    const char *names[] = {D "/d/x", D "/f", D "/lf", D "/ld", D "/llf", D "/dang", D "/loopa", D "/loopb", D "/self"};
    int ok = 1;
    for (int i = 0; i < 9; i++) ok &= a_unlink(names[i]) == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_d", a_rmdir(D "/d"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
