/* poly-mac: appkit */
/* The fitting size of a button with the title "Hello" per bezel style (SYSTEM-DEPENDENT, printed as sys_*
 * "width x height"): Push/Rounded 55 x 24, RegularSquare 55 x 24, Disclosure 13 x 13 (title not shown), ShadowlessSquare
 * 41 x 22, Circular 39 x 24, TexturedSquare 43 x 25, HelpButton 24 x 24, SmallSquare 37 x 22, TexturedRounded 47 x 23,
 * RoundRect 55 x 24, Recessed 55 x 24, RoundedDisclosure 24 x 24, Inline 37 x 18. What is fixed is that
 * Disclosure and HelpButton ignore the title (the width does not depend on it), and that a longer title
 * gives a wider button with the push style. */
#include "poly_mak.h"

static NSSize fit(NSBezelStyle s, NSString *title) {
    NSButton *b = [[NSButton alloc] initWithFrame:NSMakeRect(0, 0, 10, 10)];
    b.title = title;
    b.bezelStyle = s;
    return b.fittingSize;
}

int test_main(void) {
    CK(fit(NSBezelStyleDisclosure, @"Hello").width == fit(NSBezelStyleDisclosure, @"A much longer title").width);
    CK(fit(NSBezelStyleHelpButton, @"Hello").width == fit(NSBezelStyleHelpButton, @"A much longer title").width);
    CK(fit(NSBezelStylePush, @"A much longer title").width > fit(NSBezelStylePush, @"Hello").width);
    const struct {
        const char *name;
        NSBezelStyle s;
    } st[] = {{"push", NSBezelStylePush}, {"regular_square", NSBezelStyleRegularSquare}, {"disclosure", NSBezelStyleDisclosure}, {"shadowless_square", NSBezelStyleShadowlessSquare}, {"circular", NSBezelStyleCircular}, {"textured_square", NSBezelStyleTexturedSquare}, {"help", NSBezelStyleHelpButton}, {"small_square", NSBezelStyleSmallSquare}, {"textured_rounded", NSBezelStyleTexturedRounded}, {"round_rect", NSBezelStyleRoundRect}, {"recessed", NSBezelStyleRecessed}, {"rounded_disclosure", NSBezelStyleRoundedDisclosure}, {"inline", NSBezelStyleInline}};
    for (unsigned i = 0; i < sizeof st / sizeof *st; i++) {
        NSSize z = fit(st[i].s, @"Hello");
        char n[80];
        int k = 0;
        for (const char *p = "sys_"; *p;) n[k++] = *p++;
        for (const char *p = st[i].name; *p;) n[k++] = *p++;
        n[k++] = '_';
        n[k++] = 'w';
        n[k] = 0;
        value_f(n, z.width);
        n[k - 1] = 'h';
        value_f(n, z.height);
    }
    return finish();
}
