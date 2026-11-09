/* The FUSE_INIT request. It is the first request the server reads after
 * the mount, and it arrives without any user operation on the mount, once
 * per connection. Header: opcode 26, nodeid 0, a nonzero even unique, the
 * mounting user's uid and gid (0 and 0 here), len = 40 + the body. Body
 * (struct fuse_init_in): major 7, minor at least 31, a nonzero
 * max_readahead, flags that include FUSE_ASYNC_READ (bit 0),
 * FUSE_ATOMIC_O_TRUNC (bit 3) and FUSE_MAX_PAGES (bit 22). The body is at
 * least 16 bytes. Requests that follow have larger unique numbers, and no
 * second INIT is ever sent.
 * (linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define P "/tmp/pk_fuse_ireq"

int test_main(void) {
    expect("setup", fz_setup(P), 0);
    expect("open", fz_open() >= 0, 1);
    expect("mount", fz_mount(0), 0);
    expect("start", fz_start() > 0, 1);

    /* No client operation yet: the server still gets INIT by itself. */
    check("init_arrives_unprompted", fz_wait_count(FZ_INIT, 1, 3000) == 1, fz_count(FZ_INIT));
    for (int i = 0; i < 200 && fz->init_count == 0; i++) fz_msleep(5);
    expect("init_count_one", fz->init_count, 1);

    const struct fz_in *h = &fz->init_hdr;
    expect("opcode", h->opcode, FZ_INIT);
    expect("nodeid_zero", (i64)h->nodeid, 0);
    check("unique_nonzero_even", h->unique != 0 && (h->unique & 1) == 0, (i64)h->unique);
    expect("uid", h->uid, 0);
    expect("gid", h->gid, 0);
    check("len_covers_body", h->len >= 40 + 16, h->len);
    const struct fz_init_in *in = &fz->init_in;
    expect("major", in->major, 7);
    check("minor_at_least_31", in->minor >= 31, in->minor);
    check("max_readahead_nonzero", in->max_readahead > 0, in->max_readahead);
    check("flag_async_read", (in->flags & FZ_ASYNC_READ) != 0, in->flags);
    check("flag_atomic_o_trunc", (in->flags & FZ_ATOMIC_O_TRUNC) != 0, in->flags);
    check("flag_max_pages", (in->flags & FZ_MAX_PAGES) != 0, in->flags);

    /* Operations follow; INIT stays the first request, and the only one. */
    struct a_stat st;
    expect("stat", a_stat_path(P "/hello", &st), 0);
    expect("stat_dir", a_stat_path(P "/dir", &st), 0);
    check("init_first_in_log", fz->n > 0 && fz->rec[0].op == FZ_INIT, fz->n);
    expect("init_only_once", fz_count(FZ_INIT), 1);
    const struct fz_rec *l = fz_nth(FZ_LOOKUP, 0);
    check("later_requests_exist", l != 0, 0);
    expect("init_count_still_one", fz->init_count, 1);
    fz_cleanup();
    return finish();
}
