/* poly-mac: appkit */
/* The application does not retain its delegate: after the last strong reference to it is gone
 * delegate reads nil, and a delegate set again works; a delegate set to nil gets no calls;
 * the delegate may be changed between runs. */
#include "poly_mak.h"

static int calls;

@interface Del : NSObject <NSApplicationDelegate>
@end
@implementation Del
- (void)applicationWillFinishLaunching:(NSNotification *)x { (void)x; calls++; }
@end

int test_main(void) {
    NSApplication *a = ak_init();
    @autoreleasepool {
        Del *d = [[Del alloc] init];
        a.delegate = d;
        CK(a.delegate == d);
    }
    CK(a.delegate == nil);
    Del *e = [[Del alloc] init];
    a.delegate = e;
    a.delegate = nil;
    CK(a.delegate == nil);
    [a finishLaunching];
    CK(calls == 0);
    a.delegate = e;
    [a finishLaunching];
    CK(calls == 1);
    return finish();
}
