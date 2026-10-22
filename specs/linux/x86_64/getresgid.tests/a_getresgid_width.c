/* getresgid: each of the three ids is written as a 4-byte unsigned value
 * at exactly the address given: bytes before and after are untouched, and
 * unaligned addresses work. */
#include "a_sys.h"

#define SYS_getresgid 120 /* asm/unistd_64.h */

static unsigned char buf[64];

static u32 load32(const unsigned char *p) {
    return (u32)p[0] | (u32)p[1] << 8 | (u32)p[2] << 16 | (u32)p[3] << 24;
}

int test_main(void) {
    u32 r, e, s;
    syscall3(SYS_getresgid, &r, &e, &s);
    for (int i = 0; i < 64; i++) buf[i] = 0xa5;
    /* Slots at offsets 8, 20 and 32: aligned to 4 bytes. */
    expect("aligned_ok", syscall3(SYS_getresgid, buf + 8, buf + 20, buf + 32), 0);
    expect("aligned_real", load32(buf + 8), r);
    expect("aligned_effective", load32(buf + 20), e);
    expect("aligned_saved", load32(buf + 32), s);
    int guards = 1;
    for (int i = 0; i < 64; i++)
        if (!((i >= 8 && i < 12) || (i >= 20 && i < 24) || (i >= 32 && i < 36))) guards &= buf[i] == 0xa5;
    check("aligned_neighbours_untouched", guards, 0);
    for (int i = 0; i < 64; i++) buf[i] = 0x5a;
    /* Odd offsets 1, 13 and 27. */
    expect("unaligned_ok", syscall3(SYS_getresgid, buf + 1, buf + 13, buf + 27), 0);
    expect("unaligned_real", load32(buf + 1), r);
    expect("unaligned_effective", load32(buf + 13), e);
    expect("unaligned_saved", load32(buf + 27), s);
    guards = 1;
    for (int i = 0; i < 64; i++)
        if (!((i >= 1 && i < 5) || (i >= 13 && i < 17) || (i >= 27 && i < 31))) guards &= buf[i] == 0x5a;
    check("unaligned_neighbours_untouched", guards, 0);
    /* A u64 slot: only its low 4 bytes change. */
    u64 wide = 0xffffffffffffffffUL;
    syscall3(SYS_getresgid, &wide, &e, &s);
    expect("high_half_untouched", (i64)(wide >> 32), 0xffffffffL);
    expect("low_half_written", (i64)(wide & 0xffffffffUL), r);
    return finish();
}
