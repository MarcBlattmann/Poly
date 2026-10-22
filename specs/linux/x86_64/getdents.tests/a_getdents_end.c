/* getdents at the end of a directory: an empty directory lists just "."
 * and ".." (both DT_DIR), then 0, again and again. At the end, nothing is
 * stored, so a size of 0 and a NULL buffer also give 0 (not EINVAL or
 * EFAULT). lseek(fd, 0, SEEK_SET) starts the listing over. */
#include "a_sys.h"

#define SYS_getdents 78

struct g_dirent {
    u64 d_ino;
    u64 d_off;
    u16 d_reclen;
    char d_name[];
};

#define D "/tmp/a_getdents_end"

static char buf[4096] __attribute__((aligned(8)));

static int dot_and_dotdot(i64 n) {
    int dot = 0, dotdot = 0, other = 0;
    for (i64 off = 0; off < n;) {
        struct g_dirent *d = (struct g_dirent *)(buf + off);
        u8 type = (u8)buf[off + d->d_reclen - 1];
        if (str_eq(d->d_name, ".") && type == DT_DIR && d->d_reclen == 24)
            dot++;
        else if (str_eq(d->d_name, "..") && type == DT_DIR && d->d_reclen == 24)
            dotdot++;
        else
            other++;
        if (d->d_reclen == 0) return 0;
        off += d->d_reclen;
    }
    return dot == 1 && dotdot == 1 && other == 0;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i64 n = syscall3(SYS_getdents, fd, buf, sizeof buf);
    expect("empty_dir_bytes", n, 48);
    check("empty_dir_dot_and_dotdot", dot_and_dotdot(n), 0);
    expect("end_0", syscall3(SYS_getdents, fd, buf, sizeof buf), 0);
    expect("end_0_again", syscall3(SYS_getdents, fd, buf, sizeof buf), 0);
    expect("end_size_0", syscall3(SYS_getdents, fd, buf, 0), 0);
    expect("end_size_1", syscall3(SYS_getdents, fd, buf, 1), 0);
    expect("end_null_buffer", syscall3(SYS_getdents, fd, 0, 4096), 0);
    expect("end_bad_buffer", syscall3(SYS_getdents, fd, 0x1000, 4096), 0);

    expect("rewind", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    n = syscall3(SYS_getdents, fd, buf, sizeof buf);
    expect("after_rewind_bytes", n, 48);
    check("after_rewind_dot_and_dotdot", dot_and_dotdot(n), 0);
    expect("after_rewind_end", syscall3(SYS_getdents, fd, buf, sizeof buf), 0);
    a_close(fd);

    /* The root directory lists something, and has "." and "..". */
    fd = a_open("/", O_RDONLY | O_DIRECTORY, 0);
    int dot = 0, dotdot = 0, tmp = 0;
    for (int guard = 0; guard < 100; guard++) {
        n = syscall3(SYS_getdents, fd, buf, sizeof buf);
        if (n <= 0) break;
        for (i64 off = 0; off < n;) {
            struct g_dirent *d = (struct g_dirent *)(buf + off);
            dot += str_eq(d->d_name, ".");
            dotdot += str_eq(d->d_name, "..");
            tmp += str_eq(d->d_name, "tmp") && (u8)buf[off + d->d_reclen - 1] == DT_DIR;
            if (d->d_reclen == 0) break;
            off += d->d_reclen;
        }
    }
    expect("root_listing_ends_with_0", n, 0);
    expect("root_has_dot", dot, 1);
    expect("root_has_dotdot", dotdot, 1);
    expect("root_has_tmp_dir", tmp, 1);
    a_close(fd);

    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
