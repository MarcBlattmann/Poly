# A stand-in for libobjc in the poly-objc tests: NSObject's class and
# metaclass objects in the layout the compiler uses (abi.rs), the empty
# cache, and a class list. Written for Poly; it has no methods.
    .section __TEXT,__objc_classname,cstring_literals
L_name: .asciz "NSObject"
    .section __DATA,__objc_const
    .p2align 3
L_meta_ro:
    .long 3, 40, 40, 0
    .quad 0, L_name, 0, 0, 0, 0, 0
L_class_ro:
    .long 2, 8, 8, 0
    .quad 0, L_name, 0, 0, 0, 0, 0
    .section __DATA,__objc_data
    .p2align 3
    .globl _OBJC_METACLASS_$_NSObject
_OBJC_METACLASS_$_NSObject:
    .quad _OBJC_METACLASS_$_NSObject, _OBJC_CLASS_$_NSObject, __objc_empty_cache, 0, L_meta_ro
    .globl _OBJC_CLASS_$_NSObject
_OBJC_CLASS_$_NSObject:
    .quad _OBJC_METACLASS_$_NSObject, 0, __objc_empty_cache, 0, L_class_ro
    .globl __objc_empty_cache
__objc_empty_cache:
    .quad 0, 0
    .section __DATA,__objc_classlist,regular,no_dead_strip
    .p2align 3
    .quad _OBJC_CLASS_$_NSObject
