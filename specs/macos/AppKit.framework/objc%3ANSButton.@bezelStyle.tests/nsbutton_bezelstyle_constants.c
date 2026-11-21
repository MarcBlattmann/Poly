/* poly-mac: appkit */
/* The bezel style constants and their numbers (public SDK header NSButtonCell.h): Automatic 0,
 * Push 1 (also Rounded 1), FlexiblePush 2 (also RegularSquare 2), Disclosure 5, ShadowlessSquare 6, Circular 7,
 * TexturedSquare 8, HelpButton 9, SmallSquare 10, TexturedRounded 11, RoundRect 12, Recessed 13,
 * RoundedDisclosure 14, Inline 15; bezelStyle reads back what was set for every one of these; a new
 * button has the automatic style. */
#include "poly_mak.h"

int test_main(void) {
    CK(NSBezelStyleAutomatic == 0 && NSBezelStylePush == 1 && NSBezelStyleRounded == 1);
    CK(NSBezelStyleFlexiblePush == 2 && NSBezelStyleRegularSquare == 2);
    CK(NSBezelStyleDisclosure == 5 && NSBezelStyleShadowlessSquare == 6 && NSBezelStyleCircular == 7);
    CK(NSBezelStyleTexturedSquare == 8 && NSBezelStyleHelpButton == 9 && NSBezelStyleSmallSquare == 10);
    CK(NSBezelStyleTexturedRounded == 11 && NSBezelStyleRoundRect == 12 && NSBezelStyleRecessed == 13);
    CK(NSBezelStyleRoundedDisclosure == 14 && NSBezelStyleInline == 15);
    const NSBezelStyle all[] = {NSBezelStyleAutomatic, NSBezelStylePush, NSBezelStyleFlexiblePush, NSBezelStyleDisclosure, NSBezelStyleShadowlessSquare, NSBezelStyleCircular, NSBezelStyleTexturedSquare, NSBezelStyleHelpButton, NSBezelStyleSmallSquare, NSBezelStyleTexturedRounded, NSBezelStyleRoundRect, NSBezelStyleRecessed, NSBezelStyleRoundedDisclosure, NSBezelStyleInline};
    NSButton *b = [[NSButton alloc] initWithFrame:NSMakeRect(0, 0, 80, 24)];
    int ok = 1;
    for (unsigned i = 0; i < sizeof all / sizeof *all; i++) {
        b.bezelStyle = all[i];
        ok &= b.bezelStyle == all[i];
    }
    CK(ok);
    return finish();
}
