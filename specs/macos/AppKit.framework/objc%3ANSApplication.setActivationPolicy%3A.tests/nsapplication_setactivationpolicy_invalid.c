/* poly-mac: appkit */
/* A value that is not a policy (99, -1) answers NO and leaves the current policy as it was;
 * the policy of an application that never set one is Prohibited, and setting a valid one after
 * a refused one works. */
#include "poly_mak.h"

int test_main(void) {
    NSApplication *a = ak_init();
    CK(![a setActivationPolicy:(NSApplicationActivationPolicy)99]);
    CK(a.activationPolicy == NSApplicationActivationPolicyProhibited);
    CK(![a setActivationPolicy:(NSApplicationActivationPolicy)-1]);
    CK(a.activationPolicy == NSApplicationActivationPolicyProhibited);
    CK([a setActivationPolicy:NSApplicationActivationPolicyAccessory]);
    CK(![a setActivationPolicy:(NSApplicationActivationPolicy)3]);
    CK(a.activationPolicy == NSApplicationActivationPolicyAccessory);
    CK([a setActivationPolicy:NSApplicationActivationPolicyProhibited]);
    return finish();
}
