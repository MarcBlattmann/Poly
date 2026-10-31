//! Distinguished names (RFC 5280 4.1.2.4: `Name ::= RDNSequence`), their
//! attribute values as text, the string forms Learn's `CertNameToStr` page
//! describes, and general names (RFC 5280 4.2.1.6).

use crate::der::{self, Error, Reader, tag};
use crate::oid;
use alloc::string::String;
use alloc::vec::Vec;

/// One attribute of a relative distinguished name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    /// The attribute type, dotted.
    pub oid: String,
    /// The value's tag (a string type, usually).
    pub tag: u8,
    /// The value's contents.
    pub value: Vec<u8>,
    /// The value's whole encoding.
    pub encoded: Vec<u8>,
}

impl Attribute {
    /// The value as text, if it is one of the string types.
    pub fn text(&self) -> Option<String> {
        decode_string(self.tag, &self.value)
    }
}

/// A distinguished name: its encoding and its RDNs in encoded order.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Name {
    pub encoded: Vec<u8>,
    /// Each RDN's attributes (a SET; usually one).
    pub rdns: Vec<Vec<Attribute>>,
}

impl Name {
    /// Parse a `Name` (the whole encoding of the SEQUENCE).
    pub fn parse(encoded: &[u8]) -> Result<Name, Error> {
        let seq = der::parse_one(encoded)?;
        let mut rdns = Vec::new();
        let mut r = Reader::new(seq.expect(tag::SEQUENCE)?);
        while !r.is_empty() {
            let mut set = r.read_tag(tag::SET)?.reader();
            let mut attrs = Vec::new();
            while !set.is_empty() {
                let mut atv = set.sequence()?;
                let oid = oid::to_dotted(atv.oid()?).ok_or(Error::Invalid)?;
                let v = atv.read()?;
                atv.finish()?;
                attrs.push(Attribute { oid, tag: v.tag, value: v.value.to_vec(), encoded: v.raw.to_vec() });
            }
            if attrs.is_empty() {
                return Err(Error::Invalid);
            }
            rdns.push(attrs);
        }
        Ok(Name { encoded: encoded.to_vec(), rdns })
    }

    /// Every attribute, RDN by RDN.
    pub fn attributes(&self) -> impl Iterator<Item = &Attribute> {
        self.rdns.iter().flatten()
    }

    /// The text of the first attribute of type `oid`.
    pub fn first(&self, oid: &str) -> Option<String> {
        self.attributes().find(|a| a.oid == oid).and_then(Attribute::text)
    }

    /// The string form of [`NameStrOptions`].
    pub fn to_str(&self, o: &NameStrOptions) -> String {
        let mut out = String::new();
        let rdns: Vec<&Vec<Attribute>> =
            if o.reverse { self.rdns.iter().rev().collect() } else { self.rdns.iter().collect() };
        for (i, rdn) in rdns.iter().enumerate() {
            if i > 0 {
                out.push_str(o.rdn_separator);
            }
            for (j, a) in rdn.iter().enumerate() {
                if j > 0 {
                    out.push_str(if o.no_plus { " " } else { " + " });
                }
                match o.form {
                    NameForm::Simple => {}
                    NameForm::Oid => {
                        out.push_str(&a.oid);
                        out.push('=');
                    }
                    NameForm::X500 => {
                        out.push_str(oid::attribute_key(&a.oid).unwrap_or(&a.oid));
                        out.push('=');
                    }
                }
                match a.text() {
                    Some(t) if o.no_quoting || !needs_quotes(&t) => out.push_str(&t),
                    Some(t) => {
                        out.push('"');
                        out.push_str(&t.replace('"', "\"\""));
                        out.push('"');
                    }
                    // Not a string: the encoding in hex after '#'.
                    None => {
                        out.push('#');
                        for b in &a.encoded {
                            out.push_str(&alloc::format!("{b:02X}"));
                        }
                    }
                }
            }
        }
        out
    }
}

/// Which keys the string form puts before each value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameForm {
    /// Values only (`CERT_SIMPLE_NAME_STR`).
    Simple,
    /// `2.5.4.3=value` (`CERT_OID_NAME_STR`).
    Oid,
    /// `CN=value`, the OID where there is no key (`CERT_X500_NAME_STR`).
    X500,
}

/// How [`Name::to_str`] renders a name.
#[derive(Clone, Copy, Debug)]
pub struct NameStrOptions {
    pub form: NameForm,
    /// Between RDNs: `", "` by default, `"; "` or `"\r\n"`.
    pub rdn_separator: &'static str,
    /// Last RDN first.
    pub reverse: bool,
    /// The attributes of a multi-valued RDN separated by a space, not `+`.
    pub no_plus: bool,
    /// Never put values in quotation marks.
    pub no_quoting: bool,
}

impl NameStrOptions {
    pub const fn new(form: NameForm) -> Self {
        NameStrOptions { form, rdn_separator: ", ", reverse: false, no_plus: false, no_quoting: false }
    }
}

/// Learn (`CertNameToStr`): a value goes in quotation marks when it has
/// leading or trailing white space or one of `,` `+` `=` `"` `\n` `<` `>`
/// `#` `;`.
fn needs_quotes(s: &str) -> bool {
    s.starts_with([' ', '\t']) || s.ends_with([' ', '\t']) || s.contains([',', '+', '=', '"', '\n', '<', '>', '#', ';'])
}

/// The text of a string-typed value (X.680 41: UTF8String is UTF-8,
/// BMPString UTF-16BE, UniversalString UTF-32BE; the 8-bit types are taken
/// as ISO 8859-1, which covers their ASCII repertoire).
pub fn decode_string(t: u8, v: &[u8]) -> Option<String> {
    match t {
        tag::UTF8_STRING => core::str::from_utf8(v).ok().map(String::from),
        tag::PRINTABLE_STRING
        | tag::IA5_STRING
        | tag::NUMERIC_STRING
        | tag::VISIBLE_STRING
        | tag::T61_STRING
        | tag::VIDEOTEX_STRING
        | tag::GRAPHIC_STRING
        | tag::GENERAL_STRING => Some(v.iter().map(|&b| char::from(b)).collect()),
        tag::BMP_STRING => {
            if !v.len().is_multiple_of(2) {
                return None;
            }
            let units = v.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]]));
            char::decode_utf16(units).collect::<Result<String, _>>().ok()
        }
        tag::UNIVERSAL_STRING => {
            if !v.len().is_multiple_of(4) {
                return None;
            }
            v.chunks(4).map(|c| char::from_u32(u32::from_be_bytes([c[0], c[1], c[2], c[3]]))).collect()
        }
        _ => None,
    }
}

/// One `GeneralName` (RFC 5280 4.2.1.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeneralName {
    /// `[0] otherName`: type and the explicit value's encoding.
    Other { oid: String, value: Vec<u8> },
    /// `[1] rfc822Name`.
    Email(String),
    /// `[2] dNSName`.
    Dns(String),
    /// `[4] directoryName`.
    Directory(Name),
    /// `[6] uniformResourceIdentifier`.
    Uri(String),
    /// `[7] iPAddress`: 4 or 16 octets.
    Ip(Vec<u8>),
    /// `[8] registeredID`.
    Registered(String),
    /// x400Address, ediPartyName: kept as their encoding.
    Unsupported(u8, Vec<u8>),
}

fn ia5(v: &[u8]) -> Result<String, Error> {
    if !v.is_ascii() {
        return Err(Error::Invalid);
    }
    Ok(v.iter().map(|&b| char::from(b)).collect())
}

/// Parse `GeneralNames ::= SEQUENCE SIZE (1..MAX) OF GeneralName` (the
/// whole encoding).
pub fn parse_general_names(encoded: &[u8]) -> Result<Vec<GeneralName>, Error> {
    let seq = der::parse_one(encoded)?;
    let mut r = Reader::new(seq.expect(tag::SEQUENCE)?);
    let mut out = Vec::new();
    while !r.is_empty() {
        out.push(parse_general_name(r.read()?)?);
    }
    Ok(out)
}

/// One element of a `GeneralNames`.
pub fn parse_general_name(e: der::Tlv<'_>) -> Result<GeneralName, Error> {
    Ok(match e.tag {
        0xa0 => {
            let mut r = e.reader();
            let oid = oid::to_dotted(r.oid()?).ok_or(Error::Invalid)?;
            let v = r.read_tag(tag::context_cons(0))?;
            GeneralName::Other { oid, value: v.value.to_vec() }
        }
        0x81 => GeneralName::Email(ia5(e.value)?),
        0x82 => GeneralName::Dns(ia5(e.value)?),
        0xa4 => GeneralName::Directory(Name::parse(der::parse_one(e.value)?.raw)?),
        0x86 => GeneralName::Uri(ia5(e.value)?),
        0x87 => GeneralName::Ip(e.value.to_vec()),
        0x88 => {
            if e.value.is_empty() {
                return Err(Error::Invalid);
            }
            GeneralName::Registered(oid::to_dotted(e.value).ok_or(Error::Invalid)?)
        }
        t => GeneralName::Unsupported(t, e.raw.to_vec()),
    })
}

/// Whether two names are the same name. RFC 5280 7.1 compares
/// attribute values after normalising strings; this compares the
/// encodings first and then RDN by RDN with case-insensitive ASCII
/// comparison of string values and collapsed spaces, which covers how
/// issuers really encode names.
pub fn names_match(a: &Name, b: &Name) -> bool {
    if a.encoded == b.encoded {
        return true;
    }
    if a.rdns.len() != b.rdns.len() {
        return false;
    }
    a.rdns.iter().zip(&b.rdns).all(|(x, y)| {
        x.len() == y.len()
            && x.iter().all(|ax| {
                y.iter().any(|ay| {
                    ax.oid == ay.oid
                        && match (ax.text(), ay.text()) {
                            (Some(s), Some(t)) => normalise(&s) == normalise(&t),
                            _ => ax.encoded == ay.encoded,
                        }
                })
            })
    })
}

fn normalise(s: &str) -> String {
    let mut out = String::new();
    for w in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&w.to_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atv(oid_s: &str, t: u8, v: &[u8]) -> Vec<u8> {
        let o = der::encode(tag::OID, &oid::from_dotted(oid_s).unwrap());
        der::encode(tag::SET, &der::encode_sequence(&[&o, &der::encode(t, v)]))
    }

    fn name(parts: &[Vec<u8>]) -> Name {
        Name::parse(&der::encode(tag::SEQUENCE, &parts.concat())).unwrap()
    }

    #[test]
    fn string_forms() {
        let n = name(&[
            atv(oid::COUNTRY_NAME, tag::PRINTABLE_STRING, b"CH"),
            atv(oid::ORGANIZATION_NAME, tag::UTF8_STRING, "Zürich, AG".as_bytes()),
            atv(oid::COMMON_NAME, tag::BMP_STRING, &[0, b'H', 0, b'i']),
            atv("1.2.3.4", tag::INTEGER, &[5]),
        ]);
        let x500 = NameStrOptions::new(NameForm::X500);
        assert_eq!(n.to_str(&x500), "C=CH, O=\"Zürich, AG\", CN=Hi, 1.2.3.4=#020105");
        let simple = NameStrOptions { reverse: true, rdn_separator: "; ", ..NameStrOptions::new(NameForm::Simple) };
        assert_eq!(n.to_str(&simple), "#020105; Hi; \"Zürich, AG\"; CH");
        let o = NameStrOptions { no_quoting: true, ..NameStrOptions::new(NameForm::Oid) };
        assert_eq!(n.to_str(&o), "2.5.4.6=CH, 2.5.4.10=Zürich, AG, 2.5.4.3=Hi, 1.2.3.4=#020105");
        assert_eq!(n.first(oid::COMMON_NAME).as_deref(), Some("Hi"));
    }

    #[test]
    fn quotes_doubled_and_multi_valued() {
        let set = {
            let a = der::encode_sequence(&[
                &der::encode(tag::OID, &oid::from_dotted(oid::COMMON_NAME).unwrap()),
                &der::encode(tag::UTF8_STRING, b"a\"b"),
            ]);
            let b = der::encode_sequence(&[
                &der::encode(tag::OID, &oid::from_dotted(oid::ORGANIZATIONAL_UNIT_NAME).unwrap()),
                &der::encode(tag::UTF8_STRING, b" x"),
            ]);
            der::encode(tag::SET, &[a, b].concat())
        };
        let n = name(&[set]);
        assert_eq!(n.to_str(&NameStrOptions::new(NameForm::X500)), "CN=\"a\"\"b\" + OU=\" x\"");
    }

    #[test]
    fn matching_ignores_case_and_spaces() {
        let a = name(&[atv(oid::COMMON_NAME, tag::PRINTABLE_STRING, b"Poly  Root")]);
        let b = name(&[atv(oid::COMMON_NAME, tag::UTF8_STRING, b"poly root")]);
        let c = name(&[atv(oid::COMMON_NAME, tag::UTF8_STRING, b"other")]);
        assert!(names_match(&a, &b));
        assert!(!names_match(&a, &c));
    }

    #[test]
    fn general_names() {
        let gn = der::encode_sequence(&[
            &der::encode(0x82, b"example.test"),
            &der::encode(0x81, b"a@example.test"),
            &der::encode(0x87, &[192, 0, 2, 1]),
            &der::encode(0x86, b"https://x/"),
        ]);
        let names = parse_general_names(&gn).unwrap();
        assert_eq!(
            names,
            [
                GeneralName::Dns("example.test".into()),
                GeneralName::Email("a@example.test".into()),
                GeneralName::Ip([192, 0, 2, 1].to_vec()),
                GeneralName::Uri("https://x/".into()),
            ]
        );
    }
}
