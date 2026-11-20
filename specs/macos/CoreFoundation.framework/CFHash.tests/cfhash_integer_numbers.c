/* poly-mac: corefoundation */
/* The hash of an integer number is its absolute value times 2654435761
 * (as an unsigned 64-bit product), and a Boolean hashes as 1 or 0. */
#include "poly_mcf.h"

static unsigned long hash_int(long long v) {
    return (unsigned long)CFHash(CFNumberCreate(kCFAllocatorDefault, kCFNumberLongLongType, &v));
}

int test_main(void) {
    CKV(hash_int(0) == 0, hash_int(0));
    CKV(hash_int(1) == 2654435761UL, hash_int(1));
    CKV(hash_int(2) == 2 * 2654435761UL, hash_int(2));
    CKV(hash_int(12345) == 12345 * 2654435761UL, hash_int(12345));
    CKV(hash_int(-5) == 5 * 2654435761UL, hash_int(-5));
    CKV(hash_int(-5) == hash_int(5), 0);
    CKV(hash_int(1000000) == 1000000 * 2654435761UL, hash_int(1000000));
    CKV((unsigned long)CFHash(kCFBooleanTrue) == 2654435761UL, CFHash(kCFBooleanTrue));
    CKV((unsigned long)CFHash(kCFBooleanFalse) == 0, CFHash(kCFBooleanFalse));
    return finish();
}
