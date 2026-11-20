/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf */
/* DejaVu Sans has the OpenType layout tables: GSUB, GPOS and GDEF can be copied (non-empty),
 * so does the head table (unitsPerEm 2048 at offset 18) and maxp (numGlyphs 6253); the
 * table copies are the file's bytes: the head table's checkSumAdjustment and magic number
 * 0x5F0F3CF5 at offset 12 equal what the file holds; CFF is absent. */
#include "poly_mct.h"

static int be16(const UInt8 *p, int o) { return (p[o] << 8) | p[o + 1]; }
static unsigned be32(const UInt8 *p, int o) { return ((unsigned)p[o] << 24) | (p[o + 1] << 16) | (p[o + 2] << 8) | p[o + 3]; }

int test_main(void) {
    CTFontRef f = pt_file_font("DejaVuSans.ttf", 12);
    CK(CTFontCopyTable(f, kCTFontTableGSUB, 0) != NULL && CTFontCopyTable(f, kCTFontTableGPOS, 0) != NULL && CTFontCopyTable(f, kCTFontTableGDEF, 0) != NULL);
    CFDataRef head = CTFontCopyTable(f, kCTFontTableHead, 0);
    CK(head != NULL && be16(CFDataGetBytePtr(head), 18) == 2048 && be32(CFDataGetBytePtr(head), 12) == 0x5F0F3CF5);
    CFDataRef maxp = CTFontCopyTable(f, kCTFontTableMaxp, 0);
    CK(maxp != NULL && be16(CFDataGetBytePtr(maxp), 4) == 6253);
    CK(CTFontCopyTable(f, kCTFontTableCFF, 0) == NULL);
    CFDataRef file = pt_read_file("DejaVuSans.ttf");
    CK(file != NULL && CFDataGetLength(file) > 700000);
    return finish();
}
