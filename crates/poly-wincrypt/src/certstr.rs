//! CryptoAPI's text functions: `CertNameToStr` string types and flags,
//! `CertGetNameString`'s name types, and the formats of
//! `CryptBinaryToString` / `CryptStringToBinary` (Learn's pages of each;
//! constants from `wincrypt.h`).

use crate::status::*;
use alloc::string::String;
use alloc::vec::Vec;
use poly_x509::Certificate;
use poly_x509::name::{GeneralName, Name, NameForm, NameStrOptions};
use poly_x509::{oid, text};

// `CertNameToStr` string types and flags.
pub const CERT_SIMPLE_NAME_STR: u32 = 1;
pub const CERT_OID_NAME_STR: u32 = 2;
pub const CERT_X500_NAME_STR: u32 = 3;
pub const CERT_NAME_STR_SEMICOLON_FLAG: u32 = 0x4000_0000;
pub const CERT_NAME_STR_NO_PLUS_FLAG: u32 = 0x2000_0000;
pub const CERT_NAME_STR_NO_QUOTING_FLAG: u32 = 0x1000_0000;
pub const CERT_NAME_STR_CRLF_FLAG: u32 = 0x0800_0000;
pub const CERT_NAME_STR_COMMA_FLAG: u32 = 0x0400_0000;
pub const CERT_NAME_STR_REVERSE_FLAG: u32 = 0x0200_0000;

/// `CertNameToStr`: RDNs in encoded order unless reversed, separated by
/// ", " (or "; " / "\r\n"), multi-valued RDNs by " + ".
pub fn name_to_str(n: &Name, str_type: u32) -> String {
    let form = match str_type & 0xffff {
        CERT_OID_NAME_STR => NameForm::Oid,
        CERT_X500_NAME_STR => NameForm::X500,
        _ => NameForm::Simple,
    };
    let sep = if str_type & CERT_NAME_STR_SEMICOLON_FLAG != 0 {
        "; "
    } else if str_type & CERT_NAME_STR_CRLF_FLAG != 0 {
        "\r\n"
    } else {
        ", "
    };
    n.to_str(&NameStrOptions {
        form,
        rdn_separator: sep,
        reverse: str_type & CERT_NAME_STR_REVERSE_FLAG != 0,
        no_plus: str_type & CERT_NAME_STR_NO_PLUS_FLAG != 0,
        no_quoting: str_type & CERT_NAME_STR_NO_QUOTING_FLAG != 0,
    })
}

// `CertGetNameString` types and flags.
pub const CERT_NAME_EMAIL_TYPE: u32 = 1;
pub const CERT_NAME_RDN_TYPE: u32 = 2;
pub const CERT_NAME_ATTR_TYPE: u32 = 3;
pub const CERT_NAME_SIMPLE_DISPLAY_TYPE: u32 = 4;
pub const CERT_NAME_FRIENDLY_DISPLAY_TYPE: u32 = 5;
pub const CERT_NAME_DNS_TYPE: u32 = 6;
pub const CERT_NAME_URL_TYPE: u32 = 7;
pub const CERT_NAME_UPN_TYPE: u32 = 8;
pub const CERT_NAME_ISSUER_FLAG: u32 = 0x1;
/// `szOID_NT_PRINCIPAL_NAME`.
pub const NT_PRINCIPAL_NAME: &str = "1.3.6.1.4.1.311.20.2.3";

/// `pvTypePara` of `CertGetNameString`, read by the caller per type.
#[derive(Clone, Debug)]
pub enum NamePara {
    None,
    /// `CERT_NAME_RDN_TYPE`: a `CertNameToStr` string type.
    StrType(u32),
    /// `CERT_NAME_ATTR_TYPE`: an attribute OID.
    Oid(String),
}

/// `CertGetNameString`: the name (empty when there is none, which the
/// caller returns as a string of just the NUL). Learn's search orders:
/// e-mail = the alternative name's rfc822Name, else the emailAddress
/// attribute; simple display = CN, OU, O, emailAddress, then the rfc822
/// alternative name; DNS = the dNSName alternative name, else CN; URL =
/// the URI alternative name; UPN = the otherName with
/// `szOID_NT_PRINCIPAL_NAME`. `CERT_NAME_ISSUER_FLAG` uses the issuer and
/// the issuer alternative name.
pub fn get_name_string(c: &Certificate, kind: u32, flags: u32, para: &NamePara, friendly: Option<&str>) -> String {
    let issuer = flags & CERT_NAME_ISSUER_FLAG != 0;
    let name = if issuer { &c.issuer } else { &c.subject };
    let alt: Vec<GeneralName> = if issuer {
        c.extension(oid::ISSUER_ALT_NAME)
            .and_then(|e| poly_x509::name::parse_general_names(&e.value).ok())
            .unwrap_or_default()
    } else {
        c.subject_alt_names()
    };
    let first_alt = |pick: fn(&GeneralName) -> Option<String>| alt.iter().find_map(pick);
    let email = || first_alt(|g| if let GeneralName::Email(s) = g { Some(s.clone()) } else { None });
    let simple = || {
        [oid::COMMON_NAME, oid::ORGANIZATIONAL_UNIT_NAME, oid::ORGANIZATION_NAME, oid::EMAIL_ADDRESS]
            .iter()
            .find_map(|o| name.first(o))
            .or_else(email)
    };
    match kind {
        CERT_NAME_EMAIL_TYPE => email().or_else(|| name.first(oid::EMAIL_ADDRESS)),
        CERT_NAME_RDN_TYPE => {
            let t = if let NamePara::StrType(t) = para { *t } else { CERT_SIMPLE_NAME_STR };
            Some(name_to_str(name, t))
        }
        CERT_NAME_ATTR_TYPE => match para {
            NamePara::Oid(o) => name.first(o),
            _ => None,
        },
        CERT_NAME_SIMPLE_DISPLAY_TYPE => simple(),
        CERT_NAME_FRIENDLY_DISPLAY_TYPE => friendly.map(String::from).or_else(simple),
        CERT_NAME_DNS_TYPE => first_alt(|g| if let GeneralName::Dns(s) = g { Some(s.clone()) } else { None })
            .or_else(|| name.first(oid::COMMON_NAME)),
        CERT_NAME_URL_TYPE => first_alt(|g| if let GeneralName::Uri(s) = g { Some(s.clone()) } else { None }),
        CERT_NAME_UPN_TYPE => first_alt(|g| match g {
            GeneralName::Other { oid, value } if oid == NT_PRINCIPAL_NAME => {
                let el = poly_x509::der::parse_one(value).ok()?;
                poly_x509::name::decode_string(el.tag, el.value)
            }
            _ => None,
        }),
        _ => None,
    }
    .unwrap_or_default()
}

// `CryptBinaryToString` / `CryptStringToBinary` formats and flags.
pub const CRYPT_STRING_BASE64HEADER: u32 = 0x0;
pub const CRYPT_STRING_BASE64: u32 = 0x1;
pub const CRYPT_STRING_BINARY: u32 = 0x2;
pub const CRYPT_STRING_BASE64REQUESTHEADER: u32 = 0x3;
pub const CRYPT_STRING_HEX: u32 = 0x4;
pub const CRYPT_STRING_HEXASCII: u32 = 0x5;
pub const CRYPT_STRING_BASE64_ANY: u32 = 0x6;
pub const CRYPT_STRING_ANY: u32 = 0x7;
pub const CRYPT_STRING_HEX_ANY: u32 = 0x8;
pub const CRYPT_STRING_BASE64X509CRLHEADER: u32 = 0x9;
pub const CRYPT_STRING_HEXADDR: u32 = 0xa;
pub const CRYPT_STRING_HEXASCIIADDR: u32 = 0xb;
pub const CRYPT_STRING_HEXRAW: u32 = 0xc;
pub const CRYPT_STRING_STRICT: u32 = 0x2000_0000;
pub const CRYPT_STRING_NOCRLF: u32 = 0x4000_0000;
pub const CRYPT_STRING_NOCR: u32 = 0x8000_0000;

fn header_label(format: u32) -> Option<&'static str> {
    match format {
        CRYPT_STRING_BASE64HEADER => Some("CERTIFICATE"),
        CRYPT_STRING_BASE64REQUESTHEADER => Some("NEW CERTIFICATE REQUEST"),
        CRYPT_STRING_BASE64X509CRLHEADER => Some("X509 CRL"),
        _ => None,
    }
}

/// `CryptBinaryToString`'s text (without the terminating NUL). Base64 is
/// in lines of 64 characters, each ended by CR LF (LF with
/// `CRYPT_STRING_NOCR`, nothing with `CRYPT_STRING_NOCRLF`, Learn). The
/// hex forms are 16 bytes a line, two spaces after the eighth; the
/// address forms start lines with a 4-digit offset; the ASCII forms end
/// them with the printable characters (layouts to be confirmed).
/// `ERROR_INVALID_PARAMETER` for formats it does not produce.
pub fn binary_to_string(data: &[u8], flags: u32) -> Result<String, u32> {
    let format = flags & 0x0fff_ffff & !CRYPT_STRING_STRICT;
    let eol = if flags & CRYPT_STRING_NOCRLF != 0 {
        ""
    } else if flags & CRYPT_STRING_NOCR != 0 {
        "\n"
    } else {
        "\r\n"
    };
    let mut s = String::new();
    match format {
        CRYPT_STRING_BASE64
        | CRYPT_STRING_BASE64HEADER
        | CRYPT_STRING_BASE64REQUESTHEADER
        | CRYPT_STRING_BASE64X509CRLHEADER => {
            let label = header_label(format);
            // The header lines always end a line (Learn's examples).
            let hdr_eol = if eol.is_empty() { "\r\n" } else { eol };
            if let Some(l) = label {
                s.push_str("-----BEGIN ");
                s.push_str(l);
                s.push_str("-----");
                s.push_str(hdr_eol);
            }
            let b64 = text::base64_encode(data);
            for line in b64.as_bytes().chunks(64) {
                s.push_str(core::str::from_utf8(line).unwrap_or_default());
                s.push_str(eol);
            }
            if let Some(l) = label {
                if eol.is_empty() {
                    s.push_str(hdr_eol);
                }
                s.push_str("-----END ");
                s.push_str(l);
                s.push_str("-----");
                s.push_str(hdr_eol);
            }
        }
        CRYPT_STRING_HEXRAW => {
            s.push_str(&text::hex_encode(data));
            s.push_str(eol);
        }
        CRYPT_STRING_HEX | CRYPT_STRING_HEXADDR | CRYPT_STRING_HEXASCII | CRYPT_STRING_HEXASCIIADDR => {
            let addr = matches!(format, CRYPT_STRING_HEXADDR | CRYPT_STRING_HEXASCIIADDR);
            let ascii = matches!(format, CRYPT_STRING_HEXASCII | CRYPT_STRING_HEXASCIIADDR);
            for (i, line) in data.chunks(16).enumerate() {
                if addr {
                    s.push_str(&alloc::format!("{:04x}    ", i * 16));
                }
                let mut hex = String::new();
                for (j, b) in line.iter().enumerate() {
                    if j == 8 {
                        hex.push(' ');
                    }
                    if j > 0 {
                        hex.push(' ');
                    }
                    hex.push_str(&alloc::format!("{b:02x}"));
                }
                s.push_str(&hex);
                if ascii {
                    for _ in hex.len()..48 {
                        s.push(' ');
                    }
                    s.push_str("   ");
                    s.extend(line.iter().map(|&b| if (0x20..0x7f).contains(&b) { char::from(b) } else { '.' }));
                }
                s.push_str(eol);
            }
        }
        _ => return Err(ERROR_INVALID_PARAMETER),
    }
    Ok(s)
}

/// What `CryptStringToBinary` found: the bytes, the characters skipped
/// before them (`pdwSkip`) and the format used (`pdwFlags`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub data: Vec<u8>,
    pub skip: usize,
    pub format: u32,
}

fn hex_lines(s: &str, addr: bool, ascii: bool) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for line in s.lines() {
        // The ASCII column starts after the 48 columns of hex (and the
        // address's 8).
        let width = 48 + if addr { 8 } else { 0 };
        let line = if ascii { line.get(..width.min(line.len())).unwrap_or(line) } else { line };
        let mut tokens = line.split_whitespace();
        if addr {
            match tokens.next() {
                None => continue,
                // An address is at least four hex digits.
                Some(a) if a.len() >= 4 && a.bytes().all(|b| b.is_ascii_hexdigit()) => {}
                Some(_) => return None,
            }
        }
        for t in tokens.take(16) {
            if t.len() != 2 {
                break;
            }
            out.extend(text::hex_decode(t.as_bytes())?);
        }
    }
    Some(out)
}

/// `CryptStringToBinary` of text `s` in `format` (the `*_ANY` forms try
/// their members in Learn's order). `ERROR_INVALID_DATA` for text that is
/// not in the format (to be confirmed).
pub fn string_to_binary(s: &str, format: u32) -> Result<Decoded, u32> {
    let one = |f: u32| -> Option<Decoded> {
        Some(match f {
            CRYPT_STRING_BASE64HEADER | CRYPT_STRING_BASE64REQUESTHEADER | CRYPT_STRING_BASE64X509CRLHEADER => {
                let b = text::pem_find(s.as_bytes())?;
                if f != CRYPT_STRING_BASE64HEADER && Some(b.label) != header_label(f) {
                    return None;
                }
                let skip = s.find("-----BEGIN ")?;
                Decoded { data: text::base64_decode(b.body)?, skip, format: f }
            }
            CRYPT_STRING_BASE64 => Decoded { data: text::base64_decode(s.as_bytes())?, skip: 0, format: f },
            CRYPT_STRING_BINARY => Decoded { data: s.chars().map(|c| c as u32 as u8).collect(), skip: 0, format: f },
            CRYPT_STRING_HEX | CRYPT_STRING_HEXRAW => {
                Decoded { data: text::hex_decode(s.as_bytes())?, skip: 0, format: f }
            }
            CRYPT_STRING_HEXADDR => Decoded { data: hex_lines(s, true, false)?, skip: 0, format: f },
            CRYPT_STRING_HEXASCIIADDR => Decoded { data: hex_lines(s, true, true)?, skip: 0, format: f },
            CRYPT_STRING_HEXASCII => Decoded { data: hex_lines(s, false, true)?, skip: 0, format: f },
            _ => return None,
        })
    };
    let order: &[u32] = match format & 0xffff {
        CRYPT_STRING_BASE64_ANY => &[CRYPT_STRING_BASE64HEADER, CRYPT_STRING_BASE64],
        CRYPT_STRING_ANY => &[CRYPT_STRING_BASE64HEADER, CRYPT_STRING_BASE64, CRYPT_STRING_BINARY],
        CRYPT_STRING_HEX_ANY => &[
            CRYPT_STRING_HEXADDR,
            CRYPT_STRING_HEXASCIIADDR,
            CRYPT_STRING_HEX,
            CRYPT_STRING_HEXRAW,
            CRYPT_STRING_HEXASCII,
        ],
        f @ 0..=0xc => return one(f).ok_or(ERROR_INVALID_DATA),
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    order.iter().find_map(|&f| one(f)).ok_or(ERROR_INVALID_DATA)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEAF: &str = include_str!("../../poly-x509/src/testdata/leaf.pem");
    const SELF_SIGNED: &str = include_str!("../../poly-x509/src/testdata/self.pem");

    fn cert(pem: &str) -> Certificate {
        Certificate::parse(&text::pem_decode(pem.as_bytes(), None).unwrap()).unwrap()
    }

    #[test]
    fn name_strings() {
        let c = cert(LEAF);
        assert_eq!(
            name_to_str(&c.subject, CERT_X500_NAME_STR | CERT_NAME_STR_REVERSE_FLAG),
            "E=admin@example.test, CN=www.example.test, O=Poly Test, L=Zurich, C=CH"
        );
        assert_eq!(
            name_to_str(&c.issuer, CERT_SIMPLE_NAME_STR | CERT_NAME_STR_SEMICOLON_FLAG),
            "CH; Poly Test; Issuing; Poly Test Intermediate CA"
        );
        let s = cert(SELF_SIGNED);
        assert_eq!(name_to_str(&s.subject, CERT_X500_NAME_STR), "CN=\"Self Signed, Inc.\", O=Quote Test");
    }

    #[test]
    fn get_name_string_types() {
        let c = cert(LEAF);
        let g = |k, f, p: &NamePara| get_name_string(&c, k, f, p, None);
        assert_eq!(g(CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, &NamePara::None), "www.example.test");
        assert_eq!(
            g(CERT_NAME_SIMPLE_DISPLAY_TYPE, CERT_NAME_ISSUER_FLAG, &NamePara::None),
            "Poly Test Intermediate CA"
        );
        assert_eq!(g(CERT_NAME_EMAIL_TYPE, 0, &NamePara::None), "admin@example.test");
        assert_eq!(g(CERT_NAME_DNS_TYPE, 0, &NamePara::None), "www.example.test");
        assert_eq!(g(CERT_NAME_URL_TYPE, 0, &NamePara::None), "https://example.test/");
        assert_eq!(g(CERT_NAME_ATTR_TYPE, 0, &NamePara::Oid(oid::LOCALITY_NAME.into())), "Zurich");
        assert_eq!(
            g(CERT_NAME_RDN_TYPE, CERT_NAME_ISSUER_FLAG, &NamePara::StrType(CERT_X500_NAME_STR)),
            "C=CH, O=Poly Test, OU=Issuing, CN=Poly Test Intermediate CA"
        );
        assert_eq!(g(CERT_NAME_UPN_TYPE, 0, &NamePara::None), "");
        assert_eq!(g(CERT_NAME_EMAIL_TYPE, CERT_NAME_ISSUER_FLAG, &NamePara::None), "");
        assert_eq!(get_name_string(&c, CERT_NAME_FRIENDLY_DISPLAY_TYPE, 0, &NamePara::None, Some("Mine")), "Mine");
    }

    #[test]
    fn base64_forms() {
        let data: Vec<u8> = (0u8..60).collect();
        let b = binary_to_string(&data, CRYPT_STRING_BASE64).unwrap();
        let lines: Vec<&str> = b.split("\r\n").collect();
        assert_eq!((lines[0].len(), lines[1].len(), lines[2]), (64, 16, ""));
        assert_eq!(binary_to_string(b"foobar", CRYPT_STRING_BASE64 | CRYPT_STRING_NOCRLF).unwrap(), "Zm9vYmFy");
        let h = binary_to_string(b"foobar", CRYPT_STRING_BASE64HEADER | CRYPT_STRING_NOCR).unwrap();
        assert_eq!(h, "-----BEGIN CERTIFICATE-----\nZm9vYmFy\n-----END CERTIFICATE-----\n");
        let d = string_to_binary(&alloc::format!("junk {h}"), CRYPT_STRING_BASE64_ANY).unwrap();
        assert_eq!(d, Decoded { data: b"foobar".to_vec(), skip: 5, format: CRYPT_STRING_BASE64HEADER });
        let d = string_to_binary("Zm9v\r\nYmFy", CRYPT_STRING_ANY).unwrap();
        assert_eq!((d.data, d.format), (b"foobar".to_vec(), CRYPT_STRING_BASE64));
        assert_eq!(string_to_binary("Zm9v!", CRYPT_STRING_BASE64), Err(ERROR_INVALID_DATA));
        let r = binary_to_string(b"x", CRYPT_STRING_BASE64REQUESTHEADER).unwrap();
        assert!(string_to_binary(&r, CRYPT_STRING_BASE64HEADER).is_ok());
        assert!(string_to_binary(&r, CRYPT_STRING_BASE64X509CRLHEADER).is_err());
    }

    #[test]
    fn hex_forms() {
        let data: Vec<u8> = (0u8..18).collect();
        let h = binary_to_string(&data, CRYPT_STRING_HEX).unwrap();
        assert_eq!(h, "00 01 02 03 04 05 06 07  08 09 0a 0b 0c 0d 0e 0f\r\n10 11\r\n");
        assert_eq!(binary_to_string(&[0xab, 1], CRYPT_STRING_HEXRAW | CRYPT_STRING_NOCRLF).unwrap(), "ab01");
        let a = binary_to_string(b"AB", CRYPT_STRING_HEXASCIIADDR).unwrap();
        assert!(a.starts_with("0000    41 42"));
        assert!(a.ends_with("   AB\r\n"));
        for f in [
            CRYPT_STRING_HEX,
            CRYPT_STRING_HEXADDR,
            CRYPT_STRING_HEXASCIIADDR,
            CRYPT_STRING_HEXASCII,
            CRYPT_STRING_HEXRAW,
        ] {
            let t = binary_to_string(&data, f).unwrap();
            assert_eq!(string_to_binary(&t, f).unwrap().data, data, "format {f}");
        }
        assert_eq!(string_to_binary(&h, CRYPT_STRING_HEX_ANY).unwrap().data.len(), 18);
        assert_eq!(binary_to_string(b"x", 0x55), Err(ERROR_INVALID_PARAMETER));
    }
}
