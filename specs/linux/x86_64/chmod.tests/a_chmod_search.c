/* chmod: every directory on the path needs search (execute) permission for
 * a normal user, EACCES otherwise; read and write permission on them do not
 * matter. Root needs none. The expectations depend on the uid; the check
 * names do not. */
#include "a_sys.h"

#define D "/tmp/a_chmod_search"

static int root;
static i64 want_eacces(void) { return root ? 0 : -EACCES; }

int test_main(void) {
    root = syscall0(SYS_geteuid) == 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0644), 0);
    expect("sub_0600", syscall2(SYS_chmod, D "/sub", 0600), 0);
    expect("no_search_eacces", syscall2(SYS_chmod, D "/sub/f", 0600), want_eacces());
    expect("no_search_missing", syscall2(SYS_chmod, D "/sub/nope", 0600), root ? -ENOENT : -EACCES);
    expect("no_search_dotdot", syscall2(SYS_chmod, D "/sub/..", 0755), want_eacces());
    expect("no_search_dir_itself_ok", syscall2(SYS_chmod, D "/sub", 0700), 0);
    expect("sub_0100", syscall2(SYS_chmod, D "/sub", 0100), 0);
    expect("search_only_ok", syscall2(SYS_chmod, D "/sub/f", 0604), 0);
    expect("top_0600", syscall2(SYS_chmod, D, 0600), 0);
    expect("upper_no_search_eacces", syscall2(SYS_chmod, D "/sub/f", 0600), want_eacces());
    expect("upper_no_search_sub_eacces", syscall2(SYS_chmod, D "/sub", 0700), want_eacces());
    expect("chdir_blocked", syscall1(SYS_chdir, D "/sub"), want_eacces());
    expect("top_0700", syscall2(SYS_chmod, D, 0700), 0);
    expect("chdir_sub", syscall1(SYS_chdir, D "/sub"), 0);
    expect("top_0600_again", syscall2(SYS_chmod, D, 0600), 0);
    expect("relative_from_inside_ok", syscall2(SYS_chmod, "f", 0640), 0);
    expect("relative_dotdot_eacces", syscall2(SYS_chmod, "../sub/f", 0640), want_eacces());
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("restore_top", syscall2(SYS_chmod, D, 0755), 0);
    expect("restore_sub", syscall2(SYS_chmod, D "/sub", 0755), 0);
    struct a_stat st;
    expect("final_mode", a_stat_path(D "/sub/f", &st) == 0 ? (i64)st.st_mode : -1, S_IFREG | 0640);
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
