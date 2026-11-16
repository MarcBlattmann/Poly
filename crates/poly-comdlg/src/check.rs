//! What the common dialog functions check before they show anything, and the
//! extended error each failure leaves (`CommDlgExtendedError`).
//!
//! Observed on the oracle (`specs/windows/comdlg32.dll/*.yaml` and their
//! tests): a wrong `lStructSize` is `CDERR_STRUCTSIZE`; a hook flag without a
//! hook is `CDERR_NOHOOK`; a template flag without a template name is
//! `CDERR_FINDRESFAILURE`, a template-handle flag without a handle
//! `CDERR_LOCKRESFAILURE`; `nMaxFile` 0 is `CDERR_INITIALIZATION`;
//! `CF_LIMITSIZE` with a minimum above the maximum is
//! `CFERR_MAXLESSTHANMIN`. A NULL `lpstrFile` is not an error.

use crate::cderr;

/// `sizeof(OPENFILENAMEW)` on x64 and the size without the three trailing
/// members (`OPENFILENAME_SIZE_VERSION_400W`), both accepted.
pub const OFN_SIZE: u32 = 152;
pub const OFN_SIZE_400: u32 = 136;
/// `sizeof(CHOOSECOLORW)` and `sizeof(CHOOSEFONTW)` on x64.
pub const CHOOSECOLOR_SIZE: u32 = 72;
pub const CHOOSEFONT_SIZE: u32 = 104;

/// `OFN_*` (`commdlg.h`).
pub mod ofn {
    pub const READONLY: u32 = 0x1;
    pub const OVERWRITEPROMPT: u32 = 0x2;
    pub const HIDEREADONLY: u32 = 0x4;
    pub const NOCHANGEDIR: u32 = 0x8;
    pub const SHOWHELP: u32 = 0x10;
    pub const ENABLEHOOK: u32 = 0x20;
    pub const ENABLETEMPLATE: u32 = 0x40;
    pub const ENABLETEMPLATEHANDLE: u32 = 0x80;
    pub const NOVALIDATE: u32 = 0x100;
    pub const ALLOWMULTISELECT: u32 = 0x200;
    pub const EXTENSIONDIFFERENT: u32 = 0x400;
    pub const PATHMUSTEXIST: u32 = 0x800;
    pub const FILEMUSTEXIST: u32 = 0x1000;
    pub const CREATEPROMPT: u32 = 0x2000;
    pub const NOREADONLYRETURN: u32 = 0x8000;
    pub const EXPLORER: u32 = 0x80000;
    pub const FORCESHOWHIDDEN: u32 = 0x1000_0000;
}

/// `CC_*` (`commdlg.h`).
pub mod cc {
    pub const RGBINIT: u32 = 0x1;
    pub const FULLOPEN: u32 = 0x2;
    pub const PREVENTFULLOPEN: u32 = 0x4;
    pub const ENABLEHOOK: u32 = 0x10;
    pub const ENABLETEMPLATE: u32 = 0x20;
    pub const ENABLETEMPLATEHANDLE: u32 = 0x40;
    pub const SOLIDCOLOR: u32 = 0x80;
    pub const ANYCOLOR: u32 = 0x100;
}

/// `CF_*` (`commdlg.h`).
pub mod cf {
    pub const SCREENFONTS: u32 = 0x1;
    pub const PRINTERFONTS: u32 = 0x2;
    pub const ENABLEHOOK: u32 = 0x8;
    pub const ENABLETEMPLATE: u32 = 0x10;
    pub const ENABLETEMPLATEHANDLE: u32 = 0x20;
    pub const INITTOLOGFONTSTRUCT: u32 = 0x40;
    pub const USESTYLE: u32 = 0x80;
    pub const EFFECTS: u32 = 0x100;
    pub const LIMITSIZE: u32 = 0x2000;
    pub const FIXEDPITCHONLY: u32 = 0x4000;
    pub const FORCEFONTEXIST: u32 = 0x10000;
    pub const SCALABLEONLY: u32 = 0x20000;
    pub const TTONLY: u32 = 0x40000;
    pub const NOFACESEL: u32 = 0x80000;
    pub const NOSTYLESEL: u32 = 0x100000;
    pub const NOSIZESEL: u32 = 0x200000;
}

/// What the three dialogs' structures say about hooks and templates.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts {
    pub struct_size: u32,
    pub flags: u32,
    pub has_hook: bool,
    pub has_template_name: bool,
    /// The instance handle (which is the template handle with the `*TEMPLATEHANDLE` flags).
    pub has_instance: bool,
}

fn hook_and_template(f: &Facts, hook: u32, template: u32, handle: u32) -> Result<(), u32> {
    if f.flags & hook != 0 && !f.has_hook {
        return Err(cderr::NOHOOK);
    }
    if f.flags & template != 0 && !f.has_template_name {
        return Err(cderr::FINDRESFAILURE);
    }
    if f.flags & handle != 0 && !f.has_instance {
        return Err(cderr::LOCKRESFAILURE);
    }
    Ok(())
}

/// `GetOpenFileNameW` / `GetSaveFileNameW`: the structure checks that come before a dialog.
/// `n_max_file` is `nMaxFile`.
pub fn open_file(f: &Facts, n_max_file: u32) -> Result<(), u32> {
    if f.struct_size != OFN_SIZE && f.struct_size != OFN_SIZE_400 {
        return Err(cderr::STRUCTSIZE);
    }
    hook_and_template(f, ofn::ENABLEHOOK, ofn::ENABLETEMPLATE, ofn::ENABLETEMPLATEHANDLE)?;
    if n_max_file == 0 {
        return Err(cderr::INITIALIZATION);
    }
    Ok(())
}

/// `ChooseColorW`; `has_custom` says whether `lpCustColors` is set.
pub fn choose_color(f: &Facts, has_custom: bool) -> Result<(), u32> {
    if f.struct_size != CHOOSECOLOR_SIZE {
        return Err(cderr::STRUCTSIZE);
    }
    hook_and_template(f, cc::ENABLEHOOK, cc::ENABLETEMPLATE, cc::ENABLETEMPLATEHANDLE)?;
    if !has_custom {
        return Err(cderr::INITIALIZATION);
    }
    Ok(())
}

/// `ChooseFontW`; `has_logfont` says whether `lpLogFont` is set, `min` and `max` are `nSizeMin` and `nSizeMax`.
pub fn choose_font(f: &Facts, has_logfont: bool, min: i32, max: i32) -> Result<(), u32> {
    if f.struct_size != CHOOSEFONT_SIZE {
        return Err(cderr::STRUCTSIZE);
    }
    hook_and_template(f, cf::ENABLEHOOK, cf::ENABLETEMPLATE, cf::ENABLETEMPLATEHANDLE)?;
    if !has_logfont && f.flags & cf::INITTOLOGFONTSTRUCT != 0 {
        return Err(cderr::INITIALIZATION);
    }
    if f.flags & cf::LIMITSIZE != 0 && max < min {
        return Err(cderr::CFERR_MAXLESSTHANMIN);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(size: u32, flags: u32, hook: bool) -> Facts {
        Facts { struct_size: size, flags, has_hook: hook, has_template_name: false, has_instance: false }
    }

    #[test]
    fn open_file_codes() {
        let ok = ofn::ENABLEHOOK | ofn::EXPLORER;
        for size in [0, 4, 1000] {
            assert_eq!(open_file(&facts(size, ok, true), 260), Err(cderr::STRUCTSIZE));
        }
        assert_eq!(open_file(&facts(OFN_SIZE, ok, true), 260), Ok(()));
        assert_eq!(open_file(&facts(OFN_SIZE_400, ok, true), 260), Ok(()));
        assert_eq!(open_file(&facts(OFN_SIZE, ofn::ENABLEHOOK, false), 260), Err(cderr::NOHOOK));
        assert_eq!(open_file(&facts(OFN_SIZE, ofn::ENABLETEMPLATE, false), 260), Err(cderr::FINDRESFAILURE));
        assert_eq!(open_file(&facts(OFN_SIZE, ofn::ENABLETEMPLATEHANDLE, false), 260), Err(cderr::LOCKRESFAILURE));
        assert_eq!(open_file(&facts(OFN_SIZE, ok, true), 0), Err(cderr::INITIALIZATION));
        assert_eq!(open_file(&facts(OFN_SIZE, ok, true), 1), Ok(()));
    }

    #[test]
    fn color_codes() {
        let f = |size, flags, hook| facts(size, flags, hook);
        assert_eq!(choose_color(&f(0, 0, false), true), Err(cderr::STRUCTSIZE));
        assert_eq!(choose_color(&f(36, 0, false), true), Err(cderr::STRUCTSIZE));
        assert_eq!(choose_color(&f(72, cc::ENABLEHOOK, false), true), Err(cderr::NOHOOK));
        assert_eq!(choose_color(&f(72, cc::ENABLETEMPLATE, false), true), Err(cderr::FINDRESFAILURE));
        assert_eq!(choose_color(&f(72, cc::ENABLEHOOK | cc::RGBINIT, true), true), Ok(()));
        assert_eq!(choose_color(&f(72, 0, false), false), Err(cderr::INITIALIZATION));
    }

    #[test]
    fn font_codes() {
        let f = |size, flags, hook| facts(size, flags, hook);
        assert_eq!(choose_font(&f(0, cf::SCREENFONTS, false), true, 0, 0), Err(cderr::STRUCTSIZE));
        assert_eq!(choose_font(&f(104, cf::SCREENFONTS | cf::ENABLEHOOK, false), true, 0, 0), Err(cderr::NOHOOK));
        assert_eq!(
            choose_font(&f(104, cf::SCREENFONTS | cf::LIMITSIZE | cf::ENABLEHOOK, true), true, 20, 10),
            Err(cderr::CFERR_MAXLESSTHANMIN)
        );
        // No font type flag at all is accepted.
        assert_eq!(choose_font(&f(104, cf::ENABLEHOOK, true), true, 0, 0), Ok(()));
        assert_eq!(choose_font(&f(104, cf::SCREENFONTS | cf::LIMITSIZE, false), true, 10, 10), Ok(()));
    }
}
