//! Writing the image (mke2fs or poly-ext2), making it and the boot arguments.

use super::*;

/// Whether this process runs as root (`/proc/self` belongs to its user).
fn running_as_root() -> bool {
    crate::hostfs::running_as_root()
}

/// Whether `mke2fs` can be run.
fn have_mke2fs() -> bool {
    Command::new("mke2fs").arg("-V").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

/// The size of the image for `tree`: its contents with room to spare
/// (the boot test writes programs and Python's byte code), whole MiB.
pub(super) fn image_size(tree: &RootTree) -> (u64, u64) {
    let (bytes, entries) = tree.totals();
    let used = bytes + entries * 4096;
    let size = (used + used / 4 + (160 << 20)).div_ceil(4 << 20) * (4 << 20);
    let inodes = (entries * 2 + 16384).div_ceil(1024) * 1024;
    (size, inodes)
}

/// The mke2fs.conf the image is made with: the ext4 features `poly-ext2`
/// supports, whatever the host's own configuration says.
const MKE2FS_CONF: &str = "\
[defaults]
\tbase_features = sparse_super,large_file,filetype,resize_inode,dir_index,ext_attr
\tblocksize = 4096
\tinode_size = 256
\tinode_ratio = 16384
[fs_types]
\text4 = {
\t\tfeatures = has_journal,extent,huge_file,flex_bg,metadata_csum,64bit,dir_nlink,extra_isize
\t}
";

/// Write `tree` as an ext4 image to `image`; how it was made.
pub fn write_image(tree: &RootTree, image: &Path) -> Result<&'static str, String> {
    let (size, inodes) = image_size(tree);
    let _ = std::fs::remove_file(image);
    // POLY_ROOTDISK_FORMAT=poly-ext2 uses the repository's formatter even with e2fsprogs.
    let ours = std::env::var("POLY_ROOTDISK_FORMAT").is_ok_and(|v| v == "poly-ext2");
    // `mke2fs -d` gives every file the owner of its staged copy, and its
    // `root_owner` option covers only the root directory: run as anyone but
    // root (CI), the image's files would belong to that user. The
    // repository's formatter writes them as root.
    if !ours && have_mke2fs() && running_as_root() {
        with_mke2fs(tree, image, size, inodes)?;
        Ok("mke2fs -t ext4 -d")
    } else {
        with_poly_ext2(tree, image, size)?;
        Ok("poly-ext2's format")
    }
}

/// Write `tree` as an ext4 image to `image` with `poly-ext2`'s formatter on
/// any host, whatever tools it has: the modes are the tree's own (a host
/// file's mode is not looked at, unlike `mke2fs -d`), so a tree made from
/// archive entries keeps theirs (Chromium's disk, `chromium.rs`).
pub fn write_image_poly_ext2(tree: &RootTree, image: &Path) -> Result<(), String> {
    let (size, _) = image_size(tree);
    let _ = std::fs::remove_file(image);
    with_poly_ext2(tree, image, size)
}

fn with_mke2fs(tree: &RootTree, image: &Path, size: u64, inodes: u64) -> Result<(), String> {
    let target = crate::root().join("target");
    let stage = target.join(format!("poly-rootdisk-stage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
    let result = (|| -> Result<(), String> {
        let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
        let mut dirs = Vec::new();
        for (path, entry) in &tree.entries {
            let at = stage.join(path);
            match entry {
                Entry::Dir(mode) => {
                    std::fs::create_dir_all(&at).map_err(|e| err(&at, e))?;
                    dirs.push((at, *mode));
                }
                Entry::Symlink(t) => crate::hostfs::symlink(t, &at).map_err(|e| err(&at, e))?,
                Entry::File { mode, source, mtime } => {
                    match source {
                        // A hard link costs no space; the host's file is
                        // never written through it.
                        Source::Host(host) => {
                            if std::fs::hard_link(host, &at).is_err() {
                                std::fs::copy(host, &at).map_err(|e| err(&at, e))?;
                                set_mtime(&at, *mtime)?;
                            }
                        }
                        Source::Bytes(b) => {
                            std::fs::write(&at, b).map_err(|e| err(&at, e))?;
                            crate::hostfs::set_mode(&at, u32::from(*mode)).map_err(|e| err(&at, e))?;
                            set_mtime(&at, *mtime)?;
                        }
                    }
                }
            }
        }
        for (at, mode) in dirs.iter().rev() {
            crate::hostfs::set_mode(at, u32::from(*mode)).map_err(|e| err(at, e))?;
        }
        let conf = stage.with_extension("mke2fs.conf");
        std::fs::write(&conf, MKE2FS_CONF).map_err(|e| err(&conf, e))?;
        let out = Command::new("mke2fs")
            .env("MKE2FS_CONFIG", &conf)
            .args(["-q", "-F", "-t", "ext4", "-b", "4096", "-L", "poly-root"])
            .args(["-U", "504f4c59-726f-6f74-2d64-69736b000001", "-E", "root_owner=0:0", "-N"])
            .arg(inodes.to_string())
            .arg("-d")
            .arg(&stage)
            .arg(image)
            .arg(format!("{}k", size / 1024))
            .output()
            .map_err(|e| format!("mke2fs: {e}"))?;
        let _ = std::fs::remove_file(&conf);
        if !out.status.success() {
            return Err(format!("mke2fs -d failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&stage);
    result
}

fn set_mtime(path: &Path, mtime: i64) -> Result<(), String> {
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime.max(0) as u64);
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(when))
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub(super) struct FixedClock;

impl poly_vfs::Clock for FixedClock {
    fn now(&self) -> poly_vfs::Timespec {
        poly_vfs::Timespec::new(MTIME, 0)
    }
}

fn with_poly_ext2(tree: &RootTree, image: &Path, size: u64) -> Result<(), String> {
    use poly_block::RamDisk;
    use poly_ext2::{DeviceIo, Ext2, FormatOptions, MountOptions};
    use poly_vfs::{DeviceId, FileSystem, FileType, NewNode, SetAttr, SetTime, Timespec};
    let mut mem = vec![0u8; usize::try_from(size).map_err(|_| "image too big")?];
    let err = |what: &str, e: poly_vfs::Errno| format!("root disk: {what}: {}", e.name());
    {
        let disk = RamDisk::new(&mut mem, 512).ok_or("root disk: bad RAM disk")?;
        let io = DeviceIo::new(disk);
        let opts = FormatOptions {
            uuid: *b"POLYroot-disk\0\0\x01",
            label: *b"poly-root\0\0\0\0\0\0\0",
            time: Timespec::new(MTIME, 0),
            ..FormatOptions::ext4(4096, 4096)
        };
        poly_ext2::format(&io, &opts).map_err(|e| err("format", e))?;
        let fs = Ext2::mount(&io, std::sync::Arc::new(FixedClock), MountOptions::default())
            .map_err(|e| format!("root disk: mount: {e}"))?;
        let mut inos: BTreeMap<String, poly_vfs::Ino> = BTreeMap::new();
        inos.insert(String::new(), fs.root());
        for (path, entry) in &tree.entries {
            let (parent, name) = match path.rsplit_once('/') {
                Some((p, n)) => (p.to_string(), n),
                None => (String::new(), path.as_str()),
            };
            let dir = *inos.get(&parent).ok_or_else(|| format!("root disk: no directory for {path}"))?;
            match entry {
                Entry::Dir(mode) => {
                    let ino = fs.mkdir(dir, name.as_bytes(), *mode, 0, 0).map_err(|e| err(path, e))?;
                    inos.insert(path.clone(), ino);
                }
                Entry::Symlink(t) => {
                    fs.symlink(dir, name.as_bytes(), t.as_bytes(), 0, 0).map_err(|e| err(path, e))?;
                }
                Entry::File { mode, source, mtime } => {
                    let node =
                        NewNode { kind: FileType::Regular, perm: *mode, uid: 0, gid: 0, rdev: DeviceId::new(0, 0) };
                    let ino = fs.create(dir, name.as_bytes(), &node).map_err(|e| err(path, e))?;
                    let data = match source {
                        Source::Host(p) => std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?,
                        Source::Bytes(b) => b.clone(),
                    };
                    for (i, chunk) in data.chunks(1 << 20).enumerate() {
                        fs.write(ino, (i << 20) as u64, chunk).map_err(|e| err(path, e))?;
                    }
                    let at = SetTime::At(Timespec::new(*mtime, 0));
                    let set = SetAttr { mtime: Some(at), atime: Some(at), ..SetAttr::default() };
                    fs.setattr(ino, &set).map_err(|e| err(path, e))?;
                }
            }
        }
        fs.unmount().map_err(|e| err("unmount", e))?;
    }
    std::fs::write(image, &mem).map_err(|e| format!("{}: {e}", image.display()))
}

/// Collect the tree and write it to `target/<name>`; prints what was left
/// out.
pub fn make(name: &str) -> Result<(PathBuf, Contents), String> {
    let contents = collect()?;
    for note in &contents.notes {
        eprintln!("==> note: root disk: {note}");
    }
    let path = crate::root().join("target").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("{}: {e}", path.display()))?;
    let how = write_image(&contents.tree, &path)?;
    let (bytes, entries) = contents.tree.totals();
    eprintln!(
        "==> root disk {}: {} entries, {} MiB of files ({how}); {}",
        path.display(),
        entries,
        bytes >> 20,
        contents.have.iter().copied().collect::<Vec<_>>().join(", ")
    );
    Ok((path, contents))
}

/// The kernel command line of a boot from the root disk.
pub fn boot_args() -> String {
    format!("{} console=ttyS0", crate::busybox::INIT_ARGS)
}

/// Guest memory for boots from the root disk: the compiler and Python
/// need more than the default.
pub const MEMORY: &str = "1G";

/// QEMU arguments attaching `image` as the only virtio-blk device
/// (`virtio0`, `/dev/vda`).
pub fn drive_args(image: &Path) -> Vec<String> {
    vec![
        "-drive".into(),
        format!("if=none,id=root,format=raw,file={}", image.display()),
        "-device".into(),
        "virtio-blk-pci,drive=root,disable-legacy=on".into(),
    ]
}

/// The initramfs of a boot from the root disk (BusyBox's init, see
/// `initramfs::disk_root_image`), written to `target/`; `Err(why)` inside
/// when there is no BusyBox.
pub fn write_initramfs() -> crate::Result<std::result::Result<(PathBuf, String), String>> {
    let binary = match crate::busybox::obtain()? {
        Ok(b) => b,
        Err(why) => return Ok(Err(why)),
    };
    let image = crate::initramfs::disk_root_image(&binary.elf);
    let path = crate::initramfs::write(&crate::root(), "poly-disk-root-initramfs.cpio", &image)?;
    Ok(Ok((path, binary.origin)))
}

/// `cargo xtask root-disk`: write `target/poly-root-disk.img` afresh.
pub fn command() -> crate::Result {
    let (path, _) = make(FILE_NAME)?;
    println!("{}", path.display());
    Ok(())
}
