/* Removing and renaming names. unlink sends FUSE_LOOKUP (when the name is
 * not cached) and FUSE_UNLINK (10) for the parent node with the name;
 * rmdir FUSE_RMDIR (11); a name the server removed is ENOENT on the next
 * stat (a LOOKUP asks again and is answered ENOENT). rename sends RENAME
 * (12) {newdir = the target directory's node id} + old name + NUL + new
 * name for the old directory's node; renameat2 with RENAME_EXCHANGE sends
 * RENAME2 (45) {newdir, flags 2}; RENAME_NOREPLACE onto an existing name is
 * EEXIST without a request; renaming over an existing file replaces it
 * (the old target is gone, the node of the source answers at the new
 * name, st_ino unchanged); rename of a directory moves its contents.
 * rmdir of a non-empty directory is ENOTEMPTY (the server's answer), and
 * the server's error replies (ENOSYS from UNLINK, EACCES from MKDIR, EBUSY
 * from RENAME) are the calls' errors. unlink of a file that is open
 * succeeds; the open file keeps being readable through its node.
 * (linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define P "/tmp/pk_fuse_wns"

int test_main(void) {
    expect("setup", fz_setup(P), 0);
    expect("open", fz_open() >= 0, 1);
    expect("mount", fz_mount(0), 0);
    expect("start", fz_start() > 0, 1);

    struct a_stat st, before;
    expect("stat_hello", a_stat_path(P "/hello", &before), 0);
    int mark = fz_mark();
    expect("rename_within_dir", syscall2(SYS_rename, P "/hello", P "/hello2"), 0);
    const struct fz_rec *rn = fz_nth_since(mark, FZ_RENAME, 0);
    check("rename_sent", rn != 0, 0);
    if (rn) {
        expect("rename_old_dir", (i64)rn->nodeid, 1);
        expect("rename_new_dir", (i64)rn->a[0], 1);
        check("rename_old_name", str_eq(rn->name, "hello"), 0);
        check("rename_new_name", str_eq(rn->aux, "hello2"), 0);
    }
    expect("old_name_gone", a_stat_path(P "/hello", &st), -ENOENT);
    expect("new_name_same_inode", a_stat_path(P "/hello2", &st) == 0 ? (i64)(st.st_ino == before.st_ino) : -1, 1);

    mark = fz_mark();
    expect("rename_into_dir", syscall2(SYS_rename, P "/hello2", P "/dir/moved"), 0);
    rn = fz_nth_since(mark, FZ_RENAME, 0);
    check("rename_into_dir_sent", rn != 0, 0);
    struct a_stat dir;
    a_stat_path(P "/dir", &dir);
    if (rn) {
        expect("rename_into_dir_old_parent", (i64)rn->nodeid, 1);
        expect("rename_into_dir_newdir", (i64)rn->a[0], (i64)dir.st_ino);
    }
    expect("moved_exists", a_stat_path(P "/dir/moved", &st), 0);
    expect("moved_size", st.st_size, 16);

    expect("rename_over_existing", syscall2(SYS_rename, P "/dir/moved", P "/dir/inner"), 0);
    expect("replaced_target_content_size", a_stat_path(P "/dir/inner", &st) == 0 ? st.st_size : -1, 16);
    expect("source_name_gone", a_stat_path(P "/dir/moved", &st), -ENOENT);

    expect("rename_missing", syscall2(SYS_rename, P "/nothere", P "/other"), -ENOENT);

    expect("create_a", a_make_file(P "/a", "AAA", 0644), 0);
    expect("create_b", a_make_file(P "/b", "BBBBB", 0644), 0);
    mark = fz_mark();
    expect("exchange", syscall5(SYS_renameat2, AT_FDCWD, P "/a", AT_FDCWD, P "/b", RENAME_EXCHANGE), 0);
    const struct fz_rec *r2 = fz_nth_since(mark, FZ_RENAME2, 0);
    check("rename2_sent", r2 != 0, 0);
    if (r2) {
        expect("rename2_flags_exchange", (i64)r2->a[1], RENAME_EXCHANGE);
        expect("rename2_newdir", (i64)r2->a[0], 1);
    }
    expect("exchange_a_now_big", a_stat_path(P "/a", &st) == 0 ? st.st_size : -1, 5);
    expect("exchange_b_now_small", a_stat_path(P "/b", &st) == 0 ? st.st_size : -1, 3);
    mark = fz_mark();
    expect("noreplace_existing", syscall5(SYS_renameat2, AT_FDCWD, P "/a", AT_FDCWD, P "/b", RENAME_NOREPLACE), -EEXIST);
    expect("noreplace_no_request", fz_count_since(mark, FZ_RENAME) + fz_count_since(mark, FZ_RENAME2), 0);
    expect("noreplace_free_name", syscall5(SYS_renameat2, AT_FDCWD, P "/a", AT_FDCWD, P "/c", RENAME_NOREPLACE), 0);

    /* unlink */
    mark = fz_mark();
    expect("unlink_b", syscall3(SYS_unlinkat, AT_FDCWD, P "/b", 0), 0);
    const struct fz_rec *ul = fz_nth_since(mark, FZ_UNLINK, 0);
    check("unlink_sent", ul != 0, 0);
    if (ul) {
        expect("unlink_parent", (i64)ul->nodeid, 1);
        check("unlink_name", str_eq(ul->name, "b"), 0);
    }
    mark = fz_mark();
    expect("stat_removed", a_stat_path(P "/b", &st), -ENOENT);
    expect("lookup_asks_again", fz_count_since(mark, FZ_LOOKUP) >= 1, 1);
    expect("unlink_missing", syscall3(SYS_unlinkat, AT_FDCWD, P "/b", 0), -ENOENT);

    i64 fd = a_open(P "/c", O_RDONLY, 0);
    check("open_c", fd >= 0, fd);
    expect("unlink_open_file", syscall3(SYS_unlinkat, AT_FDCWD, P "/c", 0), 0);
    char buf[16];
    expect("open_file_still_reads", a_read(fd, buf, sizeof buf), 5);
    check("open_file_content", mem_eq(buf, "BBBBB", 5), 0);
    a_close(fd);

    /* directories */
    expect("mkdir_d", a_mkdir(P "/d", 0755), 0);
    expect("create_in_d", a_make_file(P "/d/f", "x", 0644), 0);
    expect("rmdir_nonempty", a_rmdir(P "/d"), -ENOTEMPTY);
    expect("rmdir_nonempty_request_sent", fz_last_named(FZ_RMDIR, "d") != 0, 1);
    expect("rename_dir", syscall2(SYS_rename, P "/d", P "/e"), 0);
    expect("content_moved_with_dir", a_stat_path(P "/e/f", &st), 0);
    expect("old_dir_gone", a_stat_path(P "/d", &st), -ENOENT);
    expect("unlink_inside", syscall3(SYS_unlinkat, AT_FDCWD, P "/e/f", 0), 0);
    mark = fz_mark();
    expect("rmdir_empty", a_rmdir(P "/e"), 0);
    const struct fz_rec *rd = fz_nth_since(mark, FZ_RMDIR, 0);
    check("rmdir_sent", rd != 0, 0);
    if (rd) check("rmdir_name", str_eq(rd->name, "e"), 0);
    expect("rmdir_gone", a_stat_path(P "/e", &st), -ENOENT);
    expect("rmdir_on_file", a_rmdir(P "/dir/inner"), -ENOTDIR);
    expect("unlink_on_dir", syscall3(SYS_unlinkat, AT_FDCWD, P "/dir", 0), -EISDIR);

    /* error replies */
    fz->fail_op = FZ_UNLINK;
    fz->fail_err = ENOSYS;
    expect("unlink_enosys", syscall3(SYS_unlinkat, AT_FDCWD, P "/dir/inner", 0), -ENOSYS);
    fz->fail_op = FZ_MKDIR;
    fz->fail_err = EACCES;
    expect("mkdir_eacces", a_mkdir(P "/nope", 0755), -EACCES);
    fz->fail_op = FZ_RENAME;
    fz->fail_err = EBUSY;
    expect("rename_ebusy", syscall2(SYS_rename, P "/dir/inner", P "/dir/other"), -EBUSY);
    fz->fail_op = FZ_CREATE;
    fz->fail_err = ENOSPC;
    expect("create_enospc", a_open(P "/full", O_WRONLY | O_CREAT, 0644), -ENOSPC);
    fz->fail_op = 0;
    expect("after_errors_still_there", a_stat_path(P "/dir/inner", &st), 0);
    fz_cleanup();
    return finish();
}
