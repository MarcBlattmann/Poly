/* getxattr changes nothing: the file's mode, size, contents, atime, mtime
 * and ctime stay the same after reads, ERANGE and ENODATA results. Only
 * the value's bytes are written to the buffer: a buffer that ends exactly
 * where the mapping ends works even with a larger size argument; a buffer
 * that runs into an unmapped page before the value ends, or starts in one,
 * is EFAULT. */
#include "a_sys.h"

#define SYS_mmap 9
#define SYS_munmap 11
#define SYS_setxattr 188
#define SYS_getxattr 191
#define ENODATA 61
#define D "/tmp/a_getxattr_stat"

static char buf[64];

static i64 get(const char *name, void *b, i64 size) { return syscall4(SYS_getxattr, D "/f", name, b, size); }

static int same(const struct a_stat *a, const struct a_stat *b) {
    return a->st_mode == b->st_mode && a->st_size == b->st_size && a->st_atime_sec == b->st_atime_sec &&
           a->st_atime_nsec == b->st_atime_nsec && a->st_mtime_sec == b->st_mtime_sec &&
           a->st_mtime_nsec == b->st_mtime_nsec && a->st_ctime_sec == b->st_ctime_sec &&
           a->st_ctime_nsec == b->st_ctime_nsec;
}

int test_main(void) {
    struct a_stat before, after;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "contents", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.a", "abc", 3, 0), 0);
    expect("stat_before", a_stat_path(D "/f", &before), 0);
    expect("read", get("user.a", buf, sizeof buf), 3);
    expect("read_size0", get("user.a", 0, 0), 3);
    expect("read_erange", get("user.a", buf, 1), -ERANGE);
    expect("read_missing", get("user.b", buf, sizeof buf), -ENODATA);
    expect("stat_after", a_stat_path(D "/f", &after), 0);
    check("stat_unchanged", same(&before, &after), 0);
    i64 m = syscall6(SYS_mmap, 0, 8192, 3, 0x22, -1, 0); /* PROT_READ|WRITE, MAP_PRIVATE|ANONYMOUS */
    check("setup_mmap", m > 0, m);
    char *page = (char *)m;
    expect("setup_unmap_second", syscall2(SYS_munmap, page + 4096, 4096), 0);
    expect("edge_exact", get("user.a", page + 4093, 3), 3);
    check("edge_value", mem_eq(page + 4093, "abc", 3), 0);
    expect("edge_exact_larger_size", get("user.a", page + 4093, 64), 3);
    expect("edge_over_efault", get("user.a", page + 4094, 3), -EFAULT);
    expect("edge_over_larger_size_efault", get("user.a", page + 4095, 64), -EFAULT);
    expect("unmapped_efault", get("user.a", page + 4096, 3), -EFAULT);
    expect("unmapped_size0", get("user.a", page + 4096, 0), 3);
    expect("cleanup_unmap", syscall2(SYS_munmap, page, 4096), 0);
    expect("stat_end", a_stat_path(D "/f", &after), 0);
    check("stat_unchanged_end", same(&before, &after), 0);
    /* Last: reading the contents may move the atime. */
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    char data[16];
    expect("contents_unchanged", a_read(fd, data, sizeof data), 8);
    check("contents_value", mem_eq(data, "contents", 8), 0);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
