/* The largest ids and counts a uid_map / gid_map line may hold, written by
 * the root parent of a new user namespace. The id 4294967295 (the "no id"
 * value) can never be part of a range: for both the inside and the outside
 * start, start + count may be at most 4294967295, so the last id of a
 * range is at most 4294967294. "4294967294 0 1" and "5 5 4294967290" are
 * the largest that fit; one more in the start or in the count is EINVAL,
 * for the inside and for the outside alike, and a start of 4294967295 is
 * EINVAL with any count. A number above 4294967295 is not refused for
 * its size: only its low 32 bits count (4294967296 is 0, 4294967301 is 5),
 * so as a count 4294967296 is EINVAL (a count of 0) and as an id it is 0.
 * The numbers are decimal: a
 * leading 0 does not make them octal ("010" is ten) and "0x10" is EINVAL.
 * A refused write leaves the map empty, so a correct write afterwards
 * still works. gid_map follows the same rules.
 * poly-diff: needs=root (CAP_SETUID in the parent namespace)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"

static char got[256];

/* A child in a new user namespace that waits; the parent writes `text` to
 * its `file`, then (when `second` is given) writes that too, reads the map
 * back into `got`, and lets the child go. Returns the first write's result;
 * *r2 gets the second's. */
static i64 map_write(const char *file, const char *text, const char *second, i64 *r2) {
    i32 hold[2];
    if (p_pipe(hold) < 0) return -9999;
    i64 pid = p_clone(CLONE_NEWUSER);
    if (pid == 0) {
        a_close(hold[1]);
        p_wait_byte(hold[0]);
        p_exit(0);
    }
    a_close(hold[0]);
    char path[48];
    p_proc_path(path, pid, file);
    i64 r = p_write_file(path, text);
    if (second) *r2 = p_write_file(path, second);
    got[0] = 0;
    p_read_file(path, got, sizeof got);
    a_close(hold[1]);
    p_wait(pid);
    return r;
}

static void uid_case(const char *name, const char *text, i64 want) {
    expect(name, map_write("uid_map", text, 0, 0), want);
}
static void gid_case(const char *name, const char *text, i64 want) {
    expect(name, map_write("gid_map", text, 0, 0), want);
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    uid_case("inside_last_id", "4294967294 0 1\n", 15);
    uid_case("outside_last_id", "0 4294967294 1\n", 15);
    uid_case("inside_last_id_count_2", "4294967294 0 2\n", -EINVAL);
    uid_case("outside_last_id_count_2", "0 4294967294 2\n", -EINVAL);
    uid_case("both_from_5_largest", "5 5 4294967290\n", 15);
    check("largest_reads_back", str_eq(got, "         5          5 4294967290\n"), 0);
    uid_case("inside_from_5_one_more", "5 0 4294967291\n", -EINVAL);
    uid_case("outside_from_6_same_count", "5 6 4294967290\n", -EINVAL);
    uid_case("inside_from_6_same_count", "6 5 4294967290\n", -EINVAL);
    uid_case("inside_no_id_count_2", "4294967295 0 2\n", -EINVAL);
    uid_case("outside_no_id_large_inside", "70000 4294967295 1\n", -EINVAL);
    uid_case("count_too_big_for_32_bits", "0 0 4294967296\n", -EINVAL);
    uid_case("inside_above_32_bits_wraps", "4294967296 0 1\n", 15);
    check("wrapped_inside_reads_0", str_eq(got, "         0          0          1\n"), 0);
    uid_case("outside_above_32_bits_wraps", "0 4294967301 1\n", 15);
    check("wrapped_outside_reads_5", str_eq(got, "         0          5          1\n"), 0);
    uid_case("count_no_id_from_1", "1 1 4294967295\n", -EINVAL);
    uid_case("second_line_reaches_no_id", "0 0 10\n4294967000 100 296\n", -EINVAL);
    uid_case("second_line_stops_before_no_id", "0 0 10\n4294967000 100 295\n", 26);

    uid_case("leading_zero_is_decimal", "010 0 1\n", 8);
    check("leading_zero_reads_ten", str_eq(got, "        10          0          1\n"), 0);
    uid_case("hex_refused", "0x10 0 1\n", -EINVAL);
    uid_case("plus_sign_refused", "+1 0 1\n", -EINVAL);

    i64 r2 = -9999;
    expect("refused_write", map_write("uid_map", "4294967295 0 1\n", "0 0 1\n", &r2), -EINVAL);
    expect("correct_write_after_refused", r2, 6);
    check("map_is_the_second_write", str_eq(got, "         0          0          1\n"), 0);

    gid_case("gid_inside_last_id", "4294967294 9 1\n", 15);
    gid_case("gid_inside_no_id", "4294967295 9 1\n", -EINVAL);
    gid_case("gid_outside_no_id", "7 4294967295 1\n", -EINVAL);
    gid_case("gid_outside_overflow", "0 2 4294967294\n", -EINVAL);
    gid_case("gid_largest_from_2", "0 2 4294967293\n", 15);
    return finish();
}
