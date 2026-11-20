/* poly-mac: libsystem
 * mach_port_insert_member(task, receive name, set name) puts a receive right into a port set; a second insert into the same
 * set is KERN_ALREADY_IN_SET (11); mach_port_extract_member takes it out, a second extract is KERN_NOT_IN_SET (12). The status
 * of the port shows the set in mps_pset; mach_port_get_set_status lists the members. A port can be in two sets. */
#include "poly_mapi.h"
static unsigned members(mach_port_t set, mach_port_t *first) {
    mach_port_name_array_t names = NULL;
    mach_msg_type_number_t n = 0;
    if (mach_port_get_set_status(mach_task_self(), set, &names, &n)) return 9999;
    if (n && first) *first = names[0];
    if (names) vm_deallocate(mach_task_self(), (vm_address_t)names, n * sizeof(mach_port_name_t));
    return n;
}
int test_main(void) {
    mach_port_t t = mach_task_self(), ps = 0, ps2 = 0, first = 0;
    mach_port_allocate(t, MACH_PORT_RIGHT_PORT_SET, &ps);
    mach_port_allocate(t, MACH_PORT_RIGHT_PORT_SET, &ps2);
    mach_port_t p = new_receive();
    check("an empty set has no members", members(ps, NULL) == 0, members(ps, NULL));
    kern_return_t kr = mach_port_insert_member(t, p, ps);
    check("insert succeeds", kr == KERN_SUCCESS, kr);
    check("the set has one member, the port", members(ps, &first) == 1 && first == p, first);
    mach_port_status_t s;
    mach_msg_type_number_t c = MACH_PORT_RECEIVE_STATUS_COUNT;
    mach_port_get_attributes(t, p, MACH_PORT_RECEIVE_STATUS, (mach_port_info_t)&s, &c);
    check("the port's status says it is in a set", s.mps_pset == 1, s.mps_pset);
    kr = mach_port_insert_member(t, p, ps);
    check("again: KERN_ALREADY_IN_SET (11)", kr == 11, kr);
    check("a second set takes it too", mach_port_insert_member(t, p, ps2) == KERN_SUCCESS, 0);
    check("each set lists it", members(ps, NULL) == 1 && members(ps2, NULL) == 1, members(ps2, NULL));
    check("extract from the second set", mach_port_extract_member(t, p, ps2) == KERN_SUCCESS && members(ps2, NULL) == 0, 0);
    check("extract from the first", mach_port_extract_member(t, p, ps) == KERN_SUCCESS && members(ps, NULL) == 0, 0);
    kr = mach_port_extract_member(t, p, ps);
    check("a second extract: KERN_NOT_IN_SET (12)", kr == 12, kr);
    check("the port is still a receive right", type_of(p) == MACH_PORT_TYPE_RECEIVE, type_of(p));
    return finish();
}
