/* mmap address hints and big PROT_NONE reservations, the way a JavaScript
 * engine uses them: ask for address space at a low hint, commit pieces of it
 * with mprotect later, give it back in parts. Per mmap(2): without MAP_FIXED
 * the address is only a hint, taken when the range is free; with
 * MAP_FIXED_NOREPLACE an overlap is EEXIST.
 *
 * The program is a static non-PIE image (linked at 0x400000), so the low
 * ranges used here stay below it. Addresses are never printed. */
#include "poly_test.h"

#define PAGE 4096UL
#define MAP_NORESERVE 0x4000
#define MAP_FIXED_NOREPLACE 0x100000
#define EEXIST 17
#define SYS_madvise 28
#define MADV_DONTDUMP 16

#define RESERVE (MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE)

static i64 map(i64 addr, i64 len, i64 prot, i64 flags) { return syscall6(SYS_mmap, addr, len, prot, flags, -1, 0); }

int test_main(void) {
    /* The hint is used when the range is free (3 MiB, ends below 0x400000). */
    i64 a = map(0x1a000, 0x2e6000, PROT_NONE, RESERVE);
    check("hint_below_the_program_is_honoured", a == 0x1a000, a);
    check("madvise_dontdump_on_a_reservation", syscall3(SYS_madvise, a, 0x2e6000, MADV_DONTDUMP) == 0, 0);

    /* Commit a piece in the middle and use it; the rest stays reserved. */
    i64 r = syscall3(SYS_mprotect, 0x100000, 0x40000, PROT_READ | PROT_WRITE);
    check("mprotect_commits_a_piece_of_a_reservation", r == 0, r);
    volatile unsigned char *p = (volatile unsigned char *)0x100000;
    check("committed_piece_is_zero", p[0] == 0 && p[0x3ffff] == 0, 0);
    p[0] = 0x11;
    p[0x3ffff] = 0x22;
    check("committed_piece_is_writable", p[0] == 0x11 && p[0x3ffff] == 0x22, 0);
    r = syscall3(SYS_mprotect, 0x140000, 0x40000, PROT_READ | PROT_WRITE);
    check("mprotect_commits_the_next_piece", r == 0, r);
    check("contents_survive_the_neighbour", p[0] == 0x11 && p[0x3ffff] == 0x22, 0);

    /* The range is taken now: the same hint gives another address. */
    i64 b = map(0x1a000, 0x2e6000, PROT_NONE, RESERVE);
    check("taken_hint_is_not_honoured", b > 0 && b != 0x1a000, b > 0 ? 0 : b);
    check("taken_hint_result_is_page_aligned", b > 0 && (b & (PAGE - 1)) == 0, 0);
    check("munmap_the_other_reservation", syscall2(SYS_munmap, b, 0x2e6000) == 0, 0);

    /* MAP_FIXED_NOREPLACE: an overlap is EEXIST, touching is fine. */
    r = map(0x1a000, PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE);
    check("fixed_noreplace_over_the_start_is_eexist", r == -EEXIST, r);
    r = map(0x100000, PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE);
    check("fixed_noreplace_inside_is_eexist", r == -EEXIST, r);
    r = map(0x2ff000, 2 * PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE);
    check("fixed_noreplace_over_the_end_is_eexist", r == -EEXIST, r);
    r = map(0x19000, 2 * PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE);
    check("fixed_noreplace_over_the_start_from_below_is_eexist", r == -EEXIST, r);
    r = map(0x2ff000, PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE);
    check("fixed_noreplace_in_the_last_page_is_eexist", r == -EEXIST, r);
    r = map(0x300000, PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE);
    check("fixed_noreplace_right_after_the_reservation_works", r == 0x300000, r);
    check("munmap_that_page", syscall2(SYS_munmap, 0x300000, PAGE) == 0, 0);

    /* Give back the middle; the pieces either side stay usable. */
    check("munmap_the_middle", syscall2(SYS_munmap, 0x100000, 0x80000) == 0, 0);
    r = syscall3(SYS_mprotect, 0x100000, PAGE, PROT_READ);
    check("mprotect_in_the_hole_is_enomem", r == -ENOMEM, r);
    r = syscall3(SYS_mprotect, 0x1a000, 0x1000, PROT_READ | PROT_WRITE);
    check("mprotect_before_the_hole_works", r == 0, r);
    check("munmap_the_rest", syscall2(SYS_munmap, 0x1a000, 0x2e6000) == 0, 0);

    /* A 4 GiB reservation anywhere: no memory is used until it is committed,
     * and it is committed in pieces. */
    i64 len = 4UL << 30;
    i64 big = map(0, len, PROT_NONE, RESERVE);
    check("four_gib_reservation_succeeds", big > 0, big < 0 ? big : 0);
    if (big > 0) {
        i64 mid = big + (len / 2);
        r = syscall3(SYS_mprotect, mid, 2UL << 20, PROT_READ | PROT_WRITE);
        check("four_gib_commit_a_middle_piece", r == 0, r);
        volatile unsigned char *q = (volatile unsigned char *)mid;
        q[0] = 7;
        q[(2UL << 20) - 1] = 9;
        check("four_gib_piece_is_writable", q[0] == 7 && q[(2UL << 20) - 1] == 9, 0);
        check("four_gib_unmap_the_first_half", syscall2(SYS_munmap, big, len / 2) == 0, 0);
        check("four_gib_piece_survives", q[0] == 7 && q[(2UL << 20) - 1] == 9, 0);
        check("four_gib_unmap_the_second_half", syscall2(SYS_munmap, mid, len / 2) == 0, 0);
    }
    return finish();
}
