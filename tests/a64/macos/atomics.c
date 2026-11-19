#include "rt.h"
static long counter;
static unsigned spin;
static long arr[64];
int run(void) {
    for (int i = 0; i < 1000; i++) __atomic_fetch_add(&counter, i, __ATOMIC_SEQ_CST);
    long exp = counter, ok = 0;
    for (int i = 0; i < 100; i++) { long e = exp; ok += __atomic_compare_exchange_n(&counter, &e, exp + 1, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE); exp++; }
    for (int i = 0; i < 64; i++) { __atomic_store_n(&arr[i], i * 3, __ATOMIC_RELEASE); __atomic_fetch_or(&arr[i], 0x100, __ATOMIC_RELAXED); __atomic_fetch_and(&arr[i], ~1L, __ATOMIC_ACQUIRE); __atomic_fetch_xor(&arr[i], 0x55, __ATOMIC_SEQ_CST); }
    long s = 0; for (int i = 0; i < 64; i++) s += __atomic_load_n(&arr[i], __ATOMIC_ACQUIRE) + __atomic_exchange_n(&arr[i], 0, __ATOMIC_ACQ_REL);
    while (__atomic_test_and_set(&spin, __ATOMIC_ACQUIRE)) {}
    __atomic_clear(&spin, __ATOMIC_RELEASE);
    unsigned char b = 7; __atomic_fetch_sub(&b, 3, __ATOMIC_SEQ_CST);
    puts_("counter="); puti(counter); puts_(" cas ok="); puti(ok); puts_(" arr="); puti(s); puts_(" byte="); puti(b); nl();
    return 0;
}
