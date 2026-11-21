//! libdyld: what a program asks Poly's dyld about itself. dyld calls this
//! library's initializer with its start block as the fifth argument
//! (ADR 0258), whose image table (`poly_dyld::start`) answers
//! `_dyld_image_count` and `_dyld_get_image_*`; `apple[0]`
//! (`executable_path=`) answers `_NSGetExecutablePath`. The variables
//! `NXArgc`, `NXArgv`, `environ` and `__progname` are set from the
//! initializer's arguments.

use core::ffi::{c_char, c_void};

#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
static mut NXArgc: i32 = 0;
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
static mut NXArgv: *const *const c_char = core::ptr::null();
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
static mut environ: *const *const c_char = core::ptr::null();
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
static mut __progname: *const c_char = core::ptr::null();

/// The image table: (header, slide, path) words.
static mut IMAGES: *const [u64; 3] = core::ptr::null();
static mut IMAGE_COUNT: u32 = 0;
static mut BLOCK: *const u8 = core::ptr::null();
static mut EXECUTABLE_PATH: *const u8 = core::ptr::null();

/// "PolyImgs" (`poly_dyld::start::IMAGES_MAGIC`).
const IMAGES_MAGIC: u64 = u64::from_le_bytes(*b"PolyImgs");
const EXECUTABLE_PATH_KEY: &[u8] = b"executable_path=";

extern "C" fn init(
    argc: i32,
    argv: *const *const c_char,
    envp: *const *const c_char,
    apple: *const *const u8,
    block: *const u8,
) {
    // SAFETY: dyld's contract (ADR 0237, 0258): argv, envp and apple are
    // null-terminated arrays of strings and `block` is the start block;
    // initializers run on the main thread before anything else.
    unsafe {
        NXArgc = argc;
        NXArgv = argv;
        environ = envp;
        if argc > 0 && !(*argv).is_null() {
            let a0 = *argv as *const u8;
            let slash = poly_libsystem::cstr::strrchr(a0, i32::from(b'/'));
            __progname = (if slash.is_null() { a0 } else { slash.add(1) }) as *const c_char;
        }
        if !apple.is_null() && !(*apple).is_null() {
            let a = *apple;
            let n = EXECUTABLE_PATH_KEY.len();
            if poly_libsystem::cstr::strncmp(a, EXECUTABLE_PATH_KEY.as_ptr(), n) == 0 {
                EXECUTABLE_PATH = a.add(n);
            }
        }
        if !block.is_null() {
            let words = block as *const u64;
            let inits = *words.add(3) as usize;
            let table = words.add(4 + inits);
            if *table == IMAGES_MAGIC {
                IMAGE_COUNT = *table.add(1) as u32;
                IMAGES = table.add(2) as *const [u64; 3];
                BLOCK = block;
            }
        }
    }
}

#[used]
#[unsafe(link_section = "__DATA,__mod_init_func,mod_init_funcs")]
static INIT: extern "C" fn(i32, *const *const c_char, *const *const c_char, *const *const u8, *const u8) = init;

fn image(i: u32) -> Option<[u64; 3]> {
    // SAFETY: the table was checked by `init` and is read-only.
    unsafe { (i < IMAGE_COUNT).then(|| *IMAGES.add(i as usize)) }
}

#[unsafe(no_mangle)]
extern "C" fn _dyld_image_count() -> u32 {
    // SAFETY: set once by `init`.
    unsafe { IMAGE_COUNT }
}

#[unsafe(no_mangle)]
extern "C" fn _dyld_get_image_header(i: u32) -> *const c_void {
    image(i).map_or(core::ptr::null(), |im| im[0] as *const c_void)
}

#[unsafe(no_mangle)]
extern "C" fn _dyld_get_image_vmaddr_slide(i: u32) -> isize {
    image(i).map_or(0, |im| im[1] as isize)
}

#[unsafe(no_mangle)]
extern "C" fn _dyld_get_image_name(i: u32) -> *const c_char {
    // SAFETY: the path offsets point into the start block.
    image(i).map_or(core::ptr::null(), |im| unsafe { BLOCK.add(im[2] as usize) } as *const c_char)
}

/// The program's own header (image 0).
#[unsafe(no_mangle)]
extern "C" fn _dyld_get_prog_image_header() -> *const c_void {
    _dyld_get_image_header(0)
}

/// `_NSGetExecutablePath(buf, &size)`: the path and 0, or -1 with `size`
/// set to the room needed (`dyld(3)`).
#[unsafe(no_mangle)]
unsafe extern "C" fn _NSGetExecutablePath(buf: *mut c_char, size: *mut u32) -> i32 {
    // SAFETY: set once by `init`; the caller's buffer holds `*size` bytes.
    unsafe {
        let mut path = EXECUTABLE_PATH;
        if path.is_null() {
            path = _dyld_get_image_name(0) as *const u8;
        }
        if path.is_null() {
            path = c"".as_ptr() as *const u8;
        }
        let need = poly_libsystem::cstr::strlen(path) + 1;
        if (*size as usize) < need {
            *size = need as u32;
            return -1;
        }
        core::ptr::copy_nonoverlapping(path, buf as *mut u8, need);
        0
    }
}

/// Lazy binding: Poly's dyld binds every lazy pointer at load, so this is
/// never called. Its Mach-O name has no leading underscore.
#[unsafe(export_name = "\u{1}dyld_stub_binder")]
extern "C" fn dyld_stub_binder() {
    crate::die(b"poly libSystem: dyld_stub_binder called (lazy pointers are bound at load)\n")
}

/// Thread-local variables: Poly's dyld gives every descriptor its own
/// thunk, so this is never called.
#[unsafe(no_mangle)]
extern "C" fn _tlv_bootstrap() {
    crate::die(b"poly libSystem: _tlv_bootstrap called (dyld sets every thread-local thunk)\n")
}
