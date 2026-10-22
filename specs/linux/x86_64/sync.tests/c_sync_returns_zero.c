/* sync() takes no arguments and always returns 0, also when called many
 * times in a row. */
#include "a_sys.h"

#define SYS_sync 162

int test_main(void) {
    expect("first", syscall0(SYS_sync), 0);
    expect("second", syscall0(SYS_sync), 0);
    int bad = 0;
    for (int i = 0; i < 50; i++)
        if (syscall0(SYS_sync) != 0) bad++;
    expect("fifty_more", bad, 0);
    return finish();
}
