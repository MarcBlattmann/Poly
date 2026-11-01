/* stat(2) fills the 144-byte x86-64 struct stat of what path names,
 * following symbolic links (also the last one): a link reports its
 * target. Fields for regular files, directories, FIFOs, /dev/null; the
 * padding and reserved words are 0; nothing past 144 bytes is written. */
#include "a_sys.h"

#define D "/tmp/lb_stat_kinds"
#define SYS_mknodat 259

static i64 stat_(const char *p, void *st) { return syscall2(SYS_stat, p, st); }

static unsigned char raw[160];

int test_main(void) {
    struct a_stat st, lst, fst;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "0123456789", 0640), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0750), 0);
    expect("setup_subsub", a_mkdir(D "/sub/x", 0700), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0600, 0), 0);
    expect("setup_l", syscall2(SYS_symlink, "f", D "/l"), 0);
    expect("setup_dl", syscall2(SYS_symlink, "sub", D "/dl"), 0);
    expect("setup_chain", syscall2(SYS_symlink, "l", D "/chain"), 0);

    /* regular file */
    expect("stat_f", stat_(D "/f", &st), 0);
    expect("f_mode", st.st_mode, S_IFREG | 0640);
    expect("f_size", st.st_size, 10);
    expect("f_nlink", st.st_nlink, 1);
    expect("f_uid", st.st_uid, syscall0(SYS_geteuid));
    expect("f_gid", st.st_gid, syscall0(SYS_getegid));
    expect("f_rdev", st.st_rdev, 0);
    check("f_blksize_positive", st.st_blksize > 0, st.st_blksize);
    check("f_blocks_some", st.st_blocks >= 1, st.st_blocks);
    check("f_nsec_range", st.st_atime_nsec < 1000000000 && st.st_mtime_nsec < 1000000000 && st.st_ctime_nsec < 1000000000, 0);
    expect("f_pad0", st.pad0, 0);
    expect("f_unused", st.unused[0] | st.unused[1] | st.unused[2], 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("fstat_f", syscall2(SYS_fstat, fd, &fst), 0);
    a_close(fd);
    expect("stat_equals_fstat", mem_eq(&st, &fst, sizeof st), 1);

    /* the last link is followed: same struct as the target */
    expect("stat_l", stat_(D "/l", &lst), 0);
    expect("link_reports_target", mem_eq(&st, &lst, sizeof st), 1);
    expect("stat_chain", stat_(D "/chain", &lst), 0);
    expect("chain_reports_target", lst.st_ino == st.st_ino && lst.st_mode == (S_IFREG | 0640), 1);
    expect("lstat_l", a_lstat_path(D "/l", &lst), 0);
    expect("lstat_differs", lst.st_ino != st.st_ino && (lst.st_mode & S_IFMT) == S_IFLNK, 1);

    /* directory */
    expect("stat_sub", stat_(D "/sub", &st), 0);
    expect("sub_mode", st.st_mode, S_IFDIR | 0750);
    expect("sub_nlink", st.st_nlink, 3);
    expect("stat_dl", stat_(D "/dl", &lst), 0);
    expect("dirlink_reports_dir", lst.st_ino == st.st_ino && lst.st_mode == (S_IFDIR | 0750), 1);
    expect("stat_dl_slash", stat_(D "/dl/", &lst), 0);
    expect("stat_dl_dot", stat_(D "/dl/.", &lst) == 0 && lst.st_ino == st.st_ino, 1);
    expect("stat_dl_dotdot_is_D", stat_(D "/dl/..", &lst) == 0 && stat_(D, &st) == 0 && lst.st_ino == st.st_ino, 1);
    expect("root", stat_("/", &st) == 0 && (st.st_mode & S_IFMT) == S_IFDIR, 1);
    expect("root_dotdot", stat_("/..", &lst) == 0 && lst.st_ino == st.st_ino && lst.st_dev == st.st_dev, 1);

    /* FIFO, character device */
    expect("stat_fifo", stat_(D "/fifo", &st), 0);
    expect("fifo_mode", st.st_mode, S_IFIFO | 0600);
    expect("fifo_size", st.st_size, 0);
    expect("stat_null", stat_("/dev/null", &st), 0);
    expect("null_mode", st.st_mode, S_IFCHR | 0666);
    expect("null_rdev", st.st_rdev, (1 << 8) | 3);

    /* exactly 144 bytes are written */
    for (unsigned i = 0; i < sizeof raw; i++) raw[i] = 0xAA;
    expect("stat_raw", stat_(D "/f", raw), 0);
    expect("byte_144_untouched", raw[144], 0xAA);
    expect("byte_159_untouched", raw[159], 0xAA);

    /* hard links share the inode; st_nlink counts them */
    expect("hardlink", syscall2(SYS_link, D "/f", D "/h"), 0);
    expect("stat_h", stat_(D "/h", &st) == 0 && stat_(D "/f", &lst) == 0 && st.st_ino == lst.st_ino && st.st_nlink == 2, 1);

    static const char *rm[] = {D "/h", D "/chain", D "/dl", D "/l", D "/fifo", D "/f"};
    for (unsigned i = 0; i < sizeof rm / sizeof rm[0]; i++) expect("cleanup_unlink", a_unlink(rm[i]), 0);
    expect("cleanup_subsub", a_rmdir(D "/sub/x"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
