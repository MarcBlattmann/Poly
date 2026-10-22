/* creat(2) on an existing regular file: it is emptied, keeps its inode,
 * mode and owner (the mode argument is ignored), and other descriptors for
 * it see the new size but keep their own offsets. */
#include "a_sys.h"

#define SYS_creat 85
#define D "/tmp/a_creat_trunc"

static i64 cr(const char *p, i64 mode) { return syscall2(SYS_creat, p, mode); }

int test_main(void) {
    struct a_stat st, st2;
    char buf[16];
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "0123456789", 0640), 0);
    expect("stat_before", a_stat_path(D "/f", &st), 0);

    i64 rd = a_open(D "/f", O_RDONLY, 0);
    expect("reader_open", rd, 3);
    expect("reader_read_4", a_read(rd, buf, 4), 4);

    i64 fd = cr(D "/f", 0777);
    expect("creat_existing_fd_4", fd, 4);
    expect("fstat_after", syscall2(SYS_fstat, fd, &st2), 0);
    expect("same_inode", st2.st_ino == st.st_ino, 1);
    expect("size_now_0", st2.st_size, 0);
    expect("mode_unchanged_0640", st2.st_mode, S_IFREG | 0640);
    expect("nlink_still_1", st2.st_nlink, 1);
    expect("getfl_wronly_largefile", syscall2(SYS_fcntl, fd, F_GETFL), O_WRONLY | O_LARGEFILE);

    /* The reader keeps its offset; nothing is left to read. */
    expect("reader_offset_kept", syscall3(SYS_lseek, rd, 0, SEEK_CUR), 4);
    expect("reader_eof", a_read(rd, buf, sizeof buf), 0);

    expect("write_new", a_write(fd, "ab", 2), 2);
    expect("reader_rewind", syscall3(SYS_lseek, rd, 0, SEEK_SET), 0);
    expect("reader_sees_new_2", a_read(rd, buf, sizeof buf), 2);
    check("reader_sees_new_data", buf[0] == 'a' && buf[1] == 'b', buf[0]);

    /* A second creat truncates under the first writer, whose offset stays. */
    i64 fd2 = cr(D "/f", 0600);
    expect("second_creat_fd_5", fd2, 5);
    expect("size_0_again", syscall2(SYS_fstat, fd, &st2) == 0 ? (i64)st2.st_size : -1, 0);
    expect("first_writer_offset_kept", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 2);
    expect("first_writer_writes_at_2", a_write(fd, "z", 1), 1);
    expect("size_3_hole", syscall2(SYS_fstat, fd, &st2) == 0 ? (i64)st2.st_size : -1, 3);
    expect("second_offset_0", syscall3(SYS_lseek, fd2, 0, SEEK_CUR), 0);
    a_close(fd2);
    a_close(fd);
    a_close(rd);

    /* Already empty: still opens, still size 0. */
    expect("empty_setup", a_make_file(D "/e", "", 0604), 0);
    fd = cr(D "/e", 0);
    expect("creat_empty_existing", fd, 3);
    expect("empty_mode_kept", syscall2(SYS_fstat, fd, &st2) == 0 ? (i64)st2.st_mode : -1, S_IFREG | 0604);
    a_close(fd);

    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_e", a_unlink(D "/e"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
