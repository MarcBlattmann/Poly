/* poly-mac: appkit */
/* Where a view's effectiveAppearance comes from: a view takes its window's, and a window takes the
 * application's (NSApp.appearance) or, when none is set, the system's setting (light or dark: not fixed by this
 * test, so it is not printed); setting window.appearance makes the window's, its content view's and every subview's
 * effectiveAppearance that one; a view's own `appearance` overrides the inherited one for that view and its
 * subviews and does not change its parent's; a view that is in no window has the system's appearance; NSApp.appearance set to
 * dark makes a new window (with no appearance of its own) dark and restoring nil gives back the system's; `appearance`
 * reads nil until set. */
#include "poly_mak.h"

int test_main(void) {
    NSApplication *a = ak_init();
    CK(a.appearance == nil);
    NSWindow *w = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    CK(w.appearance == nil && w.contentView.appearance == nil);
    NSAppearance *system = w.effectiveAppearance;
    CK(system != nil && w.contentView.effectiveAppearance == system);
    NSView *loose = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 10, 10)];
    CK(loose.effectiveAppearance == system);
    w.appearance = ak_dark();
    CK(w.effectiveAppearance == ak_dark() && w.contentView.effectiveAppearance == ak_dark());
    NSView *sub = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 10, 10)];
    [w.contentView addSubview:sub];
    CK(sub.effectiveAppearance == ak_dark());
    sub.appearance = ak_light();
    CK(sub.effectiveAppearance == ak_light() && w.contentView.effectiveAppearance == ak_dark());
    NSView *deep = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 5, 5)];
    [sub addSubview:deep];
    CK(deep.effectiveAppearance == ak_light());
    w.appearance = ak_light();
    CK(w.effectiveAppearance == ak_light());
    a.appearance = ak_dark();
    NSWindow *w2 = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    CK(w2.effectiveAppearance == ak_dark() && a.effectiveAppearance == ak_dark());
    a.appearance = nil;
    CK(a.appearance == nil);
    CK(w2.effectiveAppearance == system);
    return finish();
}
