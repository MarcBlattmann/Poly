//! In-kernel tests of System V shared memory (`shm.rs`): a segment's
//! attachments in memory maps, as shmat(2), fork, shmdt(2) and exit make
//! and end them.

use super::shm;
use crate::kernel_test;
use crate::user::memory::{Layout, PAGE_SIZE, Placement, Prot};
use crate::user::{self, Memory};
use poly_personality_linux::errno::Errno;

type TestResult<T = ()> = Result<T, &'static str>;

const P: u64 = PAGE_SIZE;

/// The user frame count once threads of processes that ended (earlier
/// tests') are gone and it has not changed for a while.
fn settled_frames() -> u64 {
    let deadline = crate::time::now_ns() + 30 * crate::time::NANOS_PER_SEC;
    while crate::thread::dying_user_threads() != 0 && crate::time::now_ns() < deadline {
        crate::thread::sleep(core::time::Duration::from_millis(2));
    }
    let mut last = user::user_frames();
    loop {
        crate::thread::sleep(core::time::Duration::from_millis(20));
        let now = user::user_frames();
        if now == last {
            return now;
        }
        last = now;
    }
}

fn frames_reach(expected: u64) -> TestResult {
    let deadline = crate::time::now_ns() + 30 * crate::time::NANOS_PER_SEC;
    while user::user_frames() != expected {
        if crate::time::now_ns() > deadline {
            crate::kprintln!("shm test: {} user frames, expected {expected}", user::user_frames());
            return Err("a segment's pages were not freed with it");
        }
        crate::thread::sleep(core::time::Duration::from_millis(2));
    }
    Ok(())
}

fn read(m: &Memory, addr: u64, buf: &mut [u8]) -> TestResult {
    m.read(addr, buf).map_err(|_| "read failed")
}

fn write(m: &Memory, addr: u64, data: &[u8]) -> TestResult {
    m.write(addr, data).map_err(|_| "write failed")
}

kernel_test! {
    fn sysv_shared_memory_is_zeroed_shared_by_attachments_and_fork_and_freed_last() -> TestResult {
        let before = settled_frames();
        {
            let parent = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
            let seg = shm::create(2 * P);
            let attach = |m: &Memory, placement, prot| shm::attach(m, seg.as_ref(), placement, 2 * P, prot);
            let a = attach(&parent, Placement::Hint(0), Prot::RW).map_err(|_| "attach failed")?;
            let b = attach(&parent, Placement::Hint(0), Prot::RW).map_err(|_| "attach failed")?;
            if a == b || seg.attachments() != 2 {
                return Err("two attachments are not two mappings");
            }
            let mut page = alloc::vec![0xffu8; 2 * PAGE_SIZE as usize];
            read(&parent, a, &mut page)?;
            if page.iter().any(|b| *b != 0) {
                return Err("a new segment does not read as zeros");
            }
            write(&parent, a, &[1])?;
            write(&parent, a + P, &[2])?;
            let mut one = [0u8; 1];
            read(&parent, b + P, &mut one)?;
            if one != [2] {
                return Err("the attachments do not share the pages");
            }

            let child = parent.fork().map_err(|_| "fork failed")?;
            if seg.attachments() != 4 {
                return Err("the child's copies of the attachments are not counted");
            }
            write(&child, a + 3, &[42])?;
            read(&parent, b + 3, &mut one)?;
            if one != [42] {
                return Err("the child's write did not reach the parent");
            }
            let serials = shm::detach_all(&child);
            if serials != [seg.serial(), seg.serial()] || seg.attachments() != 2 {
                return Err("exit did not detach the child's attachments");
            }
            if child.read(a, &mut one).is_ok() {
                return Err("a detached attachment is still mapped");
            }

            if shm::detach(&parent, b + P) != Err(Errno::EINVAL) {
                return Err("shmdt inside an attachment is not EINVAL");
            }
            if shm::detach(&parent, a) != Ok(seg.serial()) || shm::detach(&parent, a) != Err(Errno::EINVAL) {
                return Err("shmdt did not detach exactly once");
            }
            if attach(&parent, Placement::FixedNoReplace(b), Prot::RW) != Err(Errno::EINVAL) {
                return Err("attaching over a mapping without SHM_REMAP is not EINVAL");
            }
            let ro = attach(&parent, Placement::FixedNoReplace(a), Prot::READ).map_err(|_| "attach failed")?;
            if parent.write(ro, &[7]).is_ok() || parent.protect(ro, P, Prot::RW).is_ok() {
                return Err("a read-only attachment can be made writable");
            }
            read(&parent, ro + P, &mut one)?;
            if one != [2] || seg.attachments() != 2 {
                return Err("the read-only attachment does not see the pages");
            }
            drop(child);
            shm::detach(&parent, ro).map_err(|_| "shmdt failed")?;
            shm::detach(&parent, b).map_err(|_| "shmdt failed")?;
            if seg.attachments() != 0 {
                return Err("attachments left after the last shmdt");
            }
        }
        frames_reach(before)
    }
}

kernel_test! {
    fn sysv_shared_memory_attachments_unmap_place_and_protect_as_shmat_says() -> TestResult {
        let m = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
        let seg = shm::create(3 * P);
        let attach = |placement, prot| shm::attach(&m, seg.as_ref(), placement, 3 * P, prot);
        if attach(Placement::FixedNoReplace(0), Prot::RW) != Err(Errno::EPERM) {
            return Err("an attachment below the lowest address is not EPERM");
        }
        let end = poly_vm::USER_END;
        if attach(Placement::FixedNoReplace(end - P), Prot::RW) != Err(Errno::ENOMEM)
            || attach(Placement::Fixed(0xffff_8000_0000_0000), Prot::READ) != Err(Errno::ENOMEM)
        {
            return Err("an attachment past the end of user space is not ENOMEM");
        }
        let a = attach(Placement::Hint(0), Prot::RW).map_err(|_| "attach failed")?;
        let ro = attach(Placement::Hint(0), Prot::READ).map_err(|_| "attach failed")?;
        if m.protect(a, 3 * P, Prot::RW | Prot::EXEC).is_err() {
            return Err("a read-write attachment cannot be made executable");
        }
        if m.protect(ro, P, Prot::NONE).is_err() || m.protect(ro, P, Prot::READ).is_err() {
            return Err("a read-only attachment cannot go to PROT_NONE and back");
        }
        if shm::mapped(&m, a + P, P) != [seg.serial()] || !shm::mapped(&m, a + 3 * P, P).is_empty() {
            return Err("the attachments in a range are wrong");
        }
        m.unmap(a, P).map_err(|_| "munmap failed")?;
        write(&m, a + 2 * P, &[5])?;
        let mut one = [0u8; 1];
        read(&m, ro + 2 * P, &mut one)?;
        if seg.attachments() != 2 || one != [5] {
            return Err("a partly unmapped attachment is not attached any more");
        }
        if shm::detach(&m, a + P) != Err(Errno::EINVAL) {
            return Err("shmdt of the start of what is left of an attachment is not EINVAL");
        }
        m.unmap(a + P, 2 * P).map_err(|_| "munmap failed")?;
        if seg.attachments() != 1 {
            return Err("unmapping all of an attachment does not detach it");
        }
        shm::detach(&m, ro).map_err(|_| "shmdt failed")?;
        Ok(())
    }
}

kernel_test! {
    fn shmdt_finds_an_attachment_by_its_offset_zero_and_removes_only_its_pieces() -> TestResult {
        let m = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
        let seg = shm::create(3 * P);
        let attach = |placement| shm::attach(&m, seg.as_ref(), placement, 3 * P, Prot::RW);
        let mut one = [0u8; 1];

        // The head unmapped and refilled by anonymous memory.
        let a = attach(Placement::Hint(0)).map_err(|_| "attach failed")?;
        m.unmap(a, P).map_err(|_| "munmap failed")?;
        m.map_anonymous(Placement::Fixed(a), P, Prot::RW).map_err(|_| "mmap failed")?;
        if shm::detach(&m, a + P) != Err(Errno::EINVAL) {
            return Err("the start of what is left is an attach address");
        }
        if shm::detach(&m, a) != Ok(seg.serial()) || seg.attachments() != 0 {
            return Err("shmdt of the original start does not detach the rest");
        }
        if m.read(a, &mut one).is_err() || m.read(a + P, &mut one).is_ok() {
            return Err("shmdt removed the wrong pages");
        }
        m.unmap(a, P).map_err(|_| "munmap failed")?;

        // A middle page refilled, the same segment attached right after.
        let b = attach(Placement::Hint(0)).map_err(|_| "attach failed")?;
        let next = attach(Placement::FixedNoReplace(b + 3 * P)).map_err(|_| "attach failed")?;
        m.unmap(b + P, P).map_err(|_| "munmap failed")?;
        m.map_anonymous(Placement::Fixed(b + P), P, Prot::RW).map_err(|_| "mmap failed")?;
        write(&m, b + P, &[9])?;
        shm::detach(&m, b).map_err(|_| "shmdt failed")?;
        read(&m, b + P, &mut one)?;
        if one != [9] || m.read(b, &mut one).is_ok() || m.read(b + 2 * P, &mut one).is_ok() {
            return Err("shmdt touched what was put in the attachment's hole");
        }
        if seg.attachments() != 1 || m.read(next, &mut one).is_err() {
            return Err("shmdt removed the neighbouring attachment");
        }
        shm::detach(&m, next).map_err(|_| "shmdt failed")?;
        m.unmap(b + P, P).map_err(|_| "munmap failed")?;

        // Split by mprotect: one shmdt takes every piece.
        let c = attach(Placement::Hint(0)).map_err(|_| "attach failed")?;
        m.protect(c + P, P, Prot::READ).map_err(|_| "mprotect failed")?;
        if shm::detach(&m, c + P) != Err(Errno::EINVAL) {
            return Err("the start of a piece is an attach address");
        }
        shm::detach(&m, c).map_err(|_| "shmdt failed")?;
        if seg.attachments() != 0 || !shm::mapped(&m, c, 3 * P).is_empty() {
            return Err("shmdt left pieces of a split attachment");
        }

        // Moved whole by mremap: the new start, not the old.
        let d = attach(Placement::Hint(0)).map_err(|_| "attach failed")?;
        let to = d + 16 * P;
        let moved = m.remap(d, 3 * P, 3 * P, poly_vm::RemapPlacement::Fixed(to)).map_err(|_| "mremap failed")?;
        if moved != to || shm::detach(&m, d) != Err(Errno::EINVAL) {
            return Err("the old start of a moved attachment is an attach address");
        }
        shm::detach(&m, to).map_err(|_| "shmdt of the new start failed")?;
        if seg.attachments() != 0 {
            return Err("the moved attachment is still attached");
        }
        Ok(())
    }
}
