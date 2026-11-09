/* The remaining read-only requests and how errors map. readlink sends
 * FUSE_READLINK (5) for the link's node and returns the reply's bytes (no
 * terminating NUL), also truncated to a small buffer. statfs sends
 * FUSE_STATFS (17) for the root. fstat of an open file asks FUSE_GETATTR
 * (3) for the file's node when the attributes are not valid (attr_valid 0). Replies with an error make
 * the call fail with that error: LOOKUP EIO/EACCES, GETATTR ESTALE (116),
 * READLINK EIO. A READ reply shorter than asked ends the data of that request:
 * the caller gets exactly the bytes the server returned (1000 of the 8192
 * asked here), they are the file's first bytes, and the kernel does not ask
 * again for the rest of that read. mmap(MAP_PRIVATE, PROT_READ) of a file reads through READ
 * requests and shows the file's bytes; truncating the tail is zero.
 * (linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define P "/tmp/pk_fuse_rmisc"
#define ESTALE 116

static u8 buf[300000];

int test_main(void) {
    expect("setup", fz_setup(P), 0);
    expect("open", fz_open() >= 0, 1);
    expect("mount", fz_mount(0), 0);
    expect("start", fz_start() > 0, 1);

    char l[32];
    memset(l, 'x', sizeof l);
    expect("readlink", syscall3(SYS_readlink, P "/link", l, sizeof l), 5);
    check("readlink_content", mem_eq(l, "hello", 5) && l[5] == 'x', 0);
    const struct fz_rec *rl = fz_last(FZ_READLINK);
    check("readlink_sent", rl != 0, 0);
    if (rl) expect("readlink_node", (i64)rl->nodeid, 4);
    expect("readlink_small_buffer", syscall3(SYS_readlink, P "/link", l, 3), 3);
    expect("readlink_on_file", syscall3(SYS_readlink, P "/hello", l, sizeof l), -EINVAL);

    struct a_statfs sf;
    expect("statfs", syscall2(SYS_statfs, P "/dir", &sf), 0);
    const struct fz_rec *sr = fz_last(FZ_STATFS);
    check("statfs_sent", sr != 0, 0);
    expect("statfs_blocks", (i64)sf.f_blocks, 1000);

    /* errors */
    struct a_stat st;
    fz->fail_op = FZ_LOOKUP;
    fz->fail_err = EIO;
    expect("lookup_eio", a_stat_path(P "/dir/inner", &st), -EIO);
    fz->fail_err = EACCES;
    expect("lookup_eacces", a_stat_path(P "/dir/inner", &st), -EACCES);
    fz->fail_op = 0;
    fz->fail_op = FZ_READLINK;
    fz->fail_err = EIO;
    expect("readlink_eio", syscall3(SYS_readlink, P "/link", l, sizeof l), -EIO);
    fz->fail_op = 0;

    /* fstat asks the server when attributes are not cached */
    fz->attr_valid = 0;
    i64 fd = a_open(P "/dir/inner", O_RDONLY, 0);
    i64 n;
    int ok;
    check("open_inner", fd >= 0, fd);
    int mark = fz_mark();
    expect("fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("fstat_size", st.st_size, 6);
    const struct fz_rec *ga = fz_nth_since(mark, FZ_GETATTR, 0);
    check("fstat_sent_getattr", ga != 0, 0);
    if (ga) {
        expect("getattr_node", (i64)ga->nodeid, 6);
    }
    fz->fail_op = FZ_GETATTR;
    fz->fail_err = ESTALE;
    expect("getattr_estale", syscall2(SYS_fstat, fd, &st), -ESTALE);
    fz->fail_op = 0;
    a_close(fd);

    /* short READ replies */
    fz->read_short = 1000;
    fd = a_open(P "/big", O_RDONLY, 0);
    check("open_big_short", fd >= 0, fd);
    n = a_read(fd, buf, 8192);
    expect("short_read_count", n, 1000);
    ok = 1;
    for (u64 i = 0; i < 1000; i++)
        if (buf[i] != fz_bigbyte(i)) ok = 0;
    expect("short_read_content", ok, 1);
    a_close(fd);
    fz->read_short = 0;

    /* mmap private */
    fd = a_open(P "/big", O_RDONLY, 0);
    check("open_big", fd >= 0, fd);
    i64 m = syscall6(SYS_mmap, 0, 3 * 4096, PROT_READ, MAP_PRIVATE, fd, 4096);
    check("mmap_private", m > 0 || m < -4095, m);
    if (m > 0) {
        const u8 *p = (const u8 *)m;
        ok = 1;
        for (u64 i = 0; i < 3 * 4096; i++)
            if (p[i] != fz_bigbyte(4096 + i)) ok = 0;
        expect("mmap_content", ok, 1);
        syscall2(SYS_munmap, m, 3 * 4096);
    }
    a_close(fd);
    fd = a_open(P "/dir/inner", O_RDONLY, 0);
    m = syscall6(SYS_mmap, 0, 4096, PROT_READ, MAP_PRIVATE, fd, 0);
    check("mmap_small", m > 0, m);
    if (m > 0) {
        const u8 *p = (const u8 *)m;
        check("mmap_small_content", mem_eq(p, "inner\n", 6) && p[6] == 0 && p[4095] == 0, 0);
        syscall2(SYS_munmap, m, 4096);
    }
    a_close(fd);
    fz_cleanup();
    return finish();
}
