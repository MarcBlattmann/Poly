//! memfd_create(2)'s files and their seals (fcntl(2), "File sealing").

use super::*;
use crate::memfd::{Memfd, seal};

fn memfd_file(env: &Env, perm: u16, seals: u32) -> Arc<OpenFile> {
    let mount = env.vfs.detached_mount(tmpfs());
    env.vfs.memfd_file(&mount, &env.root, perm, Memfd::new(b"test", seals)).unwrap()
}

#[test]
fn a_memfd_is_an_unlinked_regular_file_open_for_reading_and_writing() {
    let env = Env::new();
    let f = memfd_file(&env, 0o777, 0);
    let st = f.stat().unwrap();
    assert_eq!(st.mode, S_IFREG | 0o777);
    assert_eq!(st.nlink, 0);
    assert_eq!(st.size, 0);
    assert_eq!(f.flags(), OpenFlags::RDWR | OpenFlags::LARGEFILE);
    assert!(f.is_unnameable());
    assert_eq!(f.write(b"hello"), Ok(5));
    let mut buf = [0u8; 5];
    assert_eq!(f.read_at(0, &mut buf), Ok(5));
    assert_eq!(&buf, b"hello");
    assert_eq!(env.vfs.file_path(&env.ctx, &f).unwrap(), b"/memfd:test (deleted)");
    // Not reachable from the tree.
    assert_eq!(env.ls("/"), Vec::<std::string::String>::new());
}

#[test]
fn seals_add_up_until_f_seal_seal() {
    let m = Memfd::new(b"", 0);
    assert_eq!(m.add_seals(0x40), Err(Errno::EINVAL));
    assert_eq!(m.add_seals(seal::SHRINK), Ok(()));
    assert_eq!(m.add_seals(seal::GROW | seal::SEAL), Ok(()));
    assert_eq!(m.seals(), seal::SHRINK | seal::GROW | seal::SEAL);
    assert_eq!(m.add_seals(seal::WRITE), Err(Errno::EPERM));
    assert_eq!(m.add_seals(seal::SHRINK), Err(Errno::EPERM), "also a seal already set");
    assert_eq!(m.add_seals(0), Err(Errno::EPERM));
    let sealed = Memfd::new(b"", seal::SEAL);
    assert_eq!(sealed.add_seals(seal::WRITE), Err(Errno::EPERM));
}

#[test]
fn size_seals_refuse_shrinking_and_growing() {
    let env = Env::new();
    let f = memfd_file(&env, 0o777, 0);
    assert_eq!(f.write(b"abcdef"), Ok(6));
    let m = f.memfd().unwrap().clone();
    m.add_seals(seal::SHRINK).unwrap();
    assert_eq!(f.truncate(3), Err(Errno::EPERM));
    assert_eq!(f.truncate(100), Ok(()));
    assert_eq!(f.truncate(100), Ok(()));
    m.add_seals(seal::GROW).unwrap();
    assert_eq!(f.truncate(200), Err(Errno::EPERM));
    assert_eq!(f.write_at(98, b"zz"), Ok(2));
    assert_eq!(f.write_at(99, b"zz"), Err(Errno::EPERM));
    assert_eq!(f.seek(99, Whence::Set), Ok(99));
    assert_eq!(f.write(b"zz"), Err(Errno::EPERM));
    assert_eq!(f.stat().unwrap().size, 100);
}

#[test]
fn write_seals_refuse_writes_and_writable_shared_mappings() {
    let env = Env::new();
    let f = memfd_file(&env, 0o777, 0);
    let m = f.memfd().unwrap().clone();
    m.add_seals(seal::WRITE).unwrap();
    assert_eq!(f.write(b"x"), Err(Errno::EPERM));
    assert_eq!(f.write_at(0, b"x"), Err(Errno::EPERM));
    assert_eq!(f.write(b""), Ok(0));
    assert_eq!(f.truncate(10), Ok(()), "F_SEAL_WRITE leaves the size alone");
    assert_eq!(m.map(true, true).err(), Some(Errno::EPERM));
    assert!(m.map(true, false).unwrap().is_none());
    assert!(m.map(false, true).unwrap().is_none());

    let future = Memfd::new(b"", 0);
    future.add_seals(seal::FUTURE_WRITE).unwrap();
    assert_eq!(future.check_write(0, 1, 0), Err(Errno::EPERM));
    assert_eq!(future.map(true, true).err(), Some(Errno::EPERM));
}

#[test]
fn a_writable_shared_mapping_blocks_f_seal_write_until_it_is_gone() {
    let m = Memfd::new(b"", 0);
    let a = m.map(true, true).unwrap();
    let b = m.map(true, true).unwrap();
    assert_eq!(m.writable_maps(), 2);
    assert_eq!(m.add_seals(seal::WRITE), Err(Errno::EBUSY));
    assert_eq!(m.seals(), 0);
    assert_eq!(m.add_seals(seal::FUTURE_WRITE | seal::GROW), Ok(()), "only F_SEAL_WRITE waits for mappings");
    drop(a);
    assert_eq!(m.add_seals(seal::WRITE), Err(Errno::EBUSY));
    drop(b);
    assert_eq!(m.writable_maps(), 0);
    assert_eq!(m.add_seals(seal::WRITE), Ok(()));
}

#[test]
fn f_seal_exec_keeps_the_execute_bits() {
    let env = Env::new();
    let f = memfd_file(&env, 0o666, seal::EXEC);
    for mode in [0o777, 0o766, 0o667] {
        assert_eq!(f.chmod(&env.root, mode), Err(Errno::EPERM));
    }
    assert_eq!(f.stat().unwrap().mode, S_IFREG | 0o666);
    assert_eq!(f.chmod(&env.root, 0o600), Ok(()));
    assert_eq!(f.stat().unwrap().mode, S_IFREG | 0o600);

    let x = memfd_file(&env, 0o777, 0);
    x.memfd().unwrap().add_seals(seal::EXEC).unwrap();
    assert_eq!(x.chmod(&env.root, 0o666), Err(Errno::EPERM));
    assert_eq!(x.chmod(&env.root, 0o777), Ok(()));
    assert_eq!(x.chmod(&env.root, 0o555), Ok(()));
}

#[test]
fn a_memfd_path_carries_any_name() {
    assert_eq!(Memfd::new(b"", 0).path(), b"/memfd: (deleted)");
    assert_eq!(Memfd::new(b"a/b c", 0).path(), b"/memfd:a/b c (deleted)");
}
