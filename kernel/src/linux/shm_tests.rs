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
        // Pieces of one attachment count once (shmdt's spec sheet).
        if m.protect(a + P, P, Prot::READ).is_err() || seg.attachments() != 2 {
            return Err("an attachment split by mprotect does not count once");
        }
        if shm::mapped(&m, a, 3 * P) != [seg.serial()] {
            return Err("the pieces of one attachment are not one attachment");
        }
        m.unmap(a, P).map_err(|_| "munmap failed")?;
        write(&m, a + 2 * P, &[5])?;
        let mut one = [0u8; 1];
        read(&m, ro + 2 * P, &mut one)?;
        if seg.attachments() != 2 || one != [5] {
            return Err("a partly unmapped attachment is not attached any more");
        }
        // The start of a later piece names nothing; the attach address
        // still names the attachment without its first page.
        if shm::detach(&m, a + P) != Err(Errno::EINVAL) {
            return Err("shmdt at a later piece is not EINVAL");
        }
        if shm::detach(&m, a) != Ok(seg.serial()) || seg.attachments() != 1 || m.read(a + 2 * P, &mut one).is_ok() {
            return Err("shmdt of the attach address did not detach what is left");
        }
        shm::detach(&m, ro).map_err(|_| "shmdt failed")?;
        if seg.attachments() != 0 {
            return Err("attachments left after the last shmdt");
        }
        Ok(())
    }
}
