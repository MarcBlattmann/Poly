/* poly-mac: appkit */
/* The policy does not by itself activate the application or give it windows: after each policy
 * the application is not active (the program never activates itself here), has no key
 * window and its windows list is unchanged; a window made under the Accessory policy
 * can be ordered front (invisibly at alpha 0) without the application becoming active,
 * and ordering it out brings the state back. */
#include "poly_mak.h"

int test_main(void) {
    NSApplication *a = ak_init();
    NSWindow *w = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    const NSApplicationActivationPolicy ps[] = {NSApplicationActivationPolicyAccessory, NSApplicationActivationPolicyRegular, NSApplicationActivationPolicyProhibited};
    for (int i = 0; i < 3; i++) {
        CK([a setActivationPolicy:ps[i]]);
        CK(!a.isActive && a.keyWindow == nil && a.windows.count == 1);
    }
    CK([a setActivationPolicy:NSApplicationActivationPolicyAccessory]);
    [w orderFront:nil];
    ak_spin(0.2);
    CK(w.isVisible);
    CK(!a.isActive);
    [w orderOut:nil];
    CK(!w.isVisible);
    CK([a setActivationPolicy:NSApplicationActivationPolicyProhibited]);
    return finish();
}
