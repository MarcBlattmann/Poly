use super::*;
use alloc::collections::BTreeMap;

// ---- a squashfs 4 writer for the tests -------------------------------------
//
// Uncompressed, one metadata block for inodes and one for directories, no
// fragments (the format's public description; the same image `mksquashfs
// -noI -noD -noF -no-fragments` makes).

#[derive(Clone)]
enum Node {
    Dir(BTreeMap<String, Node>),
    File(Vec<u8>),
    Link(String),
}

fn tree(entries: &[(&str, Node)]) -> Node {
    let mut root = BTreeMap::new();
    for (path, node) in entries {
        let parts: Vec<&str> = path.split('/').collect();
        let mut dir = &mut root;
        for p in &parts[..parts.len() - 1] {
            let Node::Dir(d) = dir.entry(p.to_string()).or_insert_with(|| Node::Dir(BTreeMap::new())) else {
                panic!("not a directory")
            };
            dir = d;
        }
        dir.insert(parts[parts.len() - 1].to_string(), node.clone());
    }
    Node::Dir(root)
}

struct Placed {
    ino: u32,
    /// Where the inode starts in the inode table.
    at: usize,
    size: usize,
}

struct Writer {
    inodes: Vec<u8>,
    dirs: Vec<u8>,
    data: Vec<u8>,
    next: u32,
}

fn inode_size(n: &Node) -> usize {
    match n {
        Node::Dir(_) => 32,
        Node::File(d) => 32 + 4 * d.len().div_ceil(4096),
        Node::Link(t) => 24 + t.len(),
    }
}

impl Writer {
    /// Write `n` (and below it) with parent inode `parent`; returns its
    /// place. Children are written first so a directory knows them.
    fn node(&mut self, n: &Node, parent: u32) -> Placed {
        // Children's inodes come before the directory's own: allocate this
        // directory's number first, write the children, then the listing.
        let ino = self.next;
        self.next += 1;
        match n {
            Node::File(d) => {
                let at = self.inodes.len();
                let start = 96 + self.data.len() as u32;
                let blocks = d.len().div_ceil(4096);
                let mut h = Vec::new();
                h.extend(2u16.to_le_bytes());
                h.extend(0o644u16.to_le_bytes());
                h.extend([0, 0, 0, 0]);
                h.extend(0u32.to_le_bytes());
                h.extend(ino.to_le_bytes());
                h.extend(start.to_le_bytes());
                h.extend(0xFFFF_FFFFu32.to_le_bytes());
                h.extend(0u32.to_le_bytes());
                h.extend((d.len() as u32).to_le_bytes());
                for b in 0..blocks {
                    let part = &d[b * 4096..d.len().min((b + 1) * 4096)];
                    h.extend((part.len() as u32 | 1 << 24).to_le_bytes());
                    self.data.extend(part);
                }
                self.inodes.extend(h);
                Placed { ino, at, size: inode_size(n) }
            }
            Node::Link(t) => {
                let at = self.inodes.len();
                let mut h = Vec::new();
                h.extend(3u16.to_le_bytes());
                h.extend(0o777u16.to_le_bytes());
                h.extend([0, 0, 0, 0]);
                h.extend(0u32.to_le_bytes());
                h.extend(ino.to_le_bytes());
                h.extend(1u32.to_le_bytes());
                h.extend((t.len() as u32).to_le_bytes());
                h.extend(t.as_bytes());
                self.inodes.extend(h);
                Placed { ino, at, size: inode_size(n) }
            }
            Node::Dir(children) => {
                let placed: Vec<(&String, &Node, Placed)> =
                    children.iter().map(|(name, child)| (name, child, self.node(child, ino))).collect();
                // The listing.
                let base = placed.first().map_or(0, |(_, _, p)| p.ino);
                let start = self.dirs.len();
                let mut l = Vec::new();
                if !placed.is_empty() {
                    l.extend((placed.len() as u32 - 1).to_le_bytes());
                    l.extend(0u32.to_le_bytes());
                    l.extend(base.to_le_bytes());
                    for (name, child, p) in &placed {
                        let kind: u16 = match child {
                            Node::Dir(_) => 1,
                            Node::File(_) => 2,
                            Node::Link(_) => 3,
                        };
                        l.extend((p.at as u16).to_le_bytes());
                        l.extend(((p.ino as i32 - base as i32) as i16).to_le_bytes());
                        l.extend(kind.to_le_bytes());
                        l.extend((name.len() as u16 - 1).to_le_bytes());
                        l.extend(name.as_bytes());
                    }
                }
                let len = l.len();
                self.dirs.extend(l);
                let at = self.inodes.len();
                let mut h = Vec::new();
                h.extend(1u16.to_le_bytes());
                h.extend(0o755u16.to_le_bytes());
                h.extend([0, 0, 0, 0]);
                h.extend(0u32.to_le_bytes());
                h.extend(ino.to_le_bytes());
                h.extend(0u32.to_le_bytes());
                h.extend(2u32.to_le_bytes());
                h.extend((len as u16 + 3).to_le_bytes());
                h.extend((start as u16).to_le_bytes());
                h.extend(parent.to_le_bytes());
                self.inodes.extend(h);
                Placed { ino, at, size: inode_size(n) }
            }
        }
    }
}

/// The squashfs image of `root`.
fn squashfs(root: &Node) -> Vec<u8> {
    let mut w = Writer { inodes: Vec::new(), dirs: Vec::new(), data: Vec::new(), next: 1 };
    // The root's own number is the last allocated one, as mksquashfs does.
    let placed = w.node(root, 0);
    let count = w.next - 1;
    assert!(w.inodes.len() < 8000 && w.dirs.len() < 8000, "one metadata block each");
    let mut img = alloc::vec![0u8; 96];
    img.extend(&w.data);
    let inode_table = img.len() as u64;
    img.extend((w.inodes.len() as u16 | 0x8000).to_le_bytes());
    img.extend(&w.inodes);
    let dir_table = img.len() as u64;
    img.extend((w.dirs.len() as u16 | 0x8000).to_le_bytes());
    img.extend(&w.dirs);
    let ids = img.len() as u64;
    img.extend((4u16 | 0x8000).to_le_bytes());
    img.extend(0u32.to_le_bytes());
    let id_table = img.len() as u64;
    img.extend(ids.to_le_bytes());
    let used = img.len() as u64;
    let _ = placed.size;
    let mut sb = Vec::new();
    sb.extend(0x7371_7368u32.to_le_bytes());
    sb.extend(count.to_le_bytes());
    sb.extend(0u32.to_le_bytes());
    sb.extend(4096u32.to_le_bytes());
    sb.extend(0u32.to_le_bytes());
    sb.extend(1u16.to_le_bytes());
    sb.extend(12u16.to_le_bytes());
    // Uncompressed inodes, data, no fragments, no xattrs, uncompressed ids.
    sb.extend((0x1u16 | 0x2 | 0x10 | 0x200 | 0x800).to_le_bytes());
    sb.extend(1u16.to_le_bytes());
    sb.extend(4u16.to_le_bytes());
    sb.extend(0u16.to_le_bytes());
    sb.extend((placed.at as u64).to_le_bytes());
    sb.extend(used.to_le_bytes());
    sb.extend(id_table.to_le_bytes());
    sb.extend(u64::MAX.to_le_bytes());
    sb.extend(inode_table.to_le_bytes());
    sb.extend(dir_table.to_le_bytes());
    sb.extend(u64::MAX.to_le_bytes());
    sb.extend(u64::MAX.to_le_bytes());
    img[..96].copy_from_slice(&sb);
    img
}

/// A type 2 runtime: an ELF header that says where its section headers end,
/// padded to that place.
fn runtime() -> Vec<u8> {
    let mut e = alloc::vec![0u8; 0x200];
    e[..4].copy_from_slice(b"\x7fELF");
    e[4] = 2;
    e[5] = 1;
    e[8..11].copy_from_slice(b"AI\x02");
    // Section headers at 0x100: four of 64 bytes end at 0x200.
    e[0x28..0x30].copy_from_slice(&0x100u64.to_le_bytes());
    e[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
    e[0x3c..0x3e].copy_from_slice(&4u16.to_le_bytes());
    e
}

fn png() -> Vec<u8> {
    let mut p = alloc::vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    p.extend(b"icon data");
    p
}

const DESKTOP: &str = "[Desktop Entry]\nType=Application\nName=Hello Pixels\nName[de]=Hallo\nComment=Draws a few pixels\n\
Exec=AppRun %U\nIcon=hello\nCategories=Graphics;\nX-AppImage-Version=1.2.3\n";

fn appimage(entries: &[(&str, Node)]) -> Vec<u8> {
    let mut file = runtime();
    file.extend(squashfs(&tree(entries)));
    // An AppImage ends in the image, padded as a device would.
    file.resize(file.len().div_ceil(4096) * 4096, 0);
    file
}

fn read_image(file: &[u8]) -> Result<Image, Error> {
    let owned = file.to_vec();
    let head = owned[..HEAD.min(owned.len())].to_vec();
    read(&head, owned.len() as u64, move |at, buf| match owned.get(at as usize..at as usize + buf.len()) {
        Some(src) => {
            buf.copy_from_slice(src);
            true
        }
        None => false,
    })
}

fn file(d: &[u8]) -> Node {
    Node::File(d.to_vec())
}

#[test]
fn the_squashfs_starts_where_the_elf_ends() {
    assert_eq!(squashfs_offset(&runtime()), Ok(0x200));
    // A 32-bit runtime.
    let mut e = alloc::vec![0u8; 64];
    e[..4].copy_from_slice(b"\x7fELF");
    e[4] = 1;
    e[5] = 1;
    e[8..11].copy_from_slice(b"AI\x02");
    e[0x20..0x24].copy_from_slice(&0x1000u32.to_le_bytes());
    e[0x2e..0x30].copy_from_slice(&40u16.to_le_bytes());
    e[0x30..0x32].copy_from_slice(&10u16.to_le_bytes());
    assert_eq!(squashfs_offset(&e), Ok(0x1000 + 400));
    // Type 1 is an ISO image; anything else is no AppImage.
    let mut old = runtime();
    old[10] = 1;
    assert_eq!(squashfs_offset(&old), Err(Error::OldAppImage));
    assert_eq!(squashfs_offset(b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00"), Err(Error::NotThisFormat));
    assert_eq!(squashfs_offset(b"not an elf file at all, but long enough to read"), Err(Error::NotThisFormat));
    assert_eq!(squashfs_offset(&runtime()[..0x30]), Err(Error::Truncated));
}

#[test]
fn the_desktop_file_and_the_icon_come_out_of_the_image() {
    let img = appimage(&[
        ("hello.desktop", file(DESKTOP.as_bytes())),
        (".DirIcon", Node::Link("hello.png".into())),
        ("hello.png", file(&png())),
        ("AppRun", file(b"#!/bin/sh\n")),
        ("usr/bin/hello", file(&alloc::vec![7u8; 9000])),
    ]);
    let image = read_image(&img).unwrap();
    assert_eq!(image.name, "Hello Pixels");
    assert_eq!(image.summary, "Draws a few pixels");
    assert_eq!(image.version, "1.2.3");
    assert_eq!(image.icon.as_deref(), Some(png().as_slice()));
    assert_eq!(image.size, img.len() as u64);
    assert!(image.desktop.contains("Exec=AppRun"));
}

#[test]
fn the_icon_is_found_by_its_name_or_in_the_icon_theme() {
    // No .DirIcon: `<Icon>.png` at the top.
    let img = appimage(&[("hello.desktop", file(DESKTOP.as_bytes())), ("hello.png", file(&png()))]);
    assert_eq!(read_image(&img).unwrap().icon, Some(png()));
    // Only in the icon theme: the biggest PNG; a text file named .png is no icon.
    let big = [png(), b"big".to_vec()].concat();
    let img = appimage(&[
        ("hello.desktop", file(DESKTOP.as_bytes())),
        ("usr/share/icons/hicolor/48x48/apps/hello.png", file(&png())),
        ("usr/share/icons/hicolor/256x256/apps/hello.png", file(&big)),
        ("usr/share/icons/hicolor/scalable/apps/hello.svg", file(b"<svg/>")),
    ]);
    assert_eq!(read_image(&img).unwrap().icon, Some(big));
    let img = appimage(&[("hello.desktop", file(DESKTOP.as_bytes())), ("hello.png", file(b"not a png"))]);
    assert_eq!(read_image(&img).unwrap().icon, None);
    // A link out of the image's root or in a loop finds nothing.
    let img = appimage(&[
        ("hello.desktop", file(DESKTOP.as_bytes())),
        (".DirIcon", Node::Link("loop".into())),
        ("loop", Node::Link(".DirIcon".into())),
    ]);
    assert_eq!(read_image(&img).unwrap().icon, None);
    // An absolute link is relative to the image's root, a `..` stays inside.
    let img = appimage(&[
        ("hello.desktop", file(DESKTOP.as_bytes())),
        (".DirIcon", Node::Link("/usr/share/pixmaps/../pixmaps/h.png".into())),
        ("usr/share/pixmaps/h.png", file(&png())),
    ]);
    assert_eq!(read_image(&img).unwrap().icon, Some(png()));
}

#[test]
fn an_image_without_a_desktop_file_or_not_readable_is_refused() {
    let img = appimage(&[("AppRun", file(b"x")), ("sub/a.desktop", file(DESKTOP.as_bytes()))]);
    assert_eq!(read_image(&img), Err(Error::NoDesktopEntry));
    let img = appimage(&[("a.desktop", file(b"[Desktop Entry]\nType=Application\nExec=a\n"))]);
    assert_eq!(read_image(&img), Err(Error::NoDesktopEntry));
    // Cut off inside the runtime, or with no image behind it.
    let img = appimage(&[("a.desktop", file(DESKTOP.as_bytes()))]);
    assert_eq!(read_image(&img[..0x100]), Err(Error::Truncated));
    assert_eq!(read_image(&runtime()), Err(Error::Truncated));
    let mut bad = img.clone();
    bad[0x200] = b'x';
    assert_eq!(read_image(&bad), Err(Error::Corrupt));
}

#[test]
fn names_become_slugs_and_slugs_become_paths() {
    assert_eq!(slug("Hello Pixels"), "hello-pixels");
    assert_eq!(slug("  --Qt/Creator 5.0!  "), "qt-creator-5-0");
    assert_eq!(slug("日本語"), "app");
    assert!(slug(&"abc ".repeat(30)).len() <= 40);
    for s in ["hello-pixels", "a", "x1"] {
        assert!(is_slug(s));
        assert_eq!(slug_of_id(&id_of(s)), Some(s));
    }
    for s in ["", "-a", "A", "a/b", "a b", "../x", "a.b"] {
        assert!(!is_slug(s), "{s}");
    }
    assert_eq!(slug_of_id("poly-appimage-../x.desktop"), None);
    assert_eq!(slug_of_id("galculator.desktop"), None);
    let l = layout("/home/poly/.local/share/", "hello-pixels");
    assert_eq!(l.dir, "/home/poly/.local/share/poly/appimages/hello-pixels");
    assert_eq!(l.image, "/home/poly/.local/share/poly/appimages/hello-pixels/hello-pixels.AppImage");
    assert_eq!(l.icon, "/home/poly/.local/share/poly/appimages/hello-pixels/icon.png");
    assert_eq!(l.desktop, "/home/poly/.local/share/applications/poly-appimage-hello-pixels.desktop");
    assert_eq!(l.id, "poly-appimage-hello-pixels.desktop");
    assert_eq!(applications_dir("/home/poly/.local/share/"), "/home/poly/.local/share/applications");
}

#[test]
fn the_installed_desktop_file_names_the_copy_and_the_icon() {
    let img = appimage(&[("hello.desktop", file(DESKTOP.as_bytes())), ("hello.png", file(&png()))]);
    let image = read_image(&img).unwrap();
    let l = layout("/home/poly/.local/share", &slug(&image.name));
    let text = desktop_file(&image, &l, false);
    assert_eq!(
        text,
        "[Desktop Entry]\nType=Application\nName=Hello Pixels\nComment=Draws a few pixels\n\
Exec=/home/poly/.local/share/poly/appimages/hello-pixels/hello-pixels.AppImage\n\
Icon=/home/poly/.local/share/poly/appimages/hello-pixels/icon.png\nCategories=Graphics;\n\
X-Poly-AppImage=/home/poly/.local/share/poly/appimages/hello-pixels/hello-pixels.AppImage\n"
    );
    assert_eq!(image_of_desktop(&text).as_deref(), Some(l.image.as_str()));
    // The same, for a system where the image cannot be mounted.
    assert!(desktop_file(&image, &l, true).contains(&alloc::format!("Exec=env {EXTRACT_AND_RUN}=1 /home/")));
    assert_eq!(run_environment(true), [(EXTRACT_AND_RUN, "1")]);
    assert!(run_environment(false).is_empty());
    // The launcher reads it back as the app it is.
    assert_eq!(image_of_desktop("[Desktop Entry]\nName=x\nExec=y\n"), None);
    assert_eq!(image_of_desktop("[Desktop Entry]\nX-Poly-AppImage=relative.AppImage\n"), None);
}

#[test]
fn an_exec_path_with_spaces_or_symbols_is_quoted() {
    assert_eq!(exec_arg("/a/b"), "/a/b");
    assert_eq!(exec_arg("/a b/c"), "\"/a b/c\"");
    assert_eq!(exec_arg("/a/$x\"y"), "\"/a/\\$x\\\"y\"");
    let image = Image { name: "Odd\nName".into(), summary: "Two\nlines".into(), ..Image::default() };
    let l = layout("/h/my data", "odd-name");
    let text = desktop_file(&image, &l, false);
    assert!(
        text.contains(
            "Name=Odd Name\nComment=Two lines\nExec=\"/h/my data/poly/appimages/odd-name/odd-name.AppImage\"\n"
        )
    );
    assert!(!text.contains("Icon="));
}

#[test]
fn desktop_keys_skip_translations_and_other_groups() {
    let keys = desktop_keys("# c\n[Desktop Entry]\nName=A\nName[de]=B\n\n[Desktop Action x]\nName=C\n");
    assert_eq!(keys, [("Name", "A")]);
}
