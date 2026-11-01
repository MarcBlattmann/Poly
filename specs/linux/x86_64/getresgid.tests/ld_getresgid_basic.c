/* getresgid(rgid, egid, sgid) writes the caller's real, effective and saved
 * group ids as 32-bit values and returns 0. The real id equals getgid, the
 * effective id getegid; for a program started normally (no set-group-id
 * file) the saved id equals the effective id. Only 4 bytes are written at
 * each pointer. The filesystem gid (not reported) equals the effective id
 * here. The call needs no privilege and changes nothing.
 */
#include "a_sys.h"

#define SYS_getresgid 120
#define SYS_setfsgid 123

int test_main(void) {
    u32 buf[9];
    memset(buf, 0xa5, sizeof buf);
    i64 r = syscall3(SYS_getresgid, &buf[1], &buf[4], &buf[7]);
    check("returns_0", r == 0, r);
    check("rgid_is_getgid", buf[1] == (u32)syscall0(SYS_getgid), buf[1]);
    check("egid_is_getegid", buf[4] == (u32)syscall0(SYS_getegid), buf[4]);
    check("sgid_is_egid", buf[7] == buf[4], buf[7]);
    check("only_4_bytes_each",
          buf[0] == 0xa5a5a5a5 && buf[2] == 0xa5a5a5a5 && buf[3] == 0xa5a5a5a5 && buf[5] == 0xa5a5a5a5 &&
              buf[6] == 0xa5a5a5a5 && buf[8] == 0xa5a5a5a5,
          0);
    check("fsgid_is_egid", syscall1(SYS_setfsgid, -1) == buf[4], syscall1(SYS_setfsgid, -1));

    /* The same pointer three times: the last write (saved gid) wins. */
    u32 one = 0xdeadbeef;
    r = syscall3(SYS_getresgid, &one, &one, &one);
    check("same_pointer_ok", r == 0 && one == buf[7], r);

    /* Unaligned pointers work. */
    u8 bytes[16];
    memset(bytes, 0, sizeof bytes);
    r = syscall3(SYS_getresgid, bytes + 1, bytes + 5, bytes + 9);
    u32 rg, eg, sg;
    memcpy(&rg, bytes + 1, 4);
    memcpy(&eg, bytes + 5, 4);
    memcpy(&sg, bytes + 9, 4);
    check("unaligned_ok", r == 0, r);
    check("unaligned_values", rg == buf[1] && eg == buf[4] && sg == buf[7], 0);
    check("unaligned_edges_untouched", bytes[0] == 0 && bytes[13] == 0, 0);

    /* Nothing changed. */
    u32 again[3];
    syscall3(SYS_getresgid, &again[0], &again[1], &again[2]);
    check("repeatable", again[0] == buf[1] && again[1] == buf[4] && again[2] == buf[7], 0);
    return finish();
}
