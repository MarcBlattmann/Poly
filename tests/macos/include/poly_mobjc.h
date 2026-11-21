/*
 * Support for macOS test programs about the Objective-C runtime
 * (`libobjc.A.dylib`; ADR 0182, the libobjc batch). A test that includes this
 * header says so in its header comment with the line `poly-mac: libobjc`
 * (and, for a test of what the compiler's reference counting emits,
 * `poly-mac: arc`): `mac build` then compiles the `.c` file as Objective-C,
 * links libobjc, and lets it import libobjc's and libSystem's symbols. All
 * other tests import nothing (see poly_mtest.h). The test defines
 * `int test_main(void)` and reports through poly_mtest.h's `check`, `value`
 * and `finish`, which write with raw system calls, never through the
 * runtime under test.
 *
 * Only the root class `NSObject` (which libobjc itself exports, with
 * `<objc/NSObject.h>`) is used: no Foundation. The ARC entry points
 * (`objc_retain`, `objc_storeStrong`, ...) are not in the public headers,
 * so their prototypes are written out here from their well-known
 * names and signatures; nothing else is declared that the SDK does not.
 *
 * Written for Poly from public headers and from observation only. No Apple
 * or third-party source was read; no code is copied from anywhere.
 */
#ifndef POLY_MOBJC_H
#define POLY_MOBJC_H

#include "poly_mtest.h"
#include <objc/runtime.h>
#include <objc/message.h>
#include <objc/NSObject.h>
#include <objc/objc-sync.h>
#include <objc/objc-exception.h>

#pragma clang diagnostic ignored "-Wdeprecated-declarations"
#pragma clang diagnostic ignored "-Wunused-function"
#pragma clang diagnostic ignored "-Wundeclared-selector"
#pragma clang diagnostic ignored "-Wobjc-method-access"
#pragma clang diagnostic ignored "-Wobjc-protocol-method-implementation"

#if defined(__x86_64__)
#define POLY_ARCH "x86_64"
#else
#define POLY_ARCH "arm64"
#endif

/* The entry points of reference counting, as the compiler emits calls to them. */
extern id objc_retain(id);
extern void objc_release(id);
extern id objc_autorelease(id);
extern id objc_retainAutorelease(id);
extern id objc_retainAutoreleaseReturnValue(id);
extern id objc_retainAutoreleasedReturnValue(id);
extern id objc_autoreleaseReturnValue(id);
extern id objc_unsafeClaimAutoreleasedReturnValue(id);
extern id objc_retainBlock(id);
extern void objc_storeStrong(id *, id);
extern id objc_storeWeak(id *, id);
extern id objc_initWeak(id *, id);
extern id objc_loadWeakRetained(id *);
extern void objc_destroyWeak(id *);
extern void objc_moveWeak(id *, id *);
extern void objc_copyWeak(id *, id *);
extern void *objc_autoreleasePoolPush(void);
extern void objc_autoreleasePoolPop(void *);
extern id objc_alloc(Class);
extern id objc_alloc_init(Class);
extern id objc_allocWithZone(Class);
extern void *objc_destructInstance(id);
extern id objc_constructInstance(Class, void *);

/* Message sends, cast to the signature of the method called. */
#define SEL_(name) sel_registerName(name)
/* objc_msgSend must be called through a pointer of the exact type of the
 * method (a variadic type puts the arguments in the wrong place on arm64):
 * `((long (*)(id, SEL, long))objc_msgSend)(obj, sel, 5)`. */
static inline id send0(id o, const char *sel) { return ((id(*)(id, SEL))objc_msgSend)(o, sel_registerName(sel)); }
static inline id send1(id o, const char *sel, id a) {
    return ((id(*)(id, SEL, id))objc_msgSend)(o, sel_registerName(sel), a);
}
static inline long send0l(id o, const char *sel) { return ((long(*)(id, SEL))objc_msgSend)(o, sel_registerName(sel)); }
static inline unsigned long retain_count(id o) { return ((unsigned long (*)(id, SEL))objc_msgSend)(o, sel_registerName("retainCount")); }

/* The name of an object's class, "(nil)" for nil. */
static inline const char *cname(id o) { return o ? class_getName(object_getClass(o)) : "(nil)"; }
static inline const char *cname_or_nil(Class c) { return c ? class_getName(c) : "(nil)"; }
static inline const char *or_nil(const char *s) { return s ? s : "(null)"; }

/* A plain instance of NSObject. */
static inline id new_object(void) { return send0((id)objc_getClass("NSObject"), "new"); }

#endif
