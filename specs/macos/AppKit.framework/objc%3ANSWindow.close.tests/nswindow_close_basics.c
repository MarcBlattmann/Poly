/* poly-mac: appkit */
/* close on a visible window: it becomes invisible, windowWillClose posts
 * NSWindowWillCloseNotification once (object the window), and with releasedWhenClosed NO the object
 * stays valid (title still readable) and is still listed in the application's `windows`
 * (a window leaves that list only when the object is deallocated); closing a window that is
 * already closed posts the notification no second time (the count stays 1). */
#include "poly_mak.h"

int test_main(void) {
    ak_init();
    NSWindow *w = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    w.title = @"keep";
    __block int closes = 0;
    id tok = [[NSNotificationCenter defaultCenter] addObserverForName:NSWindowWillCloseNotification object:w queue:nil usingBlock:^(NSNotification *x) {
        (void)x;
        closes++;
    }];
    [w orderFront:nil];
    ak_spin(0.05);
    CK(w.isVisible);
    [w close];
    CK(!w.isVisible);
    CKV(closes == 1, closes);
    CK([w.title isEqualToString:@"keep"]);
    [w close];
    CKV(closes == 1, closes);
    [[NSNotificationCenter defaultCenter] removeObserver:tok];
    return finish();
}
