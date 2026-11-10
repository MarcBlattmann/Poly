/* utimensat on a fuse file and renameat2(RENAME_NOREPLACE) onto a free
 * name: which request the server sees.
 * Times: one FUSE_SETATTR (4) per call. A time given as UTIME_NOW sets two
 * bits of `valid`, the plain one and the NOW one: FATTR_ATIME (16) with
 * FATTR_ATIME_NOW (128) for the access time, FATTR_MTIME (32) with
 * FATTR_MTIME_NOW (256) for the modification time (the request then
 * carries the kernel's current time in that time's fields; the value is
 * not checked here). A NULL times pointer is both times as UTIME_NOW
 * (valid 432). An explicit time
 * sets only the plain bit with the seconds and nanoseconds given.
 * UTIME_OMIT leaves that time's bits out and its fields 0; both times
 * omitted sends no request at all (the call is 0). No other bit is set
 * (no FATTR_FH, no FATTR_CTIME, ctime 0).
 * Rename: RENAME_NOREPLACE onto a name that does not exist sends
 * FUSE_RENAME2 (45) {newdir, flags 1} with both names, not FUSE_RENAME
 * (12); a renameat2 with flags 0 sends FUSE_RENAME.
 * (linux/fuse.h, utimensat(2), rename(2))
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define P "/tmp/pk_fuse_wtimes"

/* The server copies the time fields of the last SETATTR into the shared
 * block (junk_res is free in this test: init_junk_first is not used). */
static int keep_times(const struct fz_in *h, const u8 *pl) {
    if (h->opcode == FZ_SETATTR) {
        const struct fz_setattr_in *s = (const void *)pl;
        fz->junk_res[0] = (i64)s->atime;
        fz->junk_res[1] = s->atimensec;
        fz->junk_res[2] = (i64)s->mtime;
        fz->junk_res[3] = s->mtimensec;
        fz->junk_res[4] = (i64)s->ctime;
    }
    return 0;
}

/* One utimensat call: the result, the number of SETATTR requests and `valid`. */
static void times_case(const char *name, const struct a_timespec *ts, i64 want_valid) {
    char chk[64];
    for (int i = 0; i < 5; i++) fz->junk_res[i] = -1;
    int m = fz_mark();
    expect(str_cat2(chk, name, "_call"), syscall4(SYS_utimensat, AT_FDCWD, P "/dir/inner", ts, 0), 0);
    expect(str_cat2(chk, name, "_one_setattr"), fz_count_since(m, FZ_SETATTR), 1);
    const struct fz_rec *sa = fz_nth_since(m, FZ_SETATTR, 0);
    if (sa) expect(str_cat2(chk, name, "_valid"), (i64)sa->a[0], want_valid);
}

int test_main(void) {
    fz_hook = keep_times;
    expect("setup", fz_setup(P), 0);
    expect("open", fz_open() >= 0, 1);
    expect("mount", fz_mount(0), 0);
    expect("start", fz_start() > 0, 1);
    struct a_timespec ts[2];

    times_case("null", 0, FZ_FATTR_ATIME | FZ_FATTR_MTIME | FZ_FATTR_ATIME_NOW | FZ_FATTR_MTIME_NOW);
    expect("null_ctime_field_zero", fz->junk_res[4], 0);

    ts[0].tv_sec = 0; ts[0].tv_nsec = UTIME_NOW;
    ts[1].tv_sec = 0; ts[1].tv_nsec = UTIME_NOW;
    times_case("both_now", ts, FZ_FATTR_ATIME | FZ_FATTR_MTIME | FZ_FATTR_ATIME_NOW | FZ_FATTR_MTIME_NOW);

    ts[0].tv_sec = 0; ts[0].tv_nsec = UTIME_OMIT;
    ts[1].tv_sec = 0; ts[1].tv_nsec = UTIME_NOW;
    times_case("mtime_now", ts, FZ_FATTR_MTIME | FZ_FATTR_MTIME_NOW);
    expect("mtime_now_atime_field_zero", fz->junk_res[0], 0);
    expect("mtime_now_atimensec_zero", fz->junk_res[1], 0);

    ts[0].tv_sec = 86400; ts[0].tv_nsec = 123456789;
    ts[1].tv_sec = 172800; ts[1].tv_nsec = 987654321;
    times_case("both_given", ts, FZ_FATTR_ATIME | FZ_FATTR_MTIME);
    expect("given_atime", fz->junk_res[0], 86400);
    expect("given_atimensec", fz->junk_res[1], 123456789);
    expect("given_mtime", fz->junk_res[2], 172800);
    expect("given_mtimensec", fz->junk_res[3], 987654321);
    struct a_stat st;
    expect("stat_after", a_stat_path(P "/dir/inner", &st), 0);
    expect("stat_mtime_is_the_reply", (i64)st.st_mtime_sec, 172800);

    ts[0].tv_sec = 4242; ts[0].tv_nsec = 7;
    ts[1].tv_sec = 0; ts[1].tv_nsec = UTIME_NOW;
    times_case("given_and_now", ts, FZ_FATTR_ATIME | FZ_FATTR_MTIME | FZ_FATTR_MTIME_NOW);
    expect("given_and_now_atime", fz->junk_res[0], 4242);
    expect("given_and_now_atimensec", fz->junk_res[1], 7);

    ts[0].tv_sec = 0; ts[0].tv_nsec = UTIME_NOW;
    ts[1].tv_sec = 31536000; ts[1].tv_nsec = 0;
    times_case("now_and_given", ts, FZ_FATTR_ATIME | FZ_FATTR_MTIME | FZ_FATTR_ATIME_NOW);
    expect("now_and_given_mtime", fz->junk_res[2], 31536000);
    expect("now_and_given_mtimensec", fz->junk_res[3], 0);

    ts[0].tv_sec = 99; ts[0].tv_nsec = 1;
    ts[1].tv_sec = 5; ts[1].tv_nsec = UTIME_OMIT;
    times_case("atime_only", ts, FZ_FATTR_ATIME);
    expect("atime_only_atime", fz->junk_res[0], 99);
    expect("atime_only_mtime_field_zero", fz->junk_res[2], 0);

    ts[0].tv_sec = 1; ts[0].tv_nsec = UTIME_OMIT;
    ts[1].tv_sec = 2; ts[1].tv_nsec = UTIME_OMIT;
    int m = fz_mark();
    expect("both_omitted_call", syscall4(SYS_utimensat, AT_FDCWD, P "/dir/inner", ts, 0), 0);
    expect("both_omitted_no_request", fz_count_since(m, FZ_SETATTR), 0);

    /* through a descriptor (futimens): the same bits, no FATTR_FH */
    i64 fd = a_open(P "/dir/inner", O_WRONLY, 0);
    check("open_for_futimens", fd >= 0, fd);
    m = fz_mark();
    expect("futimens_null", syscall4(SYS_utimensat, fd, 0, 0, 0), 0);
    const struct fz_rec *sa = fz_nth_since(m, FZ_SETATTR, 0);
    check("futimens_setattr_sent", sa != 0, 0);
    if (sa) expect("futimens_valid", (i64)sa->a[0], FZ_FATTR_ATIME | FZ_FATTR_MTIME | FZ_FATTR_ATIME_NOW | FZ_FATTR_MTIME_NOW);
    a_close(fd);

    /* RENAME_NOREPLACE onto a free name */
    struct a_stat dir;
    expect("stat_dir", a_stat_path(P "/dir", &dir), 0);
    expect("make_src", a_make_file(P "/src1", "one", 0644), 0);
    m = fz_mark();
    expect("noreplace_free_name", syscall5(SYS_renameat2, AT_FDCWD, P "/src1", AT_FDCWD, P "/dir/dst1", RENAME_NOREPLACE), 0);
    expect("noreplace_sends_rename2", fz_count_since(m, FZ_RENAME2), 1);
    expect("noreplace_sends_no_rename", fz_count_since(m, FZ_RENAME), 0);
    const struct fz_rec *r2 = fz_nth_since(m, FZ_RENAME2, 0);
    if (r2) {
        expect("rename2_flags_noreplace", (i64)r2->a[1], RENAME_NOREPLACE);
        expect("rename2_old_parent", (i64)r2->nodeid, 1);
        expect("rename2_newdir", (i64)r2->a[0], (i64)dir.st_ino);
        check("rename2_old_name", str_eq(r2->name, "src1"), 0);
        check("rename2_new_name", str_eq(r2->aux, "dst1"), 0);
    }
    expect("old_name_gone", a_stat_path(P "/src1", &st), -ENOENT);
    expect("new_name_there", a_stat_path(P "/dir/dst1", &st) == 0 ? st.st_size : -1, 3);
    m = fz_mark();
    expect("renameat2_flags_0", syscall5(SYS_renameat2, AT_FDCWD, P "/dir/dst1", AT_FDCWD, P "/back", 0), 0);
    expect("flags_0_sends_rename", fz_count_since(m, FZ_RENAME), 1);
    expect("flags_0_sends_no_rename2", fz_count_since(m, FZ_RENAME2), 0);
    fz_cleanup();
    return finish();
}
