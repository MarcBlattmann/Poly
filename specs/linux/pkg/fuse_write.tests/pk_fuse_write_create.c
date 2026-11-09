/* Creating names. creat/open(O_CREAT|O_EXCL) of a new name sends FUSE_LOOKUP
 * (answered ENOENT) and then FUSE_CREATE (35) for the parent node with
 * struct fuse_create_in {flags (the open flags, access mode in the low two
 * bits, with O_CREAT and O_EXCL), mode (S_IFREG | the requested
 * permissions with the process umask applied: 0666 & ~022 = 0644), umask
 * (022)} followed by the name; the reply is an entry plus an open reply,
 * the file is open at once (no OPEN request), its first WRITE carries the
 * fh of the reply. mkdir sends MKDIR (9) {mode 0755 = 0777 & ~022, umask}
 * + name, mknod MKNOD (8) {mode with S_IFIFO / S_IFCHR and the permissions
 * with umask applied, rdev (old 16-bit encoding major<<8|minor, 0x103 for
 * 1:3), umask}, symlink SYMLINK (6) with name and target, link LINK (13)
 * {oldnodeid} + new name in the parent. The new names exist afterwards
 * with the attributes of the replies (mode, nlink 2 after link, size of
 * a symlink = target length), and a create of an existing name is EEXIST
 * (the lookup finds it; no CREATE is sent).
 * (linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define P "/tmp/pk_fuse_wcreate"

int test_main(void) {
    expect("setup", fz_setup(P), 0);
    expect("open", fz_open() >= 0, 1);
    expect("mount", fz_mount(0), 0);
    expect("start", fz_start() > 0, 1);
    syscall1(SYS_umask, 022);

    int mark = fz_mark();
    i64 fd = a_open(P "/newfile", O_RDWR | O_CREAT | O_EXCL, 0666);
    check("create_open", fd >= 0, fd);
    const struct fz_rec *lk = fz_nth_since(mark, FZ_LOOKUP, 0);
    check("lookup_before_create", lk != 0, 0);
    if (lk) expect("lookup_answered_enoent", lk->reply_err, ENOENT);
    const struct fz_rec *cr = fz_nth_since(mark, FZ_CREATE, 0);
    check("create_sent", cr != 0, 0);
    if (cr) {
        expect("create_parent", (i64)cr->nodeid, 1);
        check("create_name", str_eq(cr->name, "newfile"), 0);
        expect("create_access_mode", cr->a[0] & 3, O_RDWR);
        expect("create_has_creat", (i64)((cr->a[0] & O_CREAT) != 0), 1);
        expect("create_has_excl", (i64)((cr->a[0] & O_EXCL) != 0), 1);
        expect("create_mode", (i64)cr->a[1], S_IFREG | 0644);
        expect("create_umask", (i64)cr->a[2], 022);
    }
    expect("no_open_after_create", fz_count_since(mark, FZ_OPEN), 0);
    expect("write_new_file", a_write(fd, "0123456789", 10), 10);
    const struct fz_rec *wr = fz_nth_since(mark, FZ_WRITE, 0);
    check("write_sent", wr != 0, 0);
    if (wr) {
        check("write_node_is_new", wr->nodeid >= 8, (i64)wr->nodeid);
        expect("write_offset", (i64)wr->a[0], 0);
        expect("write_size", (i64)wr->a[1], 10);
        expect("write_fh_is_creates", (i64)wr->a[3], 101);
    }
    struct a_stat st;
    expect("fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("fstat_size", st.st_size, 10);
    expect("fstat_mode", st.st_mode, S_IFREG | 0644);
    expect("close", a_close(fd), 0);
    expect("stat_after_close", a_stat_path(P "/newfile", &st), 0);
    expect("size_after_close", st.st_size, 10);
    char b[16];
    fd = a_open(P "/newfile", O_RDONLY, 0);
    expect("read_back", a_read(fd, b, sizeof b), 10);
    check("read_back_content", mem_eq(b, "0123456789", 10), 0);
    a_close(fd);
    expect("create_again_eexist", a_open(P "/newfile", O_RDWR | O_CREAT | O_EXCL, 0666), -EEXIST);
    expect("create_over_existing_without_excl", ({ i64 f = a_open(P "/newfile", O_WRONLY | O_CREAT, 0600); if (f >= 0) a_close(f); f >= 0; }), 1);
    expect("creates_counted", fz_count_since(mark, FZ_CREATE), 1);

    mark = fz_mark();
    expect("mkdir", a_mkdir(P "/newdir", 0777), 0);
    const struct fz_rec *md = fz_nth_since(mark, FZ_MKDIR, 0);
    check("mkdir_sent", md != 0, 0);
    if (md) {
        expect("mkdir_parent", (i64)md->nodeid, 1);
        check("mkdir_name", str_eq(md->name, "newdir"), 0);
        expect("mkdir_mode", (i64)md->a[0] & 07777, 0755);
        expect("mkdir_umask", (i64)md->a[1], 022);
    }
    expect("stat_newdir", a_stat_path(P "/newdir", &st), 0);
    expect("newdir_mode", st.st_mode, S_IFDIR | 0755);
    expect("mkdir_inside", a_mkdir(P "/newdir/sub", 0700), 0);
    const struct fz_rec *md2 = fz_last(FZ_MKDIR);
    check("nested_mkdir_sent", md2 != 0 && str_eq(md2->name, "sub"), 0);
    if (md2) expect("nested_mkdir_parent_is_newdir", (i64)md2->nodeid, (i64)st.st_ino);
    expect("mkdir_exists", a_mkdir(P "/newdir", 0755), -EEXIST);

    mark = fz_mark();
    expect("mknod_fifo", syscall3(SYS_mknod, P "/fifo", S_IFIFO | 0666, 0), 0);
    const struct fz_rec *mk = fz_nth_since(mark, FZ_MKNOD, 0);
    check("mknod_sent", mk != 0, 0);
    if (mk) {
        expect("mknod_mode", (i64)mk->a[0], S_IFIFO | 0644);
        expect("mknod_umask", (i64)mk->a[2], 022);
    }
    expect("stat_fifo", a_stat_path(P "/fifo", &st), 0);
    expect("fifo_type", st.st_mode & S_IFMT, S_IFIFO);
    mark = fz_mark();
    expect("mknod_chr", syscall3(SYS_mknod, P "/null", S_IFCHR | 0600, (1 << 8) | 3), 0);
    mk = fz_nth_since(mark, FZ_MKNOD, 0);
    check("mknod_chr_sent", mk != 0, 0);
    if (mk) {
        expect("mknod_chr_mode", (i64)mk->a[0], S_IFCHR | 0600);
        expect("mknod_chr_rdev", (i64)mk->a[1], 0x103);
    }
    expect("stat_chr", a_stat_path(P "/null", &st), 0);
    expect("chr_rdev", (i64)st.st_rdev, 0x103);

    mark = fz_mark();
    expect("symlink", syscall2(SYS_symlink, "newfile", P "/sl"), 0);
    const struct fz_rec *sy = fz_nth_since(mark, FZ_SYMLINK, 0);
    check("symlink_sent", sy != 0, 0);
    if (sy) {
        check("symlink_name", str_eq(sy->name, "sl"), 0);
        check("symlink_target", str_eq(sy->aux, "newfile"), 0);
    }
    expect("lstat_symlink", a_lstat_path(P "/sl", &st), 0);
    expect("symlink_mode", st.st_mode & S_IFMT, S_IFLNK);
    expect("symlink_size", st.st_size, 7);
    char l[16];
    expect("readlink_new", syscall3(SYS_readlink, P "/sl", l, sizeof l), 7);

    mark = fz_mark();
    expect("link", syscall2(SYS_link, P "/newfile", P "/hardlink"), 0);
    const struct fz_rec *li = fz_nth_since(mark, FZ_LINK, 0);
    check("link_sent", li != 0, 0);
    if (li) {
        expect("link_parent", (i64)li->nodeid, 1);
        check("link_name", str_eq(li->name, "hardlink"), 0);
        struct a_stat src;
        a_stat_path(P "/newfile", &src);
        expect("link_oldnodeid_is_source", (i64)li->a[0], (i64)src.st_ino);
    }
    struct a_stat s1, s2;
    expect("stat_link_a", a_stat_path(P "/newfile", &s1), 0);
    expect("stat_link_b", a_stat_path(P "/hardlink", &s2), 0);
    expect("link_same_inode", (i64)(s1.st_ino == s2.st_ino), 1);
    expect("link_nlink_two", (i64)s2.st_nlink, 2);
    expect("link_target_exists", syscall2(SYS_link, P "/newfile", P "/hardlink"), -EEXIST);
    fz_cleanup();
    return finish();
}
