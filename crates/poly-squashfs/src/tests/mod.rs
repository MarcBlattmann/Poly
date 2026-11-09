//! Host tests against the images of the visible tests (mksquashfs 4.6.1).

mod images;

use crate::{IoFailed, MountError, Source, Squashfs};
use alloc::vec::Vec;
use poly_vfs::{Errno, FileSystem, FileType, Ino};

/// An image padded to whole 4096-byte blocks, as the tests write it.
struct Mem(Vec<u8>);

impl Mem {
    fn new(img: &[u8]) -> Mem {
        let mut v = img.to_vec();
        v.resize(v.len().div_ceil(4096) * 4096, 0);
        Mem(v)
    }
}

impl Source for Mem {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), IoFailed> {
        let o = offset as usize;
        buf.copy_from_slice(self.0.get(o..o + buf.len()).ok_or(IoFailed)?);
        Ok(())
    }
}

fn mount(img: &[u8]) -> Squashfs<Mem> {
    Squashfs::mount(Mem::new(img)).expect("mounts")
}

fn path(fs: &Squashfs<Mem>, p: &str) -> Result<Ino, Errno> {
    let mut ino = fs.root();
    for part in p.split('/').filter(|s| !s.is_empty()) {
        ino = fs.lookup(ino, part.as_bytes())?;
    }
    Ok(ino)
}

fn read_all(fs: &Squashfs<Mem>, ino: Ino) -> Vec<u8> {
    let size = fs.getattr(ino).unwrap().size as usize;
    let mut v = alloc::vec![0u8; size];
    let mut at = 0;
    while at < size {
        let n = fs.read(ino, at as u64, &mut v[at..(at + 1000).min(size)]).unwrap();
        assert!(n > 0);
        at += n;
    }
    v
}

fn pattern() -> Vec<u8> {
    b"0123456789abcdef\n".iter().copied().cycle().take(9000).collect()
}

fn check_small(img: &[u8], block: u64, files: u64) {
    let fs = mount(img);
    let st = fs.statfs().unwrap();
    assert_eq!(st.magic, 0x7371_7368);
    assert_eq!(st.block_size, block);
    assert_eq!(st.blocks, 1);
    assert_eq!(st.files, files);
    assert_eq!(read_all(&fs, path(&fs, "hello.txt").unwrap()), b"hello squashfs\n");
    assert_eq!(read_all(&fs, path(&fs, "d/inner.txt").unwrap()), b"inner\n");
    assert_eq!(read_all(&fs, path(&fs, "pattern.txt").unwrap()), pattern());
    let l = path(&fs, "l").unwrap();
    assert_eq!(fs.readlink(l).unwrap(), b"hello.txt");
    assert_eq!(fs.getattr(l).unwrap().kind, FileType::Symlink);
    let root = fs.getattr(fs.root()).unwrap();
    assert_eq!((root.perm, root.nlink), (0o755, 3));
}

#[test]
fn small_gzip() {
    check_small(&images::SMALL_GZIP, 4096, 6);
}

#[test]
fn small_xz() {
    check_small(&images::SMALL_XZ, 4096, 6);
}

#[test]
fn small_zstd() {
    check_small(&images::SMALL_ZSTD, 4096, 6);
}

#[test]
fn small_lz4() {
    check_small(&images::SMALL_LZ4, 4096, 6);
}

#[test]
fn small_lzo() {
    check_small(&images::SMALL_LZO, 4096, 6);
}

#[test]
fn small_other_block_sizes() {
    check_small(&images::SMALL_B128K, 131072, 6);
    check_small(&images::SMALL_B1M, 1 << 20, 6);
    check_small(&images::SMALL_NOFRAG, 4096, 6);
}

#[test]
fn duplicates_are_separate_inodes() {
    for img in [&images::DUPS[..], &images::NODUPS[..]] {
        let fs = mount(img);
        assert_eq!(fs.statfs().unwrap().files, 5);
        let mut inos = Vec::new();
        for n in ["one", "two", "three"] {
            let i = path(&fs, n).unwrap();
            let a = fs.getattr(i).unwrap();
            assert_eq!((a.nlink, a.perm, a.size), (1, 0o644, 18));
            assert_eq!(read_all(&fs, i), b"same content here\n");
            inos.push(i);
        }
        inos.sort();
        inos.dedup();
        assert_eq!(inos.len(), 3);
        assert_eq!(read_all(&fs, path(&fs, "four").unwrap()), b"other\n");
    }
}

#[test]
fn tree_inodes() {
    let fs = mount(&images::TREE1);
    assert_eq!(fs.statfs().unwrap().files, 323);
    let a = |p: &str| fs.getattr(path(&fs, p).unwrap()).unwrap();
    assert_eq!((a("suid").perm, a("tmp").perm), (0o4755, 0o1777));
    assert_eq!((a("owned").uid, a("owned").gid, a("owned").perm), (1000, 2000, 0o640));
    assert_eq!((a("dir").uid, a("dir").gid, a("dir").nlink), (5, 6, 3));
    assert_eq!((a("tmp").nlink, fs.getattr(fs.root()).unwrap().nlink), (2, 5));
    assert_eq!((a("hard1").nlink, a("hard1").ino), (2, a("hard2").ino));
    assert_eq!(a("link").size, 9);
    assert_eq!(a("longlink").size, 209);
    assert_eq!(a("chr").kind, FileType::CharDevice);
    assert_eq!((a("chr").rdev.major, a("chr").rdev.minor), (1, 3));
    assert_eq!((a("blk").rdev.major, a("blk").rdev.minor), (7, 42));
    assert_eq!(a("fifo").kind, FileType::Fifo);
    assert_eq!(a("hello.txt").mtime.sec, 0);
    assert_eq!(a("hello.txt").blksize, 1024);
    assert_eq!(fs.lookup(path(&fs, "dir/sub").unwrap(), b"..").unwrap(), path(&fs, "dir").unwrap());
    assert_eq!(fs.lookup(fs.root(), b".."), Ok(fs.root()));
    assert_eq!(fs.lookup(fs.root(), b"nothing"), Err(Errno::ENOENT));
    assert_eq!(fs.lookup(path(&fs, "hello.txt").unwrap(), b"x"), Err(Errno::ENOTDIR));
}

#[test]
fn tree_data() {
    let fs = mount(&images::TREE1);
    let big = read_all(&fs, path(&fs, "big.bin").unwrap());
    assert_eq!(big.len(), 10000);
    for (i, b) in big.iter().enumerate() {
        assert_eq!(*b as usize, (i * 7 + i / 251) % 256, "byte {i}");
    }
    let sparse = read_all(&fs, path(&fs, "sparse.bin").unwrap());
    assert_eq!(sparse.len(), 12388);
    assert!(sparse[..12288].iter().all(|&b| b == 0));
    assert!(sparse[12288..].iter().all(|&b| b == b'S'));
    for k in [0usize, 17, 255, 299] {
        let name = alloc::format!("many/f{k:03}");
        assert_eq!(read_all(&fs, path(&fs, &name).unwrap()), alloc::format!("{:x}", k % 16).into_bytes());
    }
    let mut buf = [0u8; 20];
    let big_ino = path(&fs, "big.bin").unwrap();
    assert_eq!(fs.read(big_ino, 4090, &mut buf), Ok(20));
    assert_eq!(fs.read(big_ino, 10000, &mut buf), Ok(0));
    let t = fs.readlink(path(&fs, "longlink").unwrap()).unwrap();
    assert_eq!(t.len(), 209);
    assert!(t.ends_with(b"hello.txt"));
}

#[test]
fn tree_readdir() {
    let fs = mount(&images::TREE1);
    let many = path(&fs, "many").unwrap();
    let mut names: Vec<Vec<u8>> = Vec::new();
    fs.readdir(many, 0, &mut |e| {
        names.push(e.name.to_vec());
        true
    })
    .unwrap();
    assert_eq!(names.len(), 302);
    assert_eq!((&names[0][..], &names[1][..]), (&b"."[..], &b".."[..]));
    assert_eq!(names[2], b"f000");
    assert_eq!(names[301], b"f299");
    // Listing from a position lists the rest.
    let mut rest = Vec::new();
    fs.readdir(many, 150, &mut |e| {
        rest.push(e.name.to_vec());
        true
    })
    .unwrap();
    assert_eq!(rest, names[150..]);
    let mut stop = 0;
    fs.readdir(many, 0, &mut |_| {
        stop += 1;
        stop < 5
    })
    .unwrap();
    assert_eq!(stop, 5);
    let root_names = {
        let mut v = Vec::new();
        fs.readdir(fs.root(), 0, &mut |e| {
            v.push((e.name.to_vec(), e.kind));
            true
        })
        .unwrap();
        v
    };
    assert!(root_names.contains(&(b"fifo".to_vec(), FileType::Fifo)));
    assert!(root_names.contains(&(b"dlink".to_vec(), FileType::Symlink)));
}

#[test]
fn tree_xattrs() {
    let fs = mount(&images::TREE1);
    let xa = path(&fs, "xa").unwrap();
    assert_eq!(fs.listxattr(xa).unwrap(), b"user.alpha\0user.beta\0");
    assert_eq!(fs.getxattr(xa, b"user.alpha").unwrap(), b"one");
    assert_eq!(fs.getxattr(xa, b"user.beta").unwrap(), b"two");
    assert_eq!(fs.getxattr(xa, b"user.gamma"), Err(Errno::ENODATA));
    assert_eq!(fs.listxattr(path(&fs, "hello.txt").unwrap()).unwrap(), b"");
}

#[test]
fn read_only() {
    let fs = mount(&images::TREE1);
    let r = fs.root();
    assert_eq!(fs.mkdir(r, b"x", 0o755, 0, 0), Err(Errno::EROFS));
    assert_eq!(fs.unlink(r, b"hello.txt"), Err(Errno::EROFS));
    assert_eq!(fs.write(path(&fs, "hello.txt").unwrap(), 0, b"x"), Err(Errno::EROFS));
    assert!(fs.info().read_only);
}

#[test]
fn bad_images() {
    let ok = images::SMALL_GZIP.to_vec();
    let with = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut v = ok.clone();
        f(&mut v);
        Squashfs::<Mem>::mount(Mem::new(&v)).err()
    };
    assert_eq!(with(&|v| v[0] = 0), Some(MountError::Invalid));
    assert_eq!(with(&|v| v[28] = 3), Some(MountError::Invalid));
    assert_eq!(with(&|v| v[28] = 5), Some(MountError::Invalid));
    assert_eq!(with(&|v| v[22] = 11), Some(MountError::Invalid));
    assert_eq!(with(&|v| v[12..16].copy_from_slice(&5000u32.to_le_bytes())), Some(MountError::Invalid));
    assert_eq!(with(&|v| v[12..16].copy_from_slice(&2048u32.to_le_bytes())), Some(MountError::Invalid));
    assert_eq!(with(&|v| v[20] = 0), Some(MountError::Invalid));
    assert_eq!(with(&|v| v[20] = 99), Some(MountError::Invalid));
    assert_eq!(with(&|v| v[20] = 2), Some(MountError::Invalid));
    assert_eq!(with(&|v| v[40..48].copy_from_slice(&(1u64 << 30).to_le_bytes())), Some(MountError::Invalid));
    // The superblock names another compressor than the data uses.
    assert_eq!(with(&|v| v[20] = 4), Some(MountError::Io));
    assert_eq!(Squashfs::<Mem>::mount(Mem(Vec::new())).err(), Some(MountError::Io));
    assert_eq!(Squashfs::<Mem>::mount(Mem(alloc::vec![0u8; 4096])).err(), Some(MountError::Invalid));
    assert_eq!(Squashfs::<Mem>::mount(Mem(Mem::new(&ok).0[..512].to_vec())).err(), Some(MountError::Io));
}
