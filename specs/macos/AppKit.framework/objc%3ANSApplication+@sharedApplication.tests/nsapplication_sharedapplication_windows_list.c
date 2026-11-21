/* poly-mac: appkit */
/* The application's windows array lists every window made, shown or not, in creation order,
 * with the window an NSWindow; a closed window stays listed as long as it is alive
 * (releasedWhenClosed off); the array is a copy (adding a window later does not
 * change one read before). */
#include "poly_mak.h"

int test_main(void) {
    NSApplication *a = ak_init();
    NSWindow *w1 = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    CK(a.windows.count == 1 && a.windows[0] == w1);
    NSWindow *w2 = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 50, 50));
    NSArray *before = a.windows;
    CK(before.count == 2 && before[0] == w1 && before[1] == w2);
    NSWindow *w3 = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 10, 10));
    CK(a.windows.count == 3 && before.count == 2);
    [w2 close];
    CK(a.windows.count == 3);
    CK([a.windows containsObject:w3]);
    return finish();
}
