/* getdents64 on tmpfs with buffers that hold one, two or three records:
 * each call returns as many whole records as fit and the next call goes
 * on with the record after the last one returned, so the listing is the
 * same, record for record and d_off for d_off, whatever the buffer size.
 * Every record here is 24 bytes (names of up to 4 bytes) but for one
 * 56-byte record (a 32-byte name) at the end: a buffer must hold the
 * whole next record (EINVAL otherwise, the position unchanged). */
#include "a_sys.h"

/* linux/magic.h */
#define TMPFS_MAGIC 0x01021994

static char buf[8192];

/* The records of the last read: name, d_off and d_type of each. */
#define MAXR 64
static char rec_name[MAXR][48];
static i64 rec_off[MAXR];
static u8 rec_type[MAXR];
static int nrec;

static A_UNUSED void take(i64 n) {
    for (i64 o = 0; o < n;) {
        struct a_dirent64 *d = (struct a_dirent64 *)(buf + o);
        if (nrec < MAXR) {
            str_cpy(rec_name[nrec], d->d_name);
            rec_off[nrec] = d->d_off;
            rec_type[nrec] = d->d_type;
            nrec++;
        }
        o += d->d_reclen;
    }
}

/* getdents64 calls of `size` bytes from the current position until one
 * returns 0 (or an error, which is returned). */
static A_UNUSED i64 read_rest(i64 fd, i64 size) {
    i64 n;
    nrec = 0;
    while ((n = syscall3(SYS_getdents64, fd, buf, size)) > 0) take(n);
    return n;
}

/* One getdents64 call of `size` bytes; returns its result. */
static A_UNUSED i64 read_call(i64 fd, i64 size) {
    nrec = 0;
    i64 n = syscall3(SYS_getdents64, fd, buf, size);
    if (n > 0) take(n);
    return n;
}

/* Rewind (lseek to 0) and read the whole listing. */
static A_UNUSED i64 read_all(i64 fd) {
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    return read_rest(fd, sizeof buf);
}

static char text[2048];

/* The names of the records, in the order returned, space-separated. */
static A_UNUSED const char *names_text(void) {
    char *p = text;
    *p = 0;
    for (int i = 0; i < nrec; i++) {
        if (i) p = str_cpy(p, " ");
        p = str_cpy(p, rec_name[i]);
    }
    return text;
}

/* Each record's d_off as its rank among the distinct d_off values of
 * these records (1 = the smallest): the raw values are not compared,
 * only their order. */
static A_UNUSED const char *ranks_text(void) {
    char *p = text, num[24];
    *p = 0;
    for (int i = 0; i < nrec; i++) {
        int rank = 1;
        for (int j = 0; j < nrec; j++) {
            if (rec_off[j] >= rec_off[i]) continue;
            int first = 1;
            for (int k = 0; k < j; k++)
                if (rec_off[k] == rec_off[j]) first = 0;
            rank += first;
        }
        if (i) p = str_cpy(p, " ");
        p = str_cpy(p, a_utoa(num, (u64)rank));
    }
    return text;
}

static char label_buf[128];

/* Print "<label>: <names>" and check the names; then, unless want_ranks
 * is 0, print "<label> d_off: <ranks>" and check those. */
static A_UNUSED void show(const char *label, const char *want_names, const char *want_ranks) {
    const char *got = names_text();
    put(label);
    put(": ");
    put(got);
    put("\n");
    check(str_cat2(label_buf, label, "_names"), str_eq(got, want_names), nrec);
    if (!want_ranks) return;
    got = ranks_text();
    put(label);
    put(" d_off: ");
    put(got);
    put("\n");
    check(str_cat2(label_buf, label, "_d_off"), str_eq(got, want_ranks), nrec);
}

/* "<dir>/<name>" in one of four rotating buffers. */
static A_UNUSED const char *at(const char *dir, const char *name) {
    static char paths[4][256];
    static int next;
    char *p = paths[next++ & 3];
    str_cpy(str_cpy(str_cpy(p, dir), "/"), name);
    return p;
}

static A_UNUSED i64 mk(const char *dir, const char *name) { return a_make_file(at(dir, name), "", 0644); }

static A_UNUSED i64 mv(const char *from_dir, const char *from, const char *to_dir, const char *to) {
    return syscall2(SYS_rename, at(from_dir, from), at(to_dir, to));
}

/* Create each space-separated name of `names` in `dir`, in that order;
 * returns how many failed. */
static A_UNUSED int mk_all(const char *dir, const char *names) {
    char one[32];
    int bad = 0;
    while (*names) {
        int n = 0;
        while (*names && *names != ' ') one[n++] = *names++;
        one[n] = 0;
        while (*names == ' ') names++;
        bad += mk(dir, one) != 0;
    }
    return bad;
}

/* Unlink each space-separated name of `names` in `dir`; returns how many
 * failed. */
static A_UNUSED int rm_all(const char *dir, const char *names) {
    char one[32];
    int bad = 0;
    while (*names) {
        int n = 0;
        while (*names && *names != ' ') one[n++] = *names++;
        one[n] = 0;
        while (*names == ' ') names++;
        bad += a_unlink(at(dir, one)) != 0;
    }
    return bad;
}

/* The test directories are on the tmpfs at /dev/shm (on Linux and on
 * Poly); say so if it is not one. */
static A_UNUSED void check_tmpfs(void) {
    struct a_statfs sf;
    i64 r = syscall2(SYS_statfs, "/dev/shm", &sf);
    check("setup_dev_shm_is_tmpfs", r == 0 && sf.f_type == TMPFS_MAGIC, r == 0 ? sf.f_type : r);
}

#define D "/dev/shm/d_getdents64_tmpfs_multi_call"

static char want[MAXR][48];
static i64 want_off[MAXR];
static int nwant;

/* The same names and d_off values as the reference listing. */
static int same_as_want(void) {
    if (nrec != nwant) return 0;
    for (int i = 0; i < nrec; i++)
        if (!str_eq(rec_name[i], want[i]) || rec_off[i] != want_off[i]) return 0;
    return 1;
}

/* Rewind and read with `size`-byte calls, checking each returns `per`
 * records (the last call may return fewer). */
static void read_by(i64 fd, i64 size, int per, const char *label) {
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    char all_names[MAXR][48];
    i64 all_off[MAXR];
    int total = 0, calls = 0, sizes_ok = 1;
    i64 n;
    while ((n = read_call(fd, size)) > 0) {
        calls++;
        sizes_ok &= n == 24 * nrec && (nrec == per || total + nrec == nwant);
        for (int i = 0; i < nrec && total < MAXR; i++, total++) {
            str_cpy(all_names[total], rec_name[i]);
            all_off[total] = rec_off[i];
        }
    }
    nrec = total;
    for (int i = 0; i < total; i++) {
        str_cpy(rec_name[i], all_names[i]);
        rec_off[i] = all_off[i];
    }
    check(str_cat2(label_buf, label, "_end_0"), n == 0, n);
    check(str_cat2(label_buf, label, "_records_per_call"), sizes_ok, calls);
    check(str_cat2(label_buf, label, "_same_records"), same_as_want(), nrec);
    put(label);
    put(" calls: ");
    put_num(calls);
    put("\n");
}

int test_main(void) {
    check_tmpfs();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_files", mk_all(D, "mm cc xx aa zz bb yy dd"), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("one_call_bytes", read_call(fd, sizeof buf), 10 * 24);
    show("one_call", ". .. dd yy bb zz aa xx cc mm", "1 9 8 7 6 5 4 3 2 10");
    nwant = nrec;
    for (int i = 0; i < nrec; i++) {
        str_cpy(want[i], rec_name[i]);
        want_off[i] = rec_off[i];
    }
    expect("one_call_then_0", read_call(fd, sizeof buf), 0);

    read_by(fd, 24, 1, "size24");
    show("size24", ". .. dd yy bb zz aa xx cc mm", "1 9 8 7 6 5 4 3 2 10");
    read_by(fd, 47, 1, "size47");
    read_by(fd, 48, 2, "size48");
    read_by(fd, 71, 2, "size71");
    read_by(fd, 72, 3, "size72");

    /* Too small for the next record: EINVAL, and the position stays. */
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    expect("size23_einval", read_call(fd, 23), -EINVAL);
    expect("then_dot", read_call(fd, 24), 24);
    show("after_einval", ".", "1");
    expect("size1_einval", read_call(fd, 1), -EINVAL);
    expect("then_dotdot", read_call(fd, 24), 24);
    show("after_einval2", "..", 0);

    /* A longer name needs a longer record: a buffer that holds two
     * short records but not the long one returns the short ones only. */
    expect("setup_long", mk(D, "a_name_of_thirty_two_characters_"), 0);
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    expect("long_first_call", read_call(fd, 48), 48);
    show("long_first", ". ..", 0);
    expect("long_too_small", read_call(fd, 48), -EINVAL);
    expect("long_fits", read_call(fd, 56), 56);
    show("long_second", "a_name_of_thirty_two_characters_", 0);
    expect("long_rest", read_rest(fd, 56), 0);
    show("long_rest", "dd yy bb zz aa xx cc mm", 0);
    a_close(fd);

    expect("cleanup_files", rm_all(D, "mm cc xx aa zz bb yy dd a_name_of_thirty_two_characters_"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
