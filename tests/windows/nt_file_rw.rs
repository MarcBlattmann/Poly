//! NtCreateFile / NtWriteFile / NtReadFile on a new file in the test's own
//! directory (Learn: "NtCreateFile", "ZwWriteFile", "ZwReadFile",
//! "NtClose"): FILE_CREATE makes it (Information = FILE_CREATED), a second
//! FILE_CREATE collides, a synchronous write and read at explicit offsets
//! move the bytes, reading at the end is STATUS_END_OF_FILE, and
//! FILE_DELETE_ON_CLOSE removes the file when its last handle closes.
#![no_std]
#![no_main]

use poly_wtest::*;

const DATA: &[u8] = b"hello from poly-wtest\r\n";

fn create(path: &NtPath, access: u32, disposition: u32, options: u32) -> (NTSTATUS, HANDLE, IoStatusBlock) {
    let name = path.unicode_string();
    let attributes = ObjectAttributes::named(&name);
    let mut handle: HANDLE = core::ptr::null_mut();
    let mut iosb = IoStatusBlock::new();
    // SAFETY: every pointer is to a live local; no EA buffer.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &attributes,
            &mut iosb,
            core::ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            disposition,
            options | FILE_SYNCHRONOUS_IO_NONALERT | FILE_NON_DIRECTORY_FILE,
            core::ptr::null(),
            0,
        )
    };
    (status, handle, iosb)
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let Some(path) = path_beside_image(utf16!("poly-wtest-file.tmp")) else {
        t.check("path_fits", false, 0);
        return;
    };
    let (status, file, iosb) = create(&path, GENERIC_READ | GENERIC_WRITE, FILE_CREATE, 0);
    if !t.expect_status("create", status, STATUS_SUCCESS) {
        return;
    }
    t.expect("create_information", iosb.information as i64, FILE_CREATED as i64);

    let (status, _, _) = create(&path, GENERIC_READ, FILE_CREATE, 0);
    t.expect_status("create_again_collides", status, STATUS_OBJECT_NAME_COLLISION);

    let mut iosb = IoStatusBlock::new();
    let offset = 0i64;
    // SAFETY: `file` is open for synchronous writes; `DATA`, `iosb` and
    // `offset` outlive the call.
    let status = unsafe {
        NtWriteFile(
            file,
            core::ptr::null_mut(),
            core::ptr::null(),
            core::ptr::null(),
            &mut iosb,
            DATA.as_ptr().cast(),
            DATA.len() as u32,
            &offset,
            core::ptr::null(),
        )
    };
    t.expect_status("write", status, STATUS_SUCCESS);
    t.expect("write_information", iosb.information as i64, DATA.len() as i64);

    let mut buf = [0u8; 64];
    let mut iosb = IoStatusBlock::new();
    // SAFETY: as above; `buf` is writable for its length.
    let status = unsafe {
        NtReadFile(
            file,
            core::ptr::null_mut(),
            core::ptr::null(),
            core::ptr::null(),
            &mut iosb,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &offset,
            core::ptr::null(),
        )
    };
    t.expect_status("read", status, STATUS_SUCCESS);
    t.expect("read_information", iosb.information as i64, DATA.len() as i64);
    t.check("read_same_bytes", buf[..DATA.len()] == *DATA, 0);

    let end = DATA.len() as i64;
    let mut iosb = IoStatusBlock::new();
    // SAFETY: as above.
    let status = unsafe {
        NtReadFile(
            file,
            core::ptr::null_mut(),
            core::ptr::null(),
            core::ptr::null(),
            &mut iosb,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &end,
            core::ptr::null(),
        )
    };
    t.expect_status("read_at_end", status, STATUS_END_OF_FILE);

    // SAFETY: closing the handle opened above, once.
    t.expect_status("close", unsafe { NtClose(file) }, STATUS_SUCCESS);

    let (status, file, iosb) = create(&path, GENERIC_READ | DELETE, FILE_OPEN, FILE_DELETE_ON_CLOSE);
    t.expect_status("open_delete_on_close", status, STATUS_SUCCESS);
    t.expect("open_information", iosb.information as i64, FILE_OPENED as i64);
    if status == STATUS_SUCCESS {
        // SAFETY: closing the handle just opened, once.
        t.expect_status("close_deletes", unsafe { NtClose(file) }, STATUS_SUCCESS);
    }
    let (status, _, _) = create(&path, GENERIC_READ, FILE_OPEN, 0);
    t.expect_status("gone_after_close", status, STATUS_OBJECT_NAME_NOT_FOUND);
}
