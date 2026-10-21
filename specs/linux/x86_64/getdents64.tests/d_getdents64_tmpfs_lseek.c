/* getdents64 on tmpfs and lseek: a record's d_off is the position just
 * after it, so lseek(fd, d_off, SEEK_SET) and a read return the next
 * record (nothing after the last one); lseek(fd, 0, SEEK_CUR) after a
 * call is the d_off of its last record; d_off values do not depend on
 * the buffer size and stay the same while other entries come and go
 * (only the d_off of the record before a new or removed entry changes);
 * a position whose entry has gone resumes with the next one. SEEK_END
 * and negative positions are EINVAL and leave the position alone.
 * poly-diff: syscalls=names (lseek gets the raw d_off cookies, which the
 * comparison does not pin down; only their behaviour is checked) */
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

#define D "/dev/shm/d_getdents64_tmpfs_lseek"

static char saved_name[MAXR][48];
static i64 saved_off[MAXR];
static int nsaved;

static void save(void) {
    nsaved = nrec;
    for (int i = 0; i < nrec; i++) {
        str_cpy(saved_name[i], rec_name[i]);
        saved_off[i] = rec_off[i];
    }
}

/* The record named `name` in the saved listing, or -1. */
static int saved_index(const char *name) {
    for (int i = 0; i < nsaved; i++)
        if (str_eq(saved_name[i], name)) return i;
    return -1;
}

int test_main(void) {
    check_tmpfs();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_files", mk_all(D, "a b c d e f"), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("read_all", read_all(fd), 0);
    show("listing", ". .. f e d c b a", "1 7 6 5 4 3 2 8");
    save();

    /* Seek to each d_off: the next record follows. */
    int seek_ret_ok = 1, next_ok = 1;
    for (int i = 0; i < nsaved; i++) {
        seek_ret_ok &= syscall3(SYS_lseek, fd, saved_off[i], SEEK_SET) == saved_off[i];
        i64 n = read_call(fd, 24);
        if (i + 1 < nsaved)
            next_ok &= n == 24 && str_eq(rec_name[0], saved_name[i + 1]) && rec_off[0] == saved_off[i + 1];
        else
            next_ok &= n == 0;
    }
    check("lseek_returns_d_off", seek_ret_ok, 0);
    check("lseek_d_off_reads_next", next_ok, 0);

    /* The position after each call is the last record's d_off. */
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    int pos_ok = 1;
    for (int i = 0; i < nsaved; i++) {
        read_call(fd, 24);
        pos_ok &= syscall3(SYS_lseek, fd, 0, SEEK_CUR) == saved_off[i];
    }
    check("position_is_last_d_off", pos_ok, 0);
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    read_call(fd, 72);
    check("position_after_three", syscall3(SYS_lseek, fd, 0, SEEK_CUR) == saved_off[2], 0);

    /* A remembered position (telldir/seekdir) gives the same rest. */
    i64 mark = syscall3(SYS_lseek, fd, 0, SEEK_CUR);
    expect("rest_from_mark", read_rest(fd, sizeof buf), 0);
    show("from_mark", "e d c b a", 0);
    check("lseek_mark", syscall3(SYS_lseek, fd, mark, SEEK_SET) == mark, 0);
    expect("rest_again", read_rest(fd, sizeof buf), 0);
    show("from_mark_again", "e d c b a", 0);

    /* Bad seeks: EINVAL, position unchanged. */
    syscall3(SYS_lseek, fd, mark, SEEK_SET);
    expect("seek_end_einval", syscall3(SYS_lseek, fd, 0, SEEK_END), -EINVAL);
    expect("seek_negative_einval", syscall3(SYS_lseek, fd, -1, SEEK_SET), -EINVAL);
    check("seek_cur_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_CUR) == mark, 0);
    expect("rest_after_bad_seeks", read_rest(fd, sizeof buf), 0);
    show("after_bad_seeks", "e d c b a", 0);
    check("position_at_end_is_last_d_off", syscall3(SYS_lseek, fd, 0, SEEK_CUR) == saved_off[nsaved - 1], 0);

    /* d_off values with 24-byte calls are the same as with one call. */
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    int same = 1;
    for (int i = 0; i < nsaved; i++) {
        read_call(fd, 24);
        same &= nrec == 1 && rec_off[0] == saved_off[i];
    }
    check("d_off_same_with_small_calls", same, 0);

    /* A new entry: only ".." gets a new d_off (the new entry's). */
    expect("create_g", mk(D, "g"), 0);
    expect("read_with_g", read_all(fd), 0);
    show("with_g", ". .. g f e d c b a", "1 8 7 6 5 4 3 2 9");
    int kept = rec_off[0] == saved_off[0] && rec_off[1] != saved_off[1] && rec_off[2] == saved_off[1];
    for (int i = 3; i < nrec; i++) kept &= rec_off[i] == saved_off[i - 1];
    check("d_off_kept_after_create", kept, 0);

    /* d removed: e's old d_off (d's position) now leads to c, the entry
     * after d; d's own d_off (c's position) still leads to c. */
    expect("unlink_d", a_unlink(D "/d"), 0);
    int e = saved_index("e"), d = saved_index("d");
    check("lseek_removed_position", syscall3(SYS_lseek, fd, saved_off[e], SEEK_SET) == saved_off[e], 0);
    expect("read_removed_position", read_rest(fd, sizeof buf), 0);
    show("from_removed_position", "c b a", 0);
    syscall3(SYS_lseek, fd, saved_off[d], SEEK_SET);
    expect("read_c_position", read_rest(fd, sizeof buf), 0);
    show("from_c_position", "c b a", 0);
    expect("read_without_d", read_all(fd), 0);
    show("without_d", ". .. g f e c b a", "1 7 6 5 4 3 2 8");
    check("e_d_off_now_c", rec_off[4] == saved_off[d], 0);

    /* The end position: 0 records, also after new entries. */
    syscall3(SYS_lseek, fd, saved_off[nsaved - 1], SEEK_SET);
    expect("end_position_0", read_call(fd, sizeof buf), 0);
    a_close(fd);

    expect("cleanup_files", rm_all(D, "a b c e f g"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
