//! The system font collection: the fonts of `C:\Windows\Fonts` (Poly puts
//! the DejaVu fonts there, ADR 0105), each also under the Windows family
//! names that stand for it (`poly_dwrite::system`, ADR 0170).

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

const FONTS_DIR: &str = "C:\\Windows\\Fonts";

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
    for name in io::list_dir(FONTS_DIR) {
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".ttf") || lower.ends_with(".otf") || lower.ends_with(".ttc") || lower.ends_with(".otc")) {
            continue;
        }
        let path: Vec<u16> = alloc::format!("{FONTS_DIR}\\{name}").encode_utf16().collect();
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
                items.push((e, src.clone()));
            }
        }
    }
    CollData::new(items)
}
