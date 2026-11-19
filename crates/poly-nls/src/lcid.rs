//! Locale names and locale identifiers (LCIDs): the table behind `LCIDToLocaleName`,
//! `LocaleNameToLCID`, `EnumSystemLocalesW`/`Ex` and the locale argument of the date and time
//! formatting functions.
//!
//! Sources (clean room): the Open Specification [MS-LCID] "Windows Language Code Identifier
//! (LCID) Reference" (the identifiers and names), Learn's "Locale Names", "Locale Identifiers",
//! "Sort Order Identifiers" and the `winnls.h`/`winnt.h` constants, and the spec sheets
//! `specs/windows/kernelbase.dll/{LCIDToLocaleName,LocaleNameToLCID,EnumSystemLocalesEx}.yaml`
//! (which names map to which identifiers, which names are neutral, the defaults of the neutral
//! locales, the alternate sorts, the grammar of a well-formed name).
//!
//! The table is Poly's: the locales [MS-LCID] gives a classic identifier for (the specific
//! ones, the neutral ones with the specific locale each stands for, the alternate sort orders),
//! the pseudo-locales, and a few script-tagged spellings of specific names.

/// `LOCALE_INVARIANT` (`winnt.h`): the invariant locale, whose name is the empty string.
pub const LOCALE_INVARIANT: u32 = 0x007F;
/// `LOCALE_CUSTOM_UNSPECIFIED` (`winnt.h`): the identifier of a locale that has a name but no
/// classic identifier.
pub const LOCALE_CUSTOM_UNSPECIFIED: u32 = 0x1000;
/// `LOCALE_ALLOW_NEUTRAL_NAMES` (`winnls.h`).
pub const LOCALE_ALLOW_NEUTRAL_NAMES: u32 = 0x0800_0000;
/// `LOCALE_NAME_MAX_LENGTH` (`winnls.h`): 85 units, the NUL included.
pub const LOCALE_NAME_MAX_LENGTH: usize = 85;

/// What a locale of the table is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A specific locale (a language in a country or region).
    Specific,
    /// A neutral locale (a language, or a language in a script); the invariant locale is one.
    Neutral,
    /// An alternate sort order of a locale (`de-DE_phoneb`).
    AltSort,
    /// A pseudo-locale (`qps-ploc`): known, never enumerated.
    Pseudo,
    /// Another spelling of a specific locale's name (`zh-Hans-CN`): known by name only.
    Alias,
}

/// One locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: &'static str,
    pub lcid: u32,
    pub kind: Kind,
    /// The specific locale whose formats it uses: a neutral locale's default (what
    /// `LocaleNameToLCID` without `LOCALE_ALLOW_NEUTRAL_NAMES` gives for it), an alternate
    /// sort's own locale, an alias's locale; the locale itself otherwise.
    pub base: u32,
}

const fn s(name: &'static str, lcid: u32) -> Entry {
    Entry { name, lcid, kind: Kind::Specific, base: lcid }
}
const fn n(name: &'static str, lcid: u32, base: u32) -> Entry {
    Entry { name, lcid, kind: Kind::Neutral, base }
}
const fn a(name: &'static str, lcid: u32, base: u32) -> Entry {
    Entry { name, lcid, kind: Kind::AltSort, base }
}
const fn p(name: &'static str, lcid: u32) -> Entry {
    Entry { name, lcid, kind: Kind::Pseudo, base: lcid }
}
const fn alias(name: &'static str, lcid: u32) -> Entry {
    Entry { name, lcid, kind: Kind::Alias, base: lcid }
}

/// Every locale Poly knows ([MS-LCID]).
pub static ENTRIES: &[Entry] = &[
    // The invariant locale: neutral, its own default.
    n("", LOCALE_INVARIANT, LOCALE_INVARIANT),
    // Neutral locales and the specific locale each stands for.
    n("af", 0x0036, 0x0436),
    n("am", 0x005E, 0x045E),
    n("ar", 0x0001, 0x0401),
    n("arn", 0x007A, 0x047A),
    n("as", 0x004D, 0x044D),
    n("az", 0x002C, 0x042C),
    n("az-Cyrl", 0x742C, 0x082C),
    n("az-Latn", 0x782C, 0x042C),
    n("ba", 0x006D, 0x046D),
    n("be", 0x0023, 0x0423),
    n("bg", 0x0002, 0x0402),
    n("bn", 0x0045, 0x0845),
    n("bo", 0x0051, 0x0451),
    n("br", 0x007E, 0x047E),
    n("bs", 0x781A, 0x141A),
    n("bs-Cyrl", 0x641A, 0x201A),
    n("bs-Latn", 0x681A, 0x141A),
    n("ca", 0x0003, 0x0403),
    n("chr", 0x005C, 0x045C),
    n("chr-Cher", 0x7C5C, 0x045C),
    n("co", 0x0083, 0x0483),
    n("cs", 0x0005, 0x0405),
    n("cy", 0x0052, 0x0452),
    n("da", 0x0006, 0x0406),
    n("de", 0x0007, 0x0407),
    n("dsb", 0x7C2E, 0x082E),
    n("dv", 0x0065, 0x0465),
    n("el", 0x0008, 0x0408),
    n("en", 0x0009, 0x0409),
    n("es", 0x000A, 0x0C0A),
    n("et", 0x0025, 0x0425),
    n("eu", 0x002D, 0x042D),
    n("fa", 0x0029, 0x0429),
    n("ff", 0x0067, 0x0867),
    n("ff-Latn", 0x7C67, 0x0867),
    n("fi", 0x000B, 0x040B),
    n("fil", 0x0064, 0x0464),
    n("fo", 0x0038, 0x0438),
    n("fr", 0x000C, 0x040C),
    n("fy", 0x0062, 0x0462),
    n("ga", 0x003C, 0x083C),
    n("gd", 0x0091, 0x0491),
    n("gl", 0x0056, 0x0456),
    n("gn", 0x0074, 0x0474),
    n("gsw", 0x0084, 0x0484),
    n("gu", 0x0047, 0x0447),
    n("ha", 0x0068, 0x0468),
    n("ha-Latn", 0x7C68, 0x0468),
    n("haw", 0x0075, 0x0475),
    n("he", 0x000D, 0x040D),
    n("hi", 0x0039, 0x0439),
    n("hr", 0x001A, 0x041A),
    n("hsb", 0x002E, 0x042E),
    n("hu", 0x000E, 0x040E),
    n("hy", 0x002B, 0x042B),
    n("id", 0x0021, 0x0421),
    n("ig", 0x0070, 0x0470),
    n("ii", 0x0078, 0x0478),
    n("is", 0x000F, 0x040F),
    n("it", 0x0010, 0x0410),
    n("iu", 0x005D, 0x085D),
    n("iu-Cans", 0x785D, 0x045D),
    n("iu-Latn", 0x7C5D, 0x085D),
    n("ja", 0x0011, 0x0411),
    n("ka", 0x0037, 0x0437),
    n("kk", 0x003F, 0x043F),
    n("kl", 0x006F, 0x046F),
    n("km", 0x0053, 0x0453),
    n("kn", 0x004B, 0x044B),
    n("ko", 0x0012, 0x0412),
    n("kok", 0x0057, 0x0457),
    n("ku", 0x0092, 0x0492),
    n("ku-Arab", 0x7C92, 0x0492),
    n("ky", 0x0040, 0x0440),
    n("lb", 0x006E, 0x046E),
    n("lo", 0x0054, 0x0454),
    n("lt", 0x0027, 0x0427),
    n("lv", 0x0026, 0x0426),
    n("mi", 0x0081, 0x0481),
    n("mk", 0x002F, 0x042F),
    n("ml", 0x004C, 0x044C),
    n("mn", 0x0050, 0x0450),
    n("mn-Cyrl", 0x7850, 0x0450),
    n("mn-Mong", 0x7C50, 0x0850),
    n("moh", 0x007C, 0x047C),
    n("mr", 0x004E, 0x044E),
    n("ms", 0x003E, 0x043E),
    n("mt", 0x003A, 0x043A),
    n("my", 0x0055, 0x0455),
    n("nb", 0x7C14, 0x0414),
    n("ne", 0x0061, 0x0461),
    n("nl", 0x0013, 0x0413),
    n("nn", 0x7814, 0x0814),
    n("no", 0x0014, 0x0414),
    n("nso", 0x006C, 0x046C),
    n("oc", 0x0082, 0x0482),
    n("om", 0x0072, 0x0472),
    n("or", 0x0048, 0x0448),
    n("pa", 0x0046, 0x0446),
    n("pa-Arab", 0x7C46, 0x0846),
    n("pl", 0x0015, 0x0415),
    n("prs", 0x008C, 0x048C),
    n("ps", 0x0063, 0x0463),
    n("pt", 0x0016, 0x0416),
    n("quc", 0x0086, 0x0486),
    n("quz", 0x006B, 0x046B),
    n("rm", 0x0017, 0x0417),
    n("ro", 0x0018, 0x0418),
    n("ru", 0x0019, 0x0419),
    n("rw", 0x0087, 0x0487),
    n("sa", 0x004F, 0x044F),
    n("sah", 0x0085, 0x0485),
    n("sd", 0x0059, 0x0859),
    n("sd-Arab", 0x7C59, 0x0859),
    n("se", 0x003B, 0x043B),
    n("si", 0x005B, 0x045B),
    n("sk", 0x001B, 0x041B),
    n("sl", 0x0024, 0x0424),
    n("sma", 0x783B, 0x183B),
    n("smj", 0x7C3B, 0x103B),
    n("smn", 0x703B, 0x243B),
    n("sms", 0x743B, 0x203B),
    n("so", 0x0077, 0x0477),
    n("sq", 0x001C, 0x041C),
    n("sr", 0x7C1A, 0x241A),
    n("sr-Cyrl", 0x6C1A, 0x281A),
    n("sr-Latn", 0x701A, 0x241A),
    n("st", 0x0030, 0x0430),
    n("sv", 0x001D, 0x041D),
    n("sw", 0x0041, 0x0441),
    n("syr", 0x005A, 0x045A),
    n("ta", 0x0049, 0x0449),
    n("te", 0x004A, 0x044A),
    n("tg", 0x0028, 0x0428),
    n("tg-Cyrl", 0x7C28, 0x0428),
    n("th", 0x001E, 0x041E),
    n("ti", 0x0073, 0x0473),
    n("tk", 0x0042, 0x0442),
    n("tn", 0x0032, 0x0432),
    n("tr", 0x001F, 0x041F),
    n("ts", 0x0031, 0x0431),
    n("tt", 0x0044, 0x0444),
    n("tzm", 0x005F, 0x085F),
    n("tzm-Latn", 0x7C5F, 0x085F),
    n("ug", 0x0080, 0x0480),
    n("uk", 0x0022, 0x0422),
    n("ur", 0x0020, 0x0420),
    n("uz", 0x0043, 0x0443),
    n("uz-Cyrl", 0x7843, 0x0843),
    n("uz-Latn", 0x7C43, 0x0443),
    n("ve", 0x0033, 0x0433),
    n("vi", 0x002A, 0x042A),
    n("wo", 0x0088, 0x0488),
    n("xh", 0x0034, 0x0434),
    n("yo", 0x006A, 0x046A),
    n("zh", 0x7804, 0x0804),
    n("zh-Hans", 0x0004, 0x0804),
    n("zh-Hant", 0x7C04, 0x0C04),
    n("zu", 0x0035, 0x0435),
    // Specific locales.
    s("af-ZA", 0x0436),
    s("am-ET", 0x045E),
    s("ar-AE", 0x3801),
    s("ar-BH", 0x3C01),
    s("ar-DZ", 0x1401),
    s("ar-EG", 0x0C01),
    s("ar-IQ", 0x0801),
    s("ar-JO", 0x2C01),
    s("ar-KW", 0x3401),
    s("ar-LB", 0x3001),
    s("ar-LY", 0x1001),
    s("ar-MA", 0x1801),
    s("ar-OM", 0x2001),
    s("ar-QA", 0x4001),
    s("ar-SA", 0x0401),
    s("ar-SY", 0x2801),
    s("ar-TN", 0x1C01),
    s("ar-YE", 0x2401),
    s("arn-CL", 0x047A),
    s("as-IN", 0x044D),
    s("az-Cyrl-AZ", 0x082C),
    s("az-Latn-AZ", 0x042C),
    s("ba-RU", 0x046D),
    s("be-BY", 0x0423),
    s("bg-BG", 0x0402),
    s("bn-BD", 0x0845),
    s("bn-IN", 0x0445),
    s("bo-CN", 0x0451),
    s("br-FR", 0x047E),
    s("bs-Cyrl-BA", 0x201A),
    s("bs-Latn-BA", 0x141A),
    s("ca-ES", 0x0403),
    s("ca-ES-valencia", 0x0803),
    s("chr-Cher-US", 0x045C),
    s("co-FR", 0x0483),
    s("cs-CZ", 0x0405),
    s("cy-GB", 0x0452),
    s("da-DK", 0x0406),
    s("de-AT", 0x0C07),
    s("de-CH", 0x0807),
    s("de-DE", 0x0407),
    s("de-LI", 0x1407),
    s("de-LU", 0x1007),
    s("dsb-DE", 0x082E),
    s("dv-MV", 0x0465),
    s("dz-BT", 0x0C51),
    s("el-GR", 0x0408),
    s("en-029", 0x2409),
    s("en-AE", 0x4C09),
    s("en-AU", 0x0C09),
    s("en-BZ", 0x2809),
    s("en-CA", 0x1009),
    s("en-GB", 0x0809),
    s("en-HK", 0x3C09),
    s("en-IE", 0x1809),
    s("en-IN", 0x4009),
    s("en-JM", 0x2009),
    s("en-MY", 0x4409),
    s("en-NZ", 0x1409),
    s("en-PH", 0x3409),
    s("en-SG", 0x4809),
    s("en-TT", 0x2C09),
    s("en-US", 0x0409),
    s("en-ZA", 0x1C09),
    s("en-ZW", 0x3009),
    s("es-419", 0x580A),
    s("es-AR", 0x2C0A),
    s("es-BO", 0x400A),
    s("es-CL", 0x340A),
    s("es-CO", 0x240A),
    s("es-CR", 0x140A),
    s("es-CU", 0x5C0A),
    s("es-DO", 0x1C0A),
    s("es-EC", 0x300A),
    s("es-ES", 0x0C0A),
    s("es-GT", 0x100A),
    s("es-HN", 0x480A),
    s("es-MX", 0x080A),
    s("es-NI", 0x4C0A),
    s("es-PA", 0x180A),
    s("es-PE", 0x280A),
    s("es-PR", 0x500A),
    s("es-PY", 0x3C0A),
    s("es-SV", 0x440A),
    s("es-US", 0x540A),
    s("es-UY", 0x380A),
    s("es-VE", 0x200A),
    s("et-EE", 0x0425),
    s("eu-ES", 0x042D),
    s("fa-IR", 0x0429),
    s("ff-Latn-SN", 0x0867),
    s("fi-FI", 0x040B),
    s("fil-PH", 0x0464),
    s("fo-FO", 0x0438),
    s("fr-029", 0x1C0C),
    s("fr-BE", 0x080C),
    s("fr-CA", 0x0C0C),
    s("fr-CD", 0x240C),
    s("fr-CH", 0x100C),
    s("fr-CI", 0x300C),
    s("fr-CM", 0x2C0C),
    s("fr-FR", 0x040C),
    s("fr-HT", 0x3C0C),
    s("fr-LU", 0x140C),
    s("fr-MA", 0x380C),
    s("fr-MC", 0x180C),
    s("fr-ML", 0x340C),
    s("fr-RE", 0x200C),
    s("fr-SN", 0x280C),
    s("fy-NL", 0x0462),
    s("ga-IE", 0x083C),
    s("gd-GB", 0x0491),
    s("gl-ES", 0x0456),
    s("gn-PY", 0x0474),
    s("gsw-FR", 0x0484),
    s("gu-IN", 0x0447),
    s("ha-Latn-NG", 0x0468),
    s("haw-US", 0x0475),
    s("he-IL", 0x040D),
    s("hi-IN", 0x0439),
    s("hr-BA", 0x101A),
    s("hr-HR", 0x041A),
    s("hsb-DE", 0x042E),
    s("hu-HU", 0x040E),
    s("hy-AM", 0x042B),
    s("id-ID", 0x0421),
    s("ig-NG", 0x0470),
    s("ii-CN", 0x0478),
    s("is-IS", 0x040F),
    s("it-CH", 0x0810),
    s("it-IT", 0x0410),
    s("iu-Cans-CA", 0x045D),
    s("iu-Latn-CA", 0x085D),
    s("ja-JP", 0x0411),
    s("ka-GE", 0x0437),
    s("kk-KZ", 0x043F),
    s("kl-GL", 0x046F),
    s("km-KH", 0x0453),
    s("kn-IN", 0x044B),
    s("ko-KR", 0x0412),
    s("kok-IN", 0x0457),
    s("ku-Arab-IQ", 0x0492),
    s("ky-KG", 0x0440),
    s("lb-LU", 0x046E),
    s("lo-LA", 0x0454),
    s("lt-LT", 0x0427),
    s("lv-LV", 0x0426),
    s("mi-NZ", 0x0481),
    s("mk-MK", 0x042F),
    s("ml-IN", 0x044C),
    s("mn-MN", 0x0450),
    s("mn-Mong-CN", 0x0850),
    s("moh-CA", 0x047C),
    s("mr-IN", 0x044E),
    s("ms-BN", 0x083E),
    s("ms-MY", 0x043E),
    s("mt-MT", 0x043A),
    s("my-MM", 0x0455),
    s("nb-NO", 0x0414),
    s("ne-NP", 0x0461),
    s("nl-BE", 0x0813),
    s("nl-NL", 0x0413),
    s("nn-NO", 0x0814),
    s("nso-ZA", 0x046C),
    s("oc-FR", 0x0482),
    s("om-ET", 0x0472),
    s("or-IN", 0x0448),
    s("pa-Arab-PK", 0x0846),
    s("pa-IN", 0x0446),
    s("pl-PL", 0x0415),
    s("prs-AF", 0x048C),
    s("ps-AF", 0x0463),
    s("pt-BR", 0x0416),
    s("pt-PT", 0x0816),
    s("quc-Latn-GT", 0x0486),
    s("quz-BO", 0x046B),
    s("quz-EC", 0x086B),
    s("quz-PE", 0x0C6B),
    s("rm-CH", 0x0417),
    s("ro-MD", 0x0818),
    s("ro-RO", 0x0418),
    s("ru-MD", 0x0819),
    s("ru-RU", 0x0419),
    s("rw-RW", 0x0487),
    s("sa-IN", 0x044F),
    s("sah-RU", 0x0485),
    s("sd-Arab-PK", 0x0859),
    s("se-FI", 0x0C3B),
    s("se-NO", 0x043B),
    s("se-SE", 0x083B),
    s("si-LK", 0x045B),
    s("sk-SK", 0x041B),
    s("sl-SI", 0x0424),
    s("sma-NO", 0x183B),
    s("sma-SE", 0x1C3B),
    s("smj-NO", 0x103B),
    s("smj-SE", 0x143B),
    s("smn-FI", 0x243B),
    s("sms-FI", 0x203B),
    s("so-SO", 0x0477),
    s("sq-AL", 0x041C),
    s("sr-Cyrl-BA", 0x1C1A),
    s("sr-Cyrl-CS", 0x0C1A),
    s("sr-Cyrl-ME", 0x301A),
    s("sr-Cyrl-RS", 0x281A),
    s("sr-Latn-BA", 0x181A),
    s("sr-Latn-CS", 0x081A),
    s("sr-Latn-ME", 0x2C1A),
    s("sr-Latn-RS", 0x241A),
    s("st-ZA", 0x0430),
    s("sv-FI", 0x081D),
    s("sv-SE", 0x041D),
    s("sw-KE", 0x0441),
    s("syr-SY", 0x045A),
    s("ta-IN", 0x0449),
    s("ta-LK", 0x0849),
    s("te-IN", 0x044A),
    s("tg-Cyrl-TJ", 0x0428),
    s("th-TH", 0x041E),
    s("ti-ER", 0x0873),
    s("ti-ET", 0x0473),
    s("tk-TM", 0x0442),
    s("tn-BW", 0x0832),
    s("tn-ZA", 0x0432),
    s("tr-TR", 0x041F),
    s("ts-ZA", 0x0431),
    s("tt-RU", 0x0444),
    s("tzm-Latn-DZ", 0x085F),
    s("ug-CN", 0x0480),
    s("uk-UA", 0x0422),
    s("ur-IN", 0x0820),
    s("ur-PK", 0x0420),
    s("uz-Cyrl-UZ", 0x0843),
    s("uz-Latn-UZ", 0x0443),
    s("ve-ZA", 0x0433),
    s("vi-VN", 0x042A),
    s("wo-SN", 0x0488),
    s("xh-ZA", 0x0434),
    s("yo-NG", 0x046A),
    s("zh-CN", 0x0804),
    s("zh-HK", 0x0C04),
    s("zh-MO", 0x1404),
    s("zh-SG", 0x1004),
    s("zh-TW", 0x0404),
    s("zu-ZA", 0x0435),
    // Alternate sort orders (Learn, "Sort Order Identifiers"; [MS-LCID]).
    a("de-DE_phoneb", 0x0001_0407, 0x0407),
    a("es-ES_tradnl", 0x0000_040A, 0x0C0A),
    a("hu-HU_technl", 0x0001_040E, 0x040E),
    a("ja-JP_radstr", 0x0004_0411, 0x0411),
    a("ka-GE_modern", 0x0001_0437, 0x0437),
    a("x-IV_mathan", 0x0001_007F, LOCALE_INVARIANT),
    a("zh-CN_stroke", 0x0002_0804, 0x0804),
    a("zh-HK_radstr", 0x0004_0C04, 0x0C04),
    a("zh-MO_radstr", 0x0004_1404, 0x1404),
    a("zh-MO_stroke", 0x0002_1404, 0x1404),
    a("zh-SG_stroke", 0x0002_1004, 0x1004),
    a("zh-TW_pronun", 0x0003_0404, 0x0404),
    a("zh-TW_radstr", 0x0004_0404, 0x0404),
    // Pseudo-locales.
    p("qps-ploc", 0x0501),
    p("qps-ploca", 0x05FE),
    p("qps-plocm", 0x09FF),
    // Script-tagged spellings of specific locales.
    alias("zh-Hans-CN", 0x0804),
    alias("zh-Hans-SG", 0x1004),
    alias("zh-Hant-HK", 0x0C04),
    alias("zh-Hant-MO", 0x1404),
    alias("zh-Hant-TW", 0x0404),
];

/// `a` and `b` are the same name: ASCII case does not matter and `_` stands for `-`
/// (`en_US` is `en-US`).
fn same_name(a: &str, b: &[u16]) -> bool {
    let sep = |c: u16| if c == u16::from(b'_') { u16::from(b'-') } else { c };
    let lower = |c: u16| if (0x41..=0x5A).contains(&c) { c + 0x20 } else { c };
    a.len() == b.len() && a.bytes().zip(b).all(|(x, &y)| lower(sep(u16::from(x))) == lower(sep(y)))
}

/// The locale with this name (the name as the API receives it, without its NUL).
pub fn by_name(name: &[u16]) -> Option<&'static Entry> {
    ENTRIES.iter().find(|e| same_name(e.name, name))
}

/// The locale with this identifier (aliases have none of their own).
pub fn by_lcid(lcid: u32) -> Option<&'static Entry> {
    ENTRIES.iter().find(|e| e.lcid == lcid && e.kind != Kind::Alias)
}

/// The name `LCIDToLocaleName` gives for a locale: a neutral locale's own name only when
/// neutral names are allowed, its default specific locale's otherwise.
pub fn name_of(e: &Entry, allow_neutral: bool) -> &'static str {
    if e.kind == Kind::Neutral && !allow_neutral {
        return by_lcid(e.base).map_or(e.name, |d| d.name);
    }
    e.name
}

/// The identifier `LocaleNameToLCID` gives for a locale: a neutral locale's own one only when
/// neutral names are allowed, its default's otherwise; an alias's locale's.
pub fn lcid_of(e: &Entry, allow_neutral: bool) -> u32 {
    match e.kind {
        Kind::Neutral if !allow_neutral => e.base,
        Kind::Alias => e.base,
        _ => e.lcid,
    }
}

/// The specific locale (or the invariant one) whose formats a locale uses.
pub fn format_locale(e: &'static Entry) -> &'static Entry {
    match e.kind {
        Kind::Specific | Kind::Pseudo => e,
        _ => by_lcid(e.base).unwrap_or(e),
    }
}

/// Whether `name` is a well-formed locale name (the spec sheets' grammar): subtags of one to
/// eight ASCII letters or digits separated by `-` or `_`; the first is a language of two to
/// eight letters, or the single letter `i` or `x` followed by more subtags; at most
/// `LOCALE_NAME_MAX_LENGTH - 1` units. The empty name (the invariant locale) is not one.
pub fn is_well_formed(name: &[u16]) -> bool {
    if name.is_empty() || name.len() >= LOCALE_NAME_MAX_LENGTH {
        return false;
    }
    let is_sep = |c: u16| c == u16::from(b'-') || c == u16::from(b'_');
    let is_alpha = |c: u16| (0x41..=0x5A).contains(&c) || (0x61..=0x7A).contains(&c);
    let is_digit = |c: u16| (0x30..=0x39).contains(&c);
    let mut subtags = name.split(|&c| is_sep(c));
    let first = subtags.next().unwrap_or(&[]);
    let rest: alloc::vec::Vec<&[u16]> = subtags.collect();
    if rest.iter().any(|t| t.is_empty() || t.len() > 8 || !t.iter().all(|&c| is_alpha(c) || is_digit(c))) {
        return false;
    }
    if !first.iter().all(|&c| is_alpha(c)) {
        return false;
    }
    match first.len() {
        1 => matches!(first[0] | 0x20, 0x69 | 0x78) && !rest.is_empty(),
        2..=8 => true,
        _ => false,
    }
}

/// What a locale name names, for the functions that take one (`LocaleNameToLCID` and the
/// `...Ex` formatting functions; the user and system defaults are the caller's to handle).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named {
    /// A locale of the table.
    Known(&'static Entry),
    /// A well-formed name the table lacks; `base` is the locale its part before a sort suffix
    /// (`en-US` of `en-US_tradnl`) names, if the table has that.
    Custom { base: Option<&'static Entry> },
}

/// Look `name` up: the empty name is the invariant locale, a name of the table is its locale,
/// a well-formed name is a custom one, anything else is malformed (`None`).
pub fn lookup(name: &[u16]) -> Option<Named> {
    if name.is_empty() {
        return by_lcid(LOCALE_INVARIANT).map(Named::Known);
    }
    if let Some(e) = by_name(name) {
        return Some(Named::Known(e));
    }
    if !is_well_formed(name) {
        return None;
    }
    let base = name
        .iter()
        .rposition(|&c| c == u16::from(b'_'))
        .and_then(|i| by_name(&name[..i]))
        .filter(|e| e.kind != Kind::AltSort);
    Some(Named::Custom { base })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn the_table_is_consistent() {
        for (i, e) in ENTRIES.iter().enumerate() {
            for f in &ENTRIES[i + 1..] {
                assert!(!same_name(e.name, &w(f.name)), "{} twice", e.name);
                if e.kind != Kind::Alias && f.kind != Kind::Alias {
                    assert_ne!(e.lcid, f.lcid, "{} and {}", e.name, f.name);
                }
            }
            let base = by_lcid(e.base).unwrap_or_else(|| panic!("{}: base {:#x}", e.name, e.base));
            match e.kind {
                Kind::Neutral | Kind::AltSort | Kind::Alias => {
                    assert!(matches!(base.kind, Kind::Specific | Kind::Neutral), "{}", e.name)
                }
                _ => assert_eq!(base, e),
            }
            assert!(e.name.is_empty() || is_well_formed(&w(e.name)), "{}", e.name);
        }
        let count = |k: Kind| ENTRIES.iter().filter(|e| e.kind == k).count();
        // The sheets' bounds: over 100 and under 300 specific locales, over 300 Windows locales
        // in all, more specific than neutral ones, ten or more alternate sorts but under 100.
        assert!((100..300).contains(&count(Kind::Specific)));
        assert!(count(Kind::Specific) + count(Kind::Neutral) > 300);
        assert!(count(Kind::Specific) > count(Kind::Neutral));
        assert!((10..100).contains(&count(Kind::AltSort)));
    }

    #[test]
    fn names_and_identifiers() {
        let name = |lcid: u32, neutral: bool| by_lcid(lcid).map(|e| name_of(e, neutral));
        assert_eq!(name(0x409, false), Some("en-US"));
        assert_eq!(name(0x9, false), Some("en-US"));
        assert_eq!(name(0x9, true), Some("en"));
        assert_eq!(name(0x7C04, false), Some("zh-HK"));
        assert_eq!(name(0x7C04, true), Some("zh-Hant"));
        assert_eq!(name(0x4, false), Some("zh-CN"));
        assert_eq!(name(0x7C, false), Some("moh-CA"));
        assert_eq!(name(0x7F, false), Some(""));
        assert_eq!(name(0x1_0407, false), Some("de-DE_phoneb"));
        assert_eq!(name(0x40A, true), Some("es-ES_tradnl"));
        assert_eq!(name(0x1_0409, false), None);
        let lcid = |n: &str, neutral: bool| by_name(&w(n)).map(|e| lcid_of(e, neutral));
        assert_eq!(lcid("EN_us", false), Some(0x409));
        assert_eq!(lcid("zh", false), Some(0x804));
        assert_eq!(lcid("zh", true), Some(0x7804));
        assert_eq!(lcid("sr-Latn", true), Some(0x701A));
        assert_eq!(lcid("sr-Latn", false), Some(0x241A));
        assert_eq!(lcid("zh-Hans-CN", true), Some(0x804));
        assert_eq!(lcid("x-iv_MATHAN", false), Some(0x1_007F));
        assert_eq!(lcid("qps-plocm", false), Some(0x9FF));
    }

    #[test]
    fn the_grammar_of_a_name() {
        for good in
            ["zz", "xx-XX", "aa-bb-cc-dd-ee", "i-klingon", "de-DE-1996", "de-u-co-phonebk", "en-US_tradnl", "iv"]
        {
            assert!(is_well_formed(&w(good)), "{good}");
        }
        let long = "a".repeat(84);
        for bad in ["en-US-", "-en-US", "en--US", "en US", " en-US", "e", "en-", "-", "!", "0409", "0x409", "x", "x-"] {
            assert!(!is_well_formed(&w(bad)), "{bad}");
        }
        assert!(!is_well_formed(&w(&long)));
        assert_eq!(lookup(&w("")).map(|n| matches!(n, Named::Known(e) if e.lcid == 0x7F)), Some(true));
        assert_eq!(lookup(&w("en-US_tradnl")), Some(Named::Custom { base: by_name(&w("en-US")) }));
        assert_eq!(lookup(&w("xx-XX")), Some(Named::Custom { base: None }));
        assert_eq!(lookup(&w("en US")), None);
    }

    #[test]
    fn formats_follow_the_base_locale() {
        let f = |n: &str| format_locale(by_name(&w(n)).unwrap()).name;
        assert_eq!(f("de"), "de-DE");
        assert_eq!(f("de-DE_phoneb"), "de-DE");
        assert_eq!(f("x-IV_mathan"), "");
        assert_eq!(f("zh-Hans"), "zh-CN");
        assert_eq!(f("zh-Hans-CN"), "zh-CN");
        assert_eq!(f("ja-JP"), "ja-JP");
    }
}
