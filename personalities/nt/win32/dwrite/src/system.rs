//! The system font collection: the fonts of `C:\Windows\Fonts` and of Poly's
//! font store (the DejaVu fonts, ADR 0105, 0181), each also under the
//! Windows family names that stand for it (`poly_dwrite::system`, ADR 0170).

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use poly_dwrite::face::FaceInfo;
use poly_dwrite::model::FaceEntry;
use poly_dwrite::names::{Localized, to_string};
use poly_dwrite::sfnt;
use poly_dwrite::system::{ALIASES, standard_face_name};
use poly_win32_gui::SpinLock;

use crate::coll::CollData;
use crate::file::{FileSource, local_key, local_loader};
use crate::io;

/// The font folders, in the order they are read (ADR 0181): the folder of
/// the Windows layout, then Poly's own font store on the root file system
/// (`Z:` is `/`). A file the first folder has is not taken from the second
/// again, so an image that links the store into `C:\Windows\Fonts` (the root
/// disk) and one that has only the store (the desktop image) give the same
/// collection, and fonts a program or the user put in `C:\Windows\Fonts` win.
const FONT_DIRS: [&str; 2] = ["C:\\Windows\\Fonts", "Z:\\usr\\share\\fonts\\truetype\\dejavu"];

static SYSTEM: SpinLock<Option<Arc<CollData>>> = SpinLock::new(None);

/// The system collection's contents, built on first use and shared by
/// every factory of the process.
pub fn system_data() -> Arc<CollData> {
    if let Some(d) = SYSTEM.with(|s| s.clone()) {
        return d;
    }
    let data = Arc::new(build());
    SYSTEM.with(|s| *s = Some(data.clone()));
    data
}

fn build() -> CollData {
    let mut items: Vec<(FaceEntry, Arc<FileSource>)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (dir, name) in FONT_DIRS.iter().flat_map(|d| io::list_dir(d).into_iter().map(move |n| (*d, n))) {
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".ttf") || lower.ends_with(".otf") || lower.ends_with(".ttc") || lower.ends_with(".otc")) {
            continue;
        }
        if seen.contains(&lower) {
            continue;
        }
        seen.push(lower);
        let path: Vec<u16> = alloc::format!("{dir}\\{name}").encode_utf16().collect();
        let src = FileSource::new(local_loader(), &local_key(&path, io::write_time(&path)));
        let Ok(bytes) = src.bytes() else { continue };
        let a = sfnt::analyze(&bytes);
        if !a.supported {
            continue;
        }
        for idx in 0..a.faces {
            let Some(info) = FaceInfo::parse(&bytes, idx) else { continue };
            let info = Arc::new(info);
            let family: String = info.family.primary().map(to_string).unwrap_or_default();
            let n = items.len();
            items.push((FaceEntry::new(n, idx, info.clone()), src.clone()));
            for (k, alias) in ALIASES.iter().enumerate().filter(|(_, al)| al.target == family) {
                let mut e = FaceEntry::new(n, idx, info.clone());
                e.family = Some(Localized::en_us(alias.name));
                e.face = Some(standard_face_name(info.weight, info.style));
                e.symbol = Some(alias.symbol);
                e.charset = alias.charset;
                e.variant = k as u32 + 1;
                items.push((e.clone(), src.clone()));
                if alias.black && info.weight >= 600 && info.style == 0 {
                    // Windows' Arial family includes Arial Black.
                    e.weight = Some(900);
                    e.face = Some(Localized::en_us("Black"));
                    items.push((e, src.clone()));
                }
            }
        }
    }
    CollData::new(items)
}
