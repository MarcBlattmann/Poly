/* dispatch_source_create with arguments that make no source: NULL. And
 * the source's accessors for a data source. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <signal.h>

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.err", DISPATCH_QUEUE_SERIAL);
    check("a signal number of 0: NULL", dispatch_source_create(DISPATCH_SOURCE_TYPE_SIGNAL, 0, 0, q) == NULL, 0);
    check("a signal number above 31: NULL", dispatch_source_create(DISPATCH_SOURCE_TYPE_SIGNAL, 100, 0, q) == NULL, 0);
    check("a nonzero mask for a signal source: NULL", dispatch_source_create(DISPATCH_SOURCE_TYPE_SIGNAL, SIGUSR1, 1, q) == NULL, 0);
    value("a timer source with a nonzero handle is made (1) or refused (0)", dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 1, 0, q) != NULL);
    check("a nonzero handle for a data source: NULL", dispatch_source_create(DISPATCH_SOURCE_TYPE_DATA_ADD, 1, 0, q) == NULL, 0);
    check("a timer with mask 0 is valid", dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, q) != NULL, 0);
    dispatch_source_t d = dispatch_source_create(DISPATCH_SOURCE_TYPE_DATA_ADD, 0, 0, q);
    check("a data source's handle is 0", dispatch_source_get_handle(d) == 0, 0);
    check("its mask is 0", dispatch_source_get_mask(d) == 0, 0);
    check("its data is 0 before anything was merged", dispatch_source_get_data(d) == 0, 0);
    check("it is not cancelled", dispatch_source_testcancel(d) == 0, 0);
    dispatch_source_t o = dispatch_source_create(DISPATCH_SOURCE_TYPE_DATA_OR, 0, 0xff, q);
    check("a DATA_OR source with a nonzero mask (0xff) is refused: NULL", o == NULL, 0);
    dispatch_source_t o2 = dispatch_source_create(DISPATCH_SOURCE_TYPE_DATA_OR, 0, 0, q);
    check("with mask 0 it is made", o2 != NULL, 0);
    return finish();
}
