/* poly-mac: appkit */
/* With the Prohibited policy (the default of this program) windows can still be made and ordered
 * front: they are visible, never key and never main, the application never becomes active,
 * canBecomeKeyWindow is still YES for a titled window and NO for a borderless one. */
#include "poly_mak.h"

int test_main(void) {
    NSApplication *a = ak_init();
    CK(a.activationPolicy == NSApplicationActivationPolicyProhibited);
    NSWindow *t = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    NSWindow *b = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, 50, 50));
    CK(t.canBecomeKeyWindow && !b.canBecomeKeyWindow);
    [t makeKeyAndOrderFront:nil];
    [b orderFront:nil];
    ak_spin(0.2);
    CK(t.isVisible && b.isVisible);
    CK(!t.isKeyWindow && !t.isMainWindow && !b.isKeyWindow);
    CK(!a.isActive && a.keyWindow == nil && a.mainWindow == nil);
    [t close];
    [b close];
    return finish();
}
