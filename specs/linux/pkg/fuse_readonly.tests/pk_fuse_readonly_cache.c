/* Cache validity in replies. entry_valid is how long a looked-up name is
 * trusted, attr_valid how long the attributes are. With 0 for both,
 * a stat some milliseconds after another asks the server again (FUSE_LOOKUP
 * for the name, FUSE_GETATTR for the attributes). With a long validity
 * (1000 s) repeated stats send nothing more: one LOOKUP in all, no
 * GETATTR (the lookup reply's attributes are used). With entry_valid long
 * and attr_valid 0 the name is looked up once, the attributes asked for
 * on each stat. A negative lookup is cached only when the server answers
 * with node id 0 and an entry_valid: then a second stat of the missing
 * name sends no LOOKUP, otherwise (ENOENT error reply) every stat does.
 * (linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define P "/tmp/pk_fuse_rcache"

static void run(const char *tag, u64 entry_valid, u64 attr_valid, u64 neg_valid, int *lookups, int *getattrs, int *neg_lookups) {
    (void)tag;
    fz->entry_valid = entry_valid;
    fz->attr_valid = attr_valid;
    fz->neg_valid = neg_valid;
    fz_open();
    fz_mount(0);
    fz_start();
    struct a_stat st;
    int m = fz_mark();
    for (int i = 0; i < 4; i++) {
        a_stat_path(P "/hello", &st);
        fz_msleep(40);
    }
    *lookups = fz_count_since(m, FZ_LOOKUP);
    *getattrs = fz_count_since(m, FZ_GETATTR);
    m = fz_mark();
    for (int i = 0; i < 3; i++) {
        i64 r = a_stat_path(P "/missing", &st);
        if (r != -ENOENT) check("missing_is_enoent", 0, r);
        fz_msleep(40);
    }
    *neg_lookups = fz_count_since(m, FZ_LOOKUP);
    syscall2(SYS_umount2, P, 0);
    fz_wait();
}

int test_main(void) {
    expect("setup", fz_setup(P), 0);
    int lk, ga, ng;

    run("zero", 0, 0, 0, &lk, &ga, &ng);
    expect("zero_lookup_each_stat", lk, 4);
    check("zero_getattr_asked", ga >= 3, ga);
    expect("zero_negative_each_stat", ng, 3);

    run("long", 1000, 1000, 1000, &lk, &ga, &ng);
    expect("long_one_lookup", lk, 1);
    expect("long_no_getattr", ga, 0);
    expect("long_negative_cached_one_lookup", ng, 1);

    run("entry_only", 1000, 0, 0, &lk, &ga, &ng);
    expect("entry_only_one_lookup", lk, 1);
    check("entry_only_getattr_each_stat", ga >= 3, ga);
    expect("entry_only_error_reply_not_cached", ng, 3);

    run("attr_only", 0, 1000, 0, &lk, &ga, &ng);
    expect("attr_only_lookup_each_stat", lk, 4);
    expect("attr_only_no_getattr", ga, 0);
    fz_cleanup();
    return finish();
}
