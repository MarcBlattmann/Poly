/* csops, BSD 169, CS_OPS_BLOB (10): the process's signature blob: it starts
 * with the magic of an embedded signature, the bytes fa de 0c c0 (CSMAGIC_EMBEDDED_SIGNATURE,
 * big endian), followed by its length (big endian u32, a value). Operations
 * 4 (path) and 5 (hash of the code directory) are refused with EINVAL
 * for an ordinary caller on this system (values). Operation numbers were
 * found by experiment; the blob magic is public. */
#include "poly_mtest.h"

/* Operations and flags: no public header; found by experiment. */
#define CS_OPS_STATUS 0
#define CS_VALID 0x00000001u

static u8 blob[4096];

int test_main(void) {
    memset(blob, 0, sizeof blob);
    sysret r = bsd4(SYS_csops, 0, 10, blob, sizeof blob);
    check("CS_OPS_BLOB returns 0", r.err == 0 && r.ret == 0, r.err);
    check("it starts with the embedded-signature magic fa de 0c c0", blob[0] == 0xfa && blob[1] == 0xde && blob[2] == 0x0c && blob[3] == 0xc0, blob[0]);
    u32 len = (u32)blob[4] << 24 | (u32)blob[5] << 16 | (u32)blob[6] << 8 | blob[7];
    check("its length (bytes 4..7, big endian) fits the buffer", len >= 8 && len <= sizeof blob, len);
    value("signature blob length", len);
    u8 buf[1024];
    value("operation 4 (pid path): errno", bsd4(SYS_csops, 0, 4, buf, sizeof buf).err);
    value("operation 5 (cdhash): errno", bsd4(SYS_csops, 0, 5, buf, sizeof buf).err);
    return finish();
}
