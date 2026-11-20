/* poly-mac: corefoundation */
/* Synchronising a domain that holds nothing answers true whatever the
 * name: a long one, one with dots only, with non-ASCII characters, and the
 * global domain; none of this creates keys. */
#include "poly_mcf.h"

int test_main(void) {
    CK(CFPreferencesAppSynchronize(CFSTR("test.poly.cfspec.sync-long-0123456789-0123456789-0123456789-0123456789-0123456789")));
    CK(CFPreferencesAppSynchronize(CFSTR("test.poly.cfspec.sync.dots.only")));
    CK(CFPreferencesAppSynchronize(CFSTR("test.poly.cfspec.sync-café")));
    CK(CFPreferencesAppSynchronize(kCFPreferencesAnyApplication));
    CK(CFPreferencesCopyKeyList(CFSTR("test.poly.cfspec.sync.dots.only"), kCFPreferencesCurrentUser, kCFPreferencesAnyHost) == NULL);
    CK(CFPreferencesCopyKeyList(CFSTR("test.poly.cfspec.sync-café"), kCFPreferencesCurrentUser, kCFPreferencesAnyHost) == NULL);
    return finish();
}
