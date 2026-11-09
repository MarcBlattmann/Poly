/* Changing the content of a lower-layer file through the merged view first
 * copies the file to the upper layer: write, append (O_APPEND), O_TRUNC,
 * truncate(2) and ftruncate(2) all do it; a regular file opened O_RDWR or
 * O_WRONLY is copied at open time, before any write; a file opened O_RDONLY
 * is not copied, nor is one only stat'ed or read. After the copy the upper
 * file has the new content, the lower file keeps the old one, and the
 * copy has the lower file's mode, owner and size. Large files (more than
 * one page, with holes) are copied byte for byte; the merged file reads
 * the same as before the change. New files appear only in the upper layer.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovc_data"
#define M B "/m"
#define U B "/u"
#define L B "/l"

static char big[3 * A_PAGE];

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(L);
    pm_mkdirs(U);
    pm_mkdirs(B "/w");
    pm_mkdirs(M);
    pm_put(B, "/l/rd", "readonly-file", 0644);
    pm_put(B, "/l/wr", "0123456789", 0640);
    syscall3(SYS_chown, L "/wr", 11, 12);
    pm_put(B, "/l/app", "base", 0644);
    pm_put(B, "/l/trunc_open", "to-be-cleared", 0644);
    pm_put(B, "/l/trunc_path", "abcdefghij", 0644);
    pm_put(B, "/l/trunc_fd", "abcdefghij", 0644);
    pm_put(B, "/l/rdwr", "rdwr-content", 0644);
    pm_put(B, "/l/wronly", "wronly-content", 0644);
    pm_put(B, "/l/grow", "xy", 0644);
    /* a three page file: page 0 'A', page 1 a hole, page 2 'C' */
    i64 fd = a_open(L "/big", O_WRONLY | O_CREAT, 0644);
    memset(big, 'A', A_PAGE);
    a_write(fd, big, A_PAGE);
    syscall3(SYS_lseek, fd, 2 * A_PAGE, SEEK_SET);
    memset(big, 'C', A_PAGE);
    a_write(fd, big, A_PAGE);
    a_close(fd);

    const char *opts = "lowerdir=" L ",upperdir=" U ",workdir=" B "/w";
    expect("mount", pm_mount("ov", M, "overlay", 0, opts), 0);

    /* reading and opening for reading never copies */
    struct a_stat st;
    expect("stat_rd", a_stat_path(M "/rd", &st), 0);
    fd = a_open(M "/rd", O_RDONLY, 0);
    check("open_rdonly", fd >= 0, fd);
    char buf[32];
    memset(buf, 0, sizeof buf);
    expect("read_rd", a_read(fd, buf, 13), 13);
    check("rd_content", str_eq(buf, "readonly-file"), 0);
    a_close(fd);
    expect("rd_not_copied", pm_exists(U "/rd"), 0);

    /* write at an offset */
    fd = a_open(M "/wr", O_WRONLY, 0);
    check("open_wronly_wr", fd >= 0, fd);
    expect("wr_copied_at_open", pm_exists(U "/wr"), 1);
    pm_expect_file("wr_unchanged_at_open", U "/wr", "0123456789");
    expect("pwrite", syscall4(SYS_pwrite64, fd, "XYZ", 3, 4), 3);
    a_close(fd);
    pm_expect_file("wr_merged", M "/wr", "0123XYZ789");
    pm_expect_file("wr_upper", U "/wr", "0123XYZ789");
    pm_expect_file("wr_lower_untouched", L "/wr", "0123456789");
    expect("wr_upper_stat", a_stat_path(U "/wr", &st), 0);
    expect("wr_upper_mode", st.st_mode, S_IFREG | 0640);
    expect("wr_upper_uid", st.st_uid, 11);
    expect("wr_upper_gid", st.st_gid, 12);
    expect("wr_upper_size", st.st_size, 10);
    expect("wr_merged_stat", a_stat_path(M "/wr", &st), 0);
    expect("wr_merged_uid", st.st_uid, 11);

    /* O_RDWR and O_WRONLY copy at open, even without a write */
    fd = a_open(M "/rdwr", O_RDWR, 0);
    check("open_rdwr", fd >= 0, fd);
    expect("rdwr_copied_at_open", pm_exists(U "/rdwr"), 1);
    a_close(fd);
    fd = a_open(M "/wronly", O_WRONLY, 0);
    check("open_wronly", fd >= 0, fd);
    expect("wronly_copied_at_open", pm_exists(U "/wronly"), 1);
    a_close(fd);
    pm_expect_file("rdwr_content_kept", U "/rdwr", "rdwr-content");
    pm_expect_file("wronly_content_kept", U "/wronly", "wronly-content");

    /* append */
    fd = a_open(M "/app", O_WRONLY | O_APPEND, 0);
    check("open_append", fd >= 0, fd);
    expect("append_write", a_write(fd, "+more", 5), 5);
    a_close(fd);
    pm_expect_file("append_merged", M "/app", "base+more");
    pm_expect_file("append_upper", U "/app", "base+more");
    pm_expect_file("append_lower", L "/app", "base");

    /* O_TRUNC, truncate, ftruncate */
    fd = a_open(M "/trunc_open", O_WRONLY | O_TRUNC, 0);
    check("open_trunc", fd >= 0, fd);
    a_close(fd);
    expect("trunc_open_size", a_stat_path(M "/trunc_open", &st), 0);
    expect("trunc_open_empty", st.st_size, 0);
    pm_expect_file("trunc_open_lower", L "/trunc_open", "to-be-cleared");
    expect("trunc_open_in_upper", pm_exists(U "/trunc_open"), 1);
    expect("truncate_path", syscall2(SYS_truncate, M "/trunc_path", 4), 0);
    pm_expect_file("truncate_merged", M "/trunc_path", "abcd");
    pm_expect_file("truncate_upper", U "/trunc_path", "abcd");
    pm_expect_file("truncate_lower", L "/trunc_path", "abcdefghij");
    fd = a_open(M "/trunc_fd", O_RDWR, 0);
    expect("ftruncate", syscall2(SYS_ftruncate, fd, 6), 0);
    a_close(fd);
    pm_expect_file("ftruncate_merged", M "/trunc_fd", "abcdef");
    pm_expect_file("ftruncate_lower", L "/trunc_fd", "abcdefghij");
    expect("truncate_grow", syscall2(SYS_truncate, M "/grow", 6), 0);
    expect("grow_stat", a_stat_path(M "/grow", &st), 0);
    expect("grow_size", st.st_size, 6);
    char gb[8];
    memset(gb, 1, sizeof gb);
    fd = a_open(M "/grow", O_RDONLY, 0);
    expect("grow_read", a_read(fd, gb, 8), 6);
    a_close(fd);
    check("grow_content", gb[0] == 'x' && gb[1] == 'y' && gb[2] == 0 && gb[5] == 0, gb[2]);

    /* a large file with a hole is copied whole */
    fd = a_open(M "/big", O_RDWR, 0);
    check("open_big", fd >= 0, fd);
    expect("big_pwrite", syscall4(SYS_pwrite64, fd, "Z", 1, A_PAGE + 10), 1);
    a_close(fd);
    expect("big_upper_stat", a_stat_path(U "/big", &st), 0);
    expect("big_upper_size", st.st_size, 3 * A_PAGE);
    fd = a_open(M "/big", O_RDONLY, 0);
    int okc = 1;
    for (int pg = 0; pg < 3; pg++) {
        memset(big, 9, A_PAGE);
        i64 n = syscall4(SYS_pread64, fd, big, A_PAGE, pg * A_PAGE);
        if (n != A_PAGE) okc = 0;
        for (int i = 0; i < A_PAGE; i++) {
            char want = pg == 0 ? 'A' : pg == 2 ? 'C' : (i == 10 ? 'Z' : 0);
            if (big[i] != want) okc = 0;
        }
    }
    a_close(fd);
    check("big_content", okc, 0);
    fd = a_open(L "/big", O_RDONLY, 0);
    memset(big, 9, A_PAGE);
    syscall4(SYS_pread64, fd, big, A_PAGE, A_PAGE);
    a_close(fd);
    check("big_lower_hole_untouched", big[10] == 0, big[10]);

    /* new files exist in the upper layer only */
    expect("new_file", a_make_file(M "/brand_new", "fresh", 0600), 0);
    pm_expect_file("new_in_upper", U "/brand_new", "fresh");
    expect("new_not_in_lower", pm_exists(L "/brand_new"), 0);
    expect("new_mode", pm_mode(M "/brand_new"), S_IFREG | 0600);
    expect("umount", pm_umount(M, 0), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(B);
    pm_in_ns(body);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
