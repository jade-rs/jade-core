//! Digital signatures in a package (ECMA-376 Part 2 §12, "Digital
//! Signatures", over W3C XML-Signature): reading the signatures a package
//! carries and checking them, signing one, and taking them out.
//!
//! A signature part (`_xmlsignatures/sigN.xml`, reached through the origin
//! part the package's relationships name) is an XML-DSig `Signature`. Its
//! `SignedInfo` lists digests of its `Object`s — the package object, whose
//! `Manifest` lists a digest of every signed part, the Office details and the
//! XAdES signed properties — and is itself signed with the certificate's
//! key (RSA PKCS #1 v1.5). Checking one means: the parts as they are now
//! still digest to what the manifest says, the objects to what `SignedInfo`
//! says, and the signature value verifies over `SignedInfo`. XML is digested
//! in its Canonical XML 1.0 form, and relationship parts through the
//! package's relationships transform (Part 2 §13.2.4.24).

use std::collections::BTreeMap;

use base64::Engine;
use quick_xml::events::Event;
use quick_xml::Reader;
use sha2::Digest;

use crate::{rels_part_name, OpcPackage};

const ORIGIN_TYPE: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/digital-signature/origin";
const SIGNATURE_TYPE: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/digital-signature/signature";
const ORIGIN_CONTENT_TYPE: &str = "application/vnd.openxmlformats-package.digital-signature-origin";
const SIGNATURE_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-package.digital-signature-xmlsignature+xml";
const DSIG: &str = "http://www.w3.org/2000/09/xmldsig#";
const C14N: &str = "http://www.w3.org/TR/2001/REC-xml-c14n-20010315";
const RELATIONSHIP_TRANSFORM: &str =
    "http://schemas.openxmlformats.org/package/2006/RelationshipTransform";
const SHA256: &str = "http://www.w3.org/2001/04/xmlenc#sha256";
const RSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256";

/// Whether a signature still holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignatureStatus {
    Valid,
    /// Why not: a part changed, the signature does not verify, or it could
    /// not be read.
    Invalid(String),
}

/// A signature a package carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageSignature {
    /// Its part, e.g. `_xmlsignatures/sig1.xml`.
    pub part: String,
    /// The certificate's subject common name.
    pub signer: Option<String>,
    /// The certificate's issuer common name.
    pub issuer: Option<String>,
    /// When it was signed, as the signature says.
    pub signed_at: Option<String>,
    /// 署名の目的 (`SignatureComments`).
    pub comment: Option<String>,
    pub status: SignatureStatus,
}

/// What signs: a certificate and its RSA private key, wherever the key is.
pub trait Signer {
    /// The certificate, DER.
    fn certificate(&self) -> Vec<u8>;
    /// RSASSA-PKCS1-v1_5 with SHA-256 over `data`.
    fn sign_sha256(&self, data: &[u8]) -> Result<Vec<u8>, String>;
}

// ---- a small XML tree -------------------------------------------------------

#[derive(Clone, Debug)]
enum Node {
    Element(Element),
    Text(String),
}

#[derive(Clone, Debug)]
struct Element {
    /// The qualified name, as written.
    name: String,
    /// Attributes as written, namespace declarations included.
    attributes: Vec<(String, String)>,
    children: Vec<Node>,
}

impl Element {
    fn local(&self) -> &str {
        local(&self.name)
    }

    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            Node::Element(element) => Some(element),
            Node::Text(_) => None,
        })
    }

    fn child(&self, local_name: &str) -> Option<&Element> {
        self.elements()
            .find(|element| element.local() == local_name)
    }

    fn text(&self) -> String {
        let mut out = String::new();
        for node in &self.children {
            match node {
                Node::Text(text) => out.push_str(text),
                Node::Element(element) => out.push_str(&element.text()),
            }
        }
        out
    }

    /// Depth first, with the namespaces in scope at each element (its own
    /// declarations not yet applied).
    fn find<'a>(
        &'a self,
        scope: &BTreeMap<String, String>,
        test: &dyn Fn(&Element) -> bool,
    ) -> Option<(&'a Element, BTreeMap<String, String>)> {
        if test(self) {
            return Some((self, scope.clone()));
        }
        let inner = with_declarations(scope, self);
        self.elements().find_map(|child| child.find(&inner, test))
    }
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// `scope` with `element`'s namespace declarations applied.
fn with_declarations(
    scope: &BTreeMap<String, String>,
    element: &Element,
) -> BTreeMap<String, String> {
    let mut inner = scope.clone();
    for (key, value) in &element.attributes {
        if key == "xmlns" {
            inner.insert(String::new(), value.clone());
        } else if let Some(prefix) = key.strip_prefix("xmlns:") {
            inner.insert(prefix.to_string(), value.clone());
        }
    }
    inner
}

fn parse(xml: &str) -> Result<Element, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    let open = |start: &quick_xml::events::BytesStart| -> Result<Element, String> {
        let mut attributes = Vec::new();
        for attribute in start.attributes() {
            let attribute = attribute.map_err(|error| error.to_string())?;
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| error.to_string())?
                .into_owned();
            attributes.push((
                String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                value,
            ));
        }
        Ok(Element {
            name: String::from_utf8_lossy(start.name().as_ref()).into_owned(),
            attributes,
            children: Vec::new(),
        })
    };
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(start) => stack.push(open(&start)?),
            Event::Empty(start) => {
                let element = open(&start)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Node::Element(element)),
                    None => root = Some(element),
                }
            }
            Event::End(_) => {
                let element = stack.pop().ok_or("unbalanced XML")?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Node::Element(element)),
                    None => root = Some(element),
                }
            }
            Event::Text(text) => {
                if let Some(parent) = stack.last_mut() {
                    let text = text.decode().map_err(|error| error.to_string())?;
                    let text = quick_xml::escape::unescape(&text)
                        .map_err(|error| error.to_string())?
                        .into_owned();
                    parent.children.push(Node::Text(text));
                }
            }
            Event::CData(data) => {
                if let Some(parent) = stack.last_mut() {
                    parent
                        .children
                        .push(Node::Text(String::from_utf8_lossy(&data).into_owned()));
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(parent) = stack.last_mut() {
                    let name = String::from_utf8_lossy(&reference).into_owned();
                    let text = match name.as_str() {
                        "amp" => "&".to_string(),
                        "lt" => "<".to_string(),
                        "gt" => ">".to_string(),
                        "quot" => "\"".to_string(),
                        "apos" => "'".to_string(),
                        _ => {
                            let code = name
                                .strip_prefix("#x")
                                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                                .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                                .and_then(char::from_u32)
                                .ok_or_else(|| format!("unknown entity &{name};"))?;
                            code.to_string()
                        }
                    };
                    parent.children.push(Node::Text(text));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    root.ok_or_else(|| "no root element".to_string())
}

// ---- Canonical XML 1.0 (inclusive, without comments) ----------------------

fn escape_text(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(ch),
        }
    }
}

fn escape_attribute(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(ch),
        }
    }
}

/// `element` in canonical form, with `scope` the namespaces in scope from
/// its ancestors and `rendered` those already written by the output's
/// ancestors.
fn canonicalize(
    element: &Element,
    scope: &BTreeMap<String, String>,
    rendered: &BTreeMap<String, String>,
    out: &mut String,
) {
    let inner = with_declarations(scope, element);
    out.push('<');
    out.push_str(&element.name);
    let mut now_rendered = rendered.clone();
    for (prefix, uri) in &inner {
        if prefix == "xml" {
            continue;
        }
        let shown = rendered.get(prefix);
        let needed = if prefix.is_empty() {
            // An empty default namespace is written only to undo one above.
            match shown {
                Some(above) => above != uri,
                None => !uri.is_empty(),
            }
        } else {
            shown != Some(uri)
        };
        if needed {
            if prefix.is_empty() {
                out.push_str(" xmlns=\"");
            } else {
                out.push_str(" xmlns:");
                out.push_str(prefix);
                out.push_str("=\"");
            }
            escape_attribute(uri, out);
            out.push('"');
            now_rendered.insert(prefix.clone(), uri.clone());
        }
    }
    let mut attributes: Vec<(String, &str, &str)> = element
        .attributes
        .iter()
        .filter(|(key, _)| key != "xmlns" && !key.starts_with("xmlns:"))
        .map(|(key, value)| {
            let uri = match key.split_once(':') {
                Some((prefix, _)) => inner.get(prefix).cloned().unwrap_or_default(),
                None => String::new(),
            };
            (uri, key.as_str(), value.as_str())
        })
        .collect();
    attributes.sort_by(|a, b| (a.0.as_str(), local(a.1)).cmp(&(b.0.as_str(), local(b.1))));
    for (_, key, value) in attributes {
        out.push(' ');
        out.push_str(key);
        out.push_str("=\"");
        escape_attribute(value, out);
        out.push('"');
    }
    out.push('>');
    for node in &element.children {
        match node {
            Node::Text(text) => escape_text(text, out),
            Node::Element(child) => canonicalize(child, &inner, &now_rendered, out),
        }
    }
    out.push_str("</");
    out.push_str(&element.name);
    out.push('>');
}

/// A whole document in canonical form.
fn canonical_document(xml: &str) -> Result<String, String> {
    let root = parse(xml)?;
    let mut out = String::with_capacity(xml.len());
    canonicalize(&root, &BTreeMap::new(), &BTreeMap::new(), &mut out);
    Ok(out)
}

/// An element of a larger document in canonical form: every namespace in
/// scope is written on it.
fn canonical_subset(element: &Element, scope: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    canonicalize(element, scope, &BTreeMap::new(), &mut out);
    out
}

// ---- digests ----------------------------------------------------------------

fn digest(algorithm: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    match algorithm {
        SHA256 => Ok(sha2::Sha256::digest(data).to_vec()),
        "http://www.w3.org/2000/09/xmldsig#sha1" => Ok(sha1::Sha1::digest(data).to_vec()),
        "http://www.w3.org/2001/04/xmlenc#sha512" => Ok(sha2::Sha512::digest(data).to_vec()),
        "http://www.w3.org/2001/04/xmldsig-more#sha384" => Ok(sha2::Sha384::digest(data).to_vec()),
        other => Err(format!("unsupported digest {other}")),
    }
}

fn base64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn unbase64(text: &str) -> Result<Vec<u8>, String> {
    let compact: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(compact)
        .map_err(|error| error.to_string())
}

/// A relationship part as the relationships transform leaves it: only the
/// relationships it selects (`source_ids`; all of them when empty), sorted
/// by id, each with its target mode spelled out.
fn transformed_relationships(xml: &str, source_ids: &[String]) -> Result<String, String> {
    let root = parse(xml)?;
    let mut selected: Vec<&Element> = root
        .elements()
        .filter(|element| element.local() == "Relationship")
        .filter(|element| {
            source_ids.is_empty()
                || element
                    .attribute("Id")
                    .is_some_and(|id| source_ids.iter().any(|wanted| wanted == id))
        })
        .collect();
    selected.sort_by_key(|element| element.attribute("Id").unwrap_or_default().to_string());
    let mut out = String::from(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for element in selected {
        out.push_str("<Relationship");
        for name in ["Id", "Target", "TargetMode", "Type"] {
            let value = element
                .attribute(name)
                .or((name == "TargetMode").then_some("Internal"));
            if let Some(value) = value {
                out.push(' ');
                out.push_str(name);
                out.push_str("=\"");
                escape_attribute(value, &mut out);
                out.push('"');
            }
        }
        out.push_str("></Relationship>");
    }
    out.push_str("</Relationships>");
    Ok(out)
}

/// The bytes a manifest reference digests: the part through its transforms.
fn transformed_part(package: &OpcPackage, reference: &Element) -> Result<Vec<u8>, String> {
    let uri = reference.attribute("URI").ok_or("a reference has no URI")?;
    let path = uri.split('?').next().unwrap_or(uri).trim_start_matches('/');
    let part = package
        .part(path)
        .ok_or_else(|| format!("the signed part {path} is missing"))?;
    let transforms: Vec<&Element> = reference
        .child("Transforms")
        .map(|transforms| transforms.elements().collect())
        .unwrap_or_default();
    if transforms.is_empty() {
        return Ok(part.to_vec());
    }
    let mut text = String::from_utf8(part.to_vec()).map_err(|_| format!("{path} is not UTF-8"))?;
    for transform in transforms {
        match transform.attribute("Algorithm") {
            Some(RELATIONSHIP_TRANSFORM) => {
                let ids: Vec<String> = transform
                    .elements()
                    .filter(|element| element.local() == "RelationshipReference")
                    .filter_map(|element| element.attribute("SourceId").map(str::to_string))
                    .collect();
                text = transformed_relationships(&text, &ids)?;
            }
            Some(C14N) => text = canonical_document(&text)?,
            other => return Err(format!("unsupported transform {other:?}")),
        }
    }
    Ok(text.into_bytes())
}

fn check_reference_digest(reference: &Element, data: &[u8]) -> Result<(), String> {
    let method = reference
        .child("DigestMethod")
        .and_then(|method| method.attribute("Algorithm"))
        .ok_or("a reference has no digest method")?;
    let expected = unbase64(
        &reference
            .child("DigestValue")
            .ok_or("no digest value")?
            .text(),
    )?;
    if digest(method, data)? == expected {
        Ok(())
    } else {
        Err(format!(
            "{} has changed since it was signed",
            reference.attribute("URI").unwrap_or("a part")
        ))
    }
}

// ---- certificates -------------------------------------------------------------

fn common_name(name: &x509_cert::name::Name) -> Option<String> {
    use der::asn1::{PrintableStringRef, Utf8StringRef};
    const CN: der::asn1::ObjectIdentifier = der::asn1::ObjectIdentifier::new_unwrap("2.5.4.3");
    name.0
        .iter()
        .flat_map(|rdn| rdn.0.iter())
        .find_map(|attribute| {
            if attribute.oid != CN {
                return None;
            }
            let value = &attribute.value;
            value
                .decode_as::<Utf8StringRef<'_>>()
                .map(|text| text.as_str().to_string())
                .or_else(|_| {
                    value
                        .decode_as::<PrintableStringRef<'_>>()
                        .map(|text| text.as_str().to_string())
                })
                .ok()
        })
}

fn verify_rsa(
    certificate: &[u8],
    method: &str,
    data: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    use der::{referenced::OwnedToRef, Decode};
    use ring::signature::{
        UnparsedPublicKey, RSA_PKCS1_1024_8192_SHA1_FOR_LEGACY_USE_ONLY,
        RSA_PKCS1_1024_8192_SHA256_FOR_LEGACY_USE_ONLY,
        RSA_PKCS1_1024_8192_SHA512_FOR_LEGACY_USE_ONLY,
    };
    let certificate =
        x509_cert::Certificate::from_der(certificate).map_err(|error| error.to_string())?;
    let spki = &certificate.tbs_certificate.subject_public_key_info;
    // 従来と同じ RSA 鍵の識別子と NULL パラメーターを検証し、別方式の鍵を受け付けない。
    let key_algorithm = spki.algorithm.owned_to_ref();
    key_algorithm
        .assert_algorithm_oid(der::asn1::ObjectIdentifier::new_unwrap(
            "1.2.840.113549.1.1.1",
        ))
        .map_err(|error| error.to_string())?;
    if key_algorithm
        .parameters_any()
        .map_err(|error| error.to_string())?
        != der::asn1::Null.into()
    {
        return Err("invalid RSA algorithm parameters".to_string());
    }
    let key = spki
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| "invalid RSA public key bit string".to_string())?;
    // 既存の Word 文書との互換性を保つため、従来受け付けていた 1024 bit / SHA-1 も検証する。
    let algorithm = match method {
        RSA_SHA256 => &RSA_PKCS1_1024_8192_SHA256_FOR_LEGACY_USE_ONLY,
        "http://www.w3.org/2000/09/xmldsig#rsa-sha1" => {
            &RSA_PKCS1_1024_8192_SHA1_FOR_LEGACY_USE_ONLY
        }
        "http://www.w3.org/2001/04/xmldsig-more#rsa-sha512" => {
            &RSA_PKCS1_1024_8192_SHA512_FOR_LEGACY_USE_ONLY
        }
        other => return Err(format!("unsupported signature method {other}")),
    };
    UnparsedPublicKey::new(algorithm, key)
        .verify(data, signature)
        .map_err(|_| "the signature does not verify".to_string())
}

// ---- reading ------------------------------------------------------------------

fn relationships_of(xml: &str) -> Vec<(String, String, String)> {
    let Ok(root) = parse(xml) else {
        return Vec::new();
    };
    root.elements()
        .filter(|element| element.local() == "Relationship")
        .map(|element| {
            (
                element.attribute("Id").unwrap_or_default().to_string(),
                element.attribute("Type").unwrap_or_default().to_string(),
                element.attribute("Target").unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn resolve(base_part: &str, target: &str) -> String {
    crate::resolve_target(base_part, target)
}

/// The signature parts the package's origin part names.
fn signature_parts(package: &OpcPackage) -> Vec<String> {
    let Ok(Some(root_rels)) = package.part_str("_rels/.rels") else {
        return Vec::new();
    };
    let Some(origin) = relationships_of(&root_rels)
        .into_iter()
        .find(|(_, kind, _)| kind == ORIGIN_TYPE)
        .map(|(_, _, target)| resolve("", &target))
    else {
        return Vec::new();
    };
    let Ok(Some(origin_rels)) = package.part_str(&rels_part_name(&origin)) else {
        return Vec::new();
    };
    relationships_of(&origin_rels)
        .into_iter()
        .filter(|(_, kind, _)| kind == SIGNATURE_TYPE)
        .map(|(_, _, target)| resolve(&origin, &target))
        .collect()
}

/// Whether the package carries any signature.
pub fn is_signed(package: &[u8]) -> bool {
    OpcPackage::open(package).is_ok_and(|package| !signature_parts(&package).is_empty())
}

/// Every signature the package carries, each checked against the package.
pub fn read_signatures(package: &[u8]) -> Vec<PackageSignature> {
    let Ok(opc) = OpcPackage::open(package) else {
        return Vec::new();
    };
    signature_parts(&opc)
        .into_iter()
        .map(|part| read_signature(&opc, &part))
        .collect()
}

fn read_signature(package: &OpcPackage, part: &str) -> PackageSignature {
    let mut signature = PackageSignature {
        part: part.to_string(),
        signer: None,
        issuer: None,
        signed_at: None,
        comment: None,
        status: SignatureStatus::Invalid("the signature could not be read".into()),
    };
    let Ok(Some(xml)) = package.part_str(part) else {
        return signature;
    };
    let Ok(root) = parse(&xml) else {
        return signature;
    };
    let scope = BTreeMap::new();
    let find = |local_name: &'static str| {
        root.find(&scope, &move |element: &Element| {
            element.local() == local_name
        })
        .map(|(element, _)| element)
    };
    let certificate = find("X509Certificate").and_then(|element| unbase64(&element.text()).ok());
    if let Some(certificate) = certificate.as_deref() {
        use der::Decode;
        if let Ok(parsed) = x509_cert::Certificate::from_der(certificate) {
            signature.signer = common_name(&parsed.tbs_certificate.subject);
            signature.issuer = common_name(&parsed.tbs_certificate.issuer);
        }
    }
    signature.signed_at = find("SigningTime")
        .or_else(|| find("SignatureTime").and_then(|time| time.child("Value")))
        .map(|element| element.text().trim().to_string());
    signature.comment = find("SignatureComments")
        .map(|element| element.text())
        .filter(|text| !text.trim().is_empty());
    signature.status = match check(package, &root, certificate.as_deref()) {
        Ok(()) => SignatureStatus::Valid,
        Err(why) => SignatureStatus::Invalid(why),
    };
    signature
}

fn check(package: &OpcPackage, root: &Element, certificate: Option<&[u8]>) -> Result<(), String> {
    let scope = BTreeMap::new();
    let (signed_info, info_scope) = root
        .find(&scope, &|element| element.local() == "SignedInfo")
        .ok_or("the signature has no SignedInfo")?;
    // Every object SignedInfo references digests as it says.
    for reference in signed_info
        .elements()
        .filter(|element| element.local() == "Reference")
    {
        let uri = reference.attribute("URI").unwrap_or_default();
        let id = uri
            .strip_prefix('#')
            .ok_or("SignedInfo references outside the signature")?;
        let (target, target_scope) = root
            .find(&scope, &|element| element.attribute("Id") == Some(id))
            .ok_or_else(|| format!("#{id} is missing"))?;
        let canonical = canonical_subset(target, &target_scope);
        check_reference_digest(reference, canonical.as_bytes())?;
        // The package object's manifest: every signed part.
        if let Some(manifest) = target.child("Manifest") {
            for part in manifest
                .elements()
                .filter(|element| element.local() == "Reference")
            {
                check_reference_digest(part, &transformed_part(package, part)?)?;
            }
        }
    }
    let method = signed_info
        .child("SignatureMethod")
        .and_then(|method| method.attribute("Algorithm"))
        .ok_or("no signature method")?;
    let value = root
        .find(&scope, &|element| element.local() == "SignatureValue")
        .map(|(element, _)| unbase64(&element.text()))
        .ok_or("no signature value")??;
    let certificate = certificate.ok_or("no certificate")?;
    verify_rsa(
        certificate,
        method,
        canonical_subset(signed_info, &info_scope).as_bytes(),
        &value,
    )
}

// ---- taking signatures out ------------------------------------------------------

/// The package without its signatures: the origin and signature parts,
/// their relationships and content types — what Word does when a signed
/// document is edited and saved.
pub fn strip_signatures(package: &[u8]) -> Result<Vec<u8>, String> {
    let mut opc = OpcPackage::open(package).map_err(|error| error.to_string())?;
    let parts = signature_parts(&opc);
    if parts.is_empty() {
        return Ok(package.to_vec());
    }
    let names: Vec<String> = opc
        .part_names()
        .iter()
        .filter(|name| name.starts_with("_xmlsignatures/"))
        .cloned()
        .collect();
    for name in names {
        opc.remove(&name);
    }
    if let Some(rels) = opc
        .part_str("_rels/.rels")
        .map_err(|error| error.to_string())?
    {
        let kept = remove_elements(&rels, "<Relationship ", |chunk| chunk.contains(ORIGIN_TYPE));
        opc.replace("_rels/.rels", kept.into_bytes());
    }
    if let Some(types) = opc
        .part_str("[Content_Types].xml")
        .map_err(|error| error.to_string())?
    {
        let kept = remove_elements(&types, "<Override ", |chunk| {
            chunk.contains("/_xmlsignatures/")
        });
        let kept = remove_elements(&kept, "<Default ", |chunk| {
            chunk.contains(ORIGIN_CONTENT_TYPE)
        });
        opc.replace("[Content_Types].xml", kept.into_bytes());
    }
    opc.write().map_err(|error| error.to_string())
}

/// `xml` without the elements starting with `open` (empty elements, as
/// relationship and content type entries are) that `drop` picks.
fn remove_elements(xml: &str, open: &str, drop: impl Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(at) = rest.find(open) {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let end = tail.find("/>").map_or(tail.len(), |end| end + 2);
        if !drop(&tail[..end]) {
            out.push_str(&tail[..end]);
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

// ---- signing --------------------------------------------------------------------------

/// The content type of `part`, from `[Content_Types].xml`.
fn content_type(types: &str, part: &str) -> Option<String> {
    let root = parse(types).ok()?;
    let name = format!("/{part}");
    let found = root
        .elements()
        .find(|element| {
            element.local() == "Override"
                && element
                    .attribute("PartName")
                    .is_some_and(|value| value.eq_ignore_ascii_case(&name))
        })
        .or_else(|| {
            let extension = part.rsplit('.').next()?;
            root.elements().find(|element| {
                element.local() == "Default"
                    && element
                        .attribute("Extension")
                        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
            })
        })
        .and_then(|element| element.attribute("ContentType"))
        .map(str::to_string);
    found
}

fn xml_escape(text: &str) -> String {
    let mut out = String::new();
    escape_attribute(text, &mut out);
    out.replace('>', "&gt;")
}

/// The package signed by `signer`, at `signed_at` (`YYYY-MM-DDThh:mm:ssZ`),
/// with `comment` as 署名の目的. Signatures it already has are kept.
pub fn sign_package(
    package: &[u8],
    signer: &dyn Signer,
    signed_at: &str,
    comment: &str,
) -> Result<Vec<u8>, String> {
    use der::Decode;
    let mut opc = OpcPackage::open(package).map_err(|error| error.to_string())?;
    let types = opc
        .part_str("[Content_Types].xml")
        .map_err(|error| error.to_string())?
        .ok_or("the package has no content types")?;
    let existing = signature_parts(&opc);
    let number = (1..)
        .find(|n| !existing.contains(&format!("_xmlsignatures/sig{n}.xml")))
        .expect("a free name");
    let signature_part = format!("_xmlsignatures/sig{number}.xml");

    // The manifest: every part but the signatures and the content types.
    let mut manifest = String::new();
    let mut names: Vec<String> = opc
        .part_names()
        .iter()
        .filter(|name| {
            !name.starts_with("_xmlsignatures/")
                && *name != "[Content_Types].xml"
                && !name.starts_with("docProps/")
        })
        .cloned()
        .collect();
    names.sort();
    for name in &names {
        let content_type = if name.ends_with(".rels") {
            "application/vnd.openxmlformats-package.relationships+xml".to_string()
        } else {
            content_type(&types, name).unwrap_or_else(|| "application/octet-stream".into())
        };
        let uri = format!(
            "/{}?ContentType={}",
            xml_escape(name),
            xml_escape(&content_type)
        );
        let mut reference = String::new();
        if name.ends_with(".rels") {
            let text = opc
                .part_str(name)
                .map_err(|error| error.to_string())?
                .unwrap_or_default();
            reference.push_str("<Transforms><Transform Algorithm=\"");
            reference.push_str(RELATIONSHIP_TRANSFORM);
            reference.push_str("\">");
            for (id, kind, _) in relationships_of(&text) {
                if kind != ORIGIN_TYPE {
                    reference.push_str(&format!(
                        "<mdssi:RelationshipReference xmlns:mdssi=\"http://schemas.openxmlformats.org/package/2006/digital-signature\" SourceId=\"{}\"></mdssi:RelationshipReference>",
                        xml_escape(&id)
                    ));
                }
            }
            reference.push_str("</Transform><Transform Algorithm=\"");
            reference.push_str(C14N);
            reference.push_str("\"></Transform></Transforms>");
        } else if content_type.ends_with("xml") {
            reference.push_str("<Transforms><Transform Algorithm=\"");
            reference.push_str(C14N);
            reference.push_str("\"></Transform></Transforms>");
        }
        // Digested as the reference's own transforms will leave it.
        let parsed = parse(&format!(
            "<Reference xmlns=\"{DSIG}\" URI=\"{uri}\">{reference}</Reference>"
        ))?;
        let data = transformed_part(&opc, &parsed)?;
        manifest.push_str(&format!(
            "<Reference URI=\"{uri}\">{reference}<DigestMethod Algorithm=\"{SHA256}\"></DigestMethod><DigestValue>{}</DigestValue></Reference>",
            base64(&sha2::Sha256::digest(&data))
        ));
    }

    let certificate = signer.certificate();
    let parsed_certificate =
        x509_cert::Certificate::from_der(&certificate).map_err(|error| error.to_string())?;
    let issuer = parsed_certificate.tbs_certificate.issuer.to_string();
    let serial = big_decimal(parsed_certificate.tbs_certificate.serial_number.as_bytes());

    let package_object = format!(
        "<Object Id=\"idPackageObject\"><Manifest>{manifest}</Manifest><SignatureProperties><SignatureProperty Id=\"idSignatureTime\" Target=\"#idPackageSignature\"><mdssi:SignatureTime xmlns:mdssi=\"http://schemas.openxmlformats.org/package/2006/digital-signature\"><mdssi:Format>YYYY-MM-DDThh:mm:ssTZD</mdssi:Format><mdssi:Value>{time}</mdssi:Value></mdssi:SignatureTime></SignatureProperty></SignatureProperties></Object>",
        time = xml_escape(signed_at),
    );
    let office_object = format!(
        "<Object Id=\"idOfficeObject\"><SignatureProperties><SignatureProperty Id=\"idOfficeV1Details\" Target=\"#idPackageSignature\"><SignatureInfoV1 xmlns=\"http://schemas.microsoft.com/office/2006/digsig\"><SetupID></SetupID><SignatureText></SignatureText><SignatureImage></SignatureImage><SignatureComments>{comment}</SignatureComments><WindowsVersion>10.0</WindowsVersion><OfficeVersion>16.0</OfficeVersion><ApplicationVersion>16.0</ApplicationVersion><Monitors>1</Monitors><HorizontalResolution>1920</HorizontalResolution><VerticalResolution>1080</VerticalResolution><ColorDepth>32</ColorDepth><SignatureProviderId>{{00000000-0000-0000-0000-000000000000}}</SignatureProviderId><SignatureProviderUrl></SignatureProviderUrl><SignatureProviderDetails>9</SignatureProviderDetails><SignatureType>1</SignatureType></SignatureInfoV1></SignatureProperty></SignatureProperties></Object>",
        comment = xml_escape(comment),
    );
    let signed_properties = format!(
        "<xd:SignedProperties Id=\"idSignedProperties\"><xd:SignedSignatureProperties><xd:SigningTime>{time}</xd:SigningTime><xd:SigningCertificate><xd:Cert><xd:CertDigest><DigestMethod Algorithm=\"{SHA256}\"></DigestMethod><DigestValue>{cert_digest}</DigestValue></xd:CertDigest><xd:IssuerSerial><X509IssuerName>{issuer}</X509IssuerName><X509SerialNumber>{serial}</X509SerialNumber></xd:IssuerSerial></xd:Cert></xd:SigningCertificate><xd:SignaturePolicyIdentifier><xd:SignaturePolicyImplied></xd:SignaturePolicyImplied></xd:SignaturePolicyIdentifier></xd:SignedSignatureProperties></xd:SignedProperties>",
        time = xml_escape(signed_at),
        cert_digest = base64(&sha2::Sha256::digest(&certificate)),
        issuer = xml_escape(&issuer),
    );
    let xades_object = format!(
        "<Object><xd:QualifyingProperties xmlns:xd=\"http://uri.etsi.org/01903/v1.3.2#\" Target=\"#idPackageSignature\">{signed_properties}</xd:QualifyingProperties></Object>"
    );
    let objects = format!("{package_object}{office_object}{xades_object}");

    // The objects' digests, canonical within the signature.
    let shell =
        format!("<Signature xmlns=\"{DSIG}\" Id=\"idPackageSignature\">{objects}</Signature>");
    let tree = parse(&shell)?;
    let scope = BTreeMap::new();
    let object_digest = |id: &str| -> Result<String, String> {
        let (element, element_scope) = tree
            .find(&scope, &|element| element.attribute("Id") == Some(id))
            .ok_or_else(|| format!("#{id} is missing"))?;
        Ok(base64(&sha2::Sha256::digest(
            canonical_subset(element, &element_scope).as_bytes(),
        )))
    };
    let signed_info = format!(
        "<SignedInfo><CanonicalizationMethod Algorithm=\"{C14N}\"></CanonicalizationMethod><SignatureMethod Algorithm=\"{RSA_SHA256}\"></SignatureMethod><Reference Type=\"http://www.w3.org/2000/09/xmldsig#Object\" URI=\"#idPackageObject\"><DigestMethod Algorithm=\"{SHA256}\"></DigestMethod><DigestValue>{}</DigestValue></Reference><Reference Type=\"http://www.w3.org/2000/09/xmldsig#Object\" URI=\"#idOfficeObject\"><DigestMethod Algorithm=\"{SHA256}\"></DigestMethod><DigestValue>{}</DigestValue></Reference><Reference Type=\"http://uri.etsi.org/01903#SignedProperties\" URI=\"#idSignedProperties\"><Transforms><Transform Algorithm=\"{C14N}\"></Transform></Transforms><DigestMethod Algorithm=\"{SHA256}\"></DigestMethod><DigestValue>{}</DigestValue></Reference></SignedInfo>",
        object_digest("idPackageObject")?,
        object_digest("idOfficeObject")?,
        object_digest("idSignedProperties")?,
    );
    let info_tree = parse(&format!(
        "<Signature xmlns=\"{DSIG}\" Id=\"idPackageSignature\">{signed_info}</Signature>"
    ))?;
    let (info, info_scope) = info_tree
        .find(&scope, &|element| element.local() == "SignedInfo")
        .expect("just written");
    let value = signer.sign_sha256(canonical_subset(info, &info_scope).as_bytes())?;
    let signature = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Signature xmlns=\"{DSIG}\" Id=\"idPackageSignature\">{signed_info}<SignatureValue>{}</SignatureValue><KeyInfo><X509Data><X509Certificate>{}</X509Certificate></X509Data></KeyInfo>{objects}</Signature>",
        base64(&value),
        base64(&certificate),
    );

    // The parts: the signature, the origin with its relationship to it, and
    // the package's relationship to the origin.
    opc.replace(&signature_part, signature.into_bytes());
    let origin = "_xmlsignatures/origin.sigs";
    if opc.part(origin).is_none() {
        opc.replace(origin, Vec::new());
    }
    let origin_rels = rels_part_name(origin);
    let mut rels = opc
        .part_str(&origin_rels)
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| {
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>".to_string()
        });
    let id = format!("rIdSig{number}");
    insert_before_close(
        &mut rels,
        "</Relationships>",
        &format!(
            "<Relationship Id=\"{id}\" Type=\"{SIGNATURE_TYPE}\" Target=\"sig{number}.xml\"/>"
        ),
    );
    opc.replace(&origin_rels, rels.into_bytes());
    if let Some(mut root_rels) = opc
        .part_str("_rels/.rels")
        .map_err(|error| error.to_string())?
    {
        if !root_rels.contains(ORIGIN_TYPE) {
            insert_before_close(
                &mut root_rels,
                "</Relationships>",
                &format!("<Relationship Id=\"rIdOrigin\" Type=\"{ORIGIN_TYPE}\" Target=\"_xmlsignatures/origin.sigs\"/>"),
            );
            opc.replace("_rels/.rels", root_rels.into_bytes());
        }
    }
    let mut types = types;
    if !types.contains("Extension=\"sigs\"") {
        insert_before_close(
            &mut types,
            "</Types>",
            &format!("<Default Extension=\"sigs\" ContentType=\"{ORIGIN_CONTENT_TYPE}\"/>"),
        );
    }
    insert_before_close(
        &mut types,
        "</Types>",
        &format!(
            "<Override PartName=\"/{signature_part}\" ContentType=\"{SIGNATURE_CONTENT_TYPE}\"/>"
        ),
    );
    opc.replace("[Content_Types].xml", types.into_bytes());
    opc.write().map_err(|error| error.to_string())
}

fn insert_before_close(xml: &mut String, close: &str, element: &str) {
    match xml.rfind(close) {
        Some(at) => xml.insert_str(at, element),
        None => xml.push_str(element),
    }
}

/// A big-endian unsigned integer in decimal.
fn big_decimal(bytes: &[u8]) -> String {
    let mut digits: Vec<u8> = vec![0];
    for &byte in bytes {
        let mut carry = u32::from(byte);
        for digit in digits.iter_mut() {
            let value = u32::from(*digit) * 256 + carry;
            *digit = (value % 10) as u8;
            carry = value / 10;
        }
        while carry > 0 {
            digits.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }
    digits
        .iter()
        .rev()
        .map(|digit| char::from(b'0' + digit))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_xml_sorts_attributes_and_writes_namespaces_once() {
        let xml = "<?xml version=\"1.0\"?>\n<a:root xmlns:b=\"urn:b\" xmlns:a=\"urn:a\" z=\"1\" b:y=\"2\" a=\"3\"><a:child xmlns:a=\"urn:a\" t=\"x&#10;y\"/>text &amp; &#x41;\r</a:root>";
        assert_eq!(
            canonical_document(xml).unwrap(),
            "<a:root xmlns:a=\"urn:a\" xmlns:b=\"urn:b\" a=\"3\" z=\"1\" b:y=\"2\"><a:child t=\"x&#xA;y\"></a:child>text &amp; A&#xD;</a:root>"
        );
    }

    #[test]
    fn a_subset_carries_the_namespaces_in_scope() {
        let root =
            parse("<r xmlns=\"urn:d\" xmlns:p=\"urn:p\"><s Id=\"x\"><p:t/></s></r>").unwrap();
        let (element, scope) = root
            .find(&BTreeMap::new(), &|element| {
                element.attribute("Id") == Some("x")
            })
            .unwrap();
        assert_eq!(
            canonical_subset(element, &scope),
            "<s xmlns=\"urn:d\" xmlns:p=\"urn:p\" Id=\"x\"><p:t></p:t></s>"
        );
    }

    #[test]
    fn the_relationships_transform_selects_and_sorts() {
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="t2" Target="b.xml"/><Relationship Id="rId1" Type="t1" Target="http://x" TargetMode="External"/><Relationship Id="rId3" Type="t3" Target="c.xml"/></Relationships>"#;
        assert_eq!(
            transformed_relationships(rels, &["rId2".into(), "rId1".into()]).unwrap(),
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Target=\"http://x\" TargetMode=\"External\" Type=\"t1\"></Relationship><Relationship Id=\"rId2\" Target=\"b.xml\" TargetMode=\"Internal\" Type=\"t2\"></Relationship></Relationships>"
        );
    }

    /// A key and a self-signed certificate for it.
    struct TestSigner {
        key: rsa::RsaPrivateKey,
        certificate: Vec<u8>,
    }

    impl TestSigner {
        fn new(name: &str) -> Self {
            use der::Encode;
            use rsa::pkcs8::EncodePublicKey;
            use std::str::FromStr;
            use x509_cert::builder::{Builder, CertificateBuilder, Profile};
            let mut rng = rand::thread_rng();
            let key = rsa::RsaPrivateKey::new(&mut rng, 1024).unwrap();
            let spki_der = key.to_public_key().to_public_key_der().unwrap();
            let spki =
                x509_cert::spki::SubjectPublicKeyInfoOwned::try_from(spki_der.as_bytes()).unwrap();
            let subject = x509_cert::name::Name::from_str(&format!("CN={name},O=Jade")).unwrap();
            let signing_key = rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new(key.clone());
            let builder = CertificateBuilder::new(
                Profile::Root,
                x509_cert::serial_number::SerialNumber::from(4242u32),
                x509_cert::time::Validity::from_now(std::time::Duration::from_secs(3600)).unwrap(),
                subject,
                spki,
                &signing_key,
            )
            .unwrap();
            let certificate = builder.build::<rsa::pkcs1v15::Signature>().unwrap();
            Self {
                key,
                certificate: certificate.to_der().unwrap(),
            }
        }
    }

    impl Signer for TestSigner {
        fn certificate(&self) -> Vec<u8> {
            self.certificate.clone()
        }
        fn sign_sha256(&self, data: &[u8]) -> Result<Vec<u8>, String> {
            self.key
                .sign(
                    rsa::Pkcs1v15Sign::new::<sha2::Sha256>(),
                    &sha2::Sha256::digest(data),
                )
                .map_err(|error| error.to_string())
        }
    }

    #[test]
    fn rsa_verification_preserves_supported_hashes_and_rejects_tampering() {
        let signer = TestSigner::new("Compatibility");
        let data = b"signed package properties";
        let cases = [
            (
                RSA_SHA256,
                rsa::Pkcs1v15Sign::new::<sha2::Sha256>(),
                sha2::Sha256::digest(data).to_vec(),
            ),
            (
                "http://www.w3.org/2000/09/xmldsig#rsa-sha1",
                rsa::Pkcs1v15Sign::new::<sha1::Sha1>(),
                sha1::Sha1::digest(data).to_vec(),
            ),
            (
                "http://www.w3.org/2001/04/xmldsig-more#rsa-sha512",
                rsa::Pkcs1v15Sign::new::<sha2::Sha512>(),
                sha2::Sha512::digest(data).to_vec(),
            ),
        ];
        for (method, padding, digest) in cases {
            let signature = signer.key.sign(padding, &digest).unwrap();
            assert!(verify_rsa(&signer.certificate, method, data, &signature).is_ok());
            assert!(verify_rsa(&signer.certificate, method, b"changed", &signature).is_err());
            let mut damaged = signature;
            damaged[0] ^= 1;
            assert!(verify_rsa(&signer.certificate, method, data, &damaged).is_err());
        }
        assert!(verify_rsa(&signer.certificate, "unsupported", data, &[]).is_err());
        assert!(verify_rsa(b"invalid certificate", RSA_SHA256, data, &[]).is_err());
        use der::{Decode, Encode};
        let mut certificate = x509_cert::Certificate::from_der(&signer.certificate).unwrap();
        certificate
            .tbs_certificate
            .subject_public_key_info
            .algorithm
            .parameters = None;
        assert!(
            verify_rsa(&certificate.to_der().unwrap(), RSA_SHA256, data, &[])
                .unwrap_err()
                .contains("parameters")
        );
        certificate
            .tbs_certificate
            .subject_public_key_info
            .algorithm
            .oid = der::asn1::ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
        assert!(verify_rsa(&certificate.to_der().unwrap(), RSA_SHA256, data, &[]).is_err());
    }

    fn package(document: &str) -> Vec<u8> {
        use std::io::Write;
        let parts = [
            ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_string()),
            ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string()),
            ("word/document.xml", document.to_string()),
            ("word/_rels/document.xml.rels", r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/a.png"/></Relationships>"#.to_string()),
            ("word/media/a.png", "\u{89}PNG fake".to_string()),
        ];
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, content) in parts {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    const BODY: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t xml:space="preserve">Signed &amp; sealed</w:t></w:r></w:p></w:body></w:document>"#;

    #[test]
    fn a_signed_package_verifies_until_a_part_changes() {
        let signer = TestSigner::new("Ann Lee");
        let unsigned = package(BODY);
        assert!(!is_signed(&unsigned));
        let signed = sign_package(&unsigned, &signer, "2026-03-04T05:06:07Z", "承認").unwrap();
        assert!(is_signed(&signed));
        let signatures = read_signatures(&signed);
        assert_eq!(signatures.len(), 1);
        let signature = &signatures[0];
        assert_eq!(signature.status, SignatureStatus::Valid);
        assert_eq!(signature.signer.as_deref(), Some("Ann Lee"));
        assert_eq!(signature.signed_at.as_deref(), Some("2026-03-04T05:06:07Z"));
        assert_eq!(signature.comment.as_deref(), Some("承認"));

        // A second signature beside the first: both hold.
        let twice =
            sign_package(&signed, &TestSigner::new("Bob"), "2026-03-05T00:00:00Z", "").unwrap();
        let both = read_signatures(&twice);
        assert_eq!(both.len(), 2);
        assert!(both
            .iter()
            .all(|signature| signature.status == SignatureStatus::Valid));

        // The document changed after signing: the signature no longer holds.
        let mut opc = OpcPackage::open(&signed).unwrap();
        opc.replace(
            "word/document.xml",
            BODY.replace("sealed", "changed").into_bytes(),
        );
        let changed = opc.write().unwrap();
        assert!(matches!(
            read_signatures(&changed)[0].status,
            SignatureStatus::Invalid(_)
        ));

        // Taken out: no signature, and the document as it was.
        let stripped = strip_signatures(&signed).unwrap();
        assert!(!is_signed(&stripped));
        let opc = OpcPackage::open(&stripped).unwrap();
        assert!(opc
            .part_names()
            .iter()
            .all(|name| !name.starts_with("_xmlsignatures/")));
        assert_eq!(opc.part("word/document.xml").unwrap(), BODY.as_bytes());
        assert!(!opc
            .part_str("[Content_Types].xml")
            .unwrap()
            .unwrap()
            .contains("xmlsignature"));
    }

    #[test]
    fn a_tampered_signature_value_does_not_verify() {
        let signer = TestSigner::new("Ann");
        let signed = sign_package(&package(BODY), &signer, "2026-03-04T05:06:07Z", "").unwrap();
        let mut opc = OpcPackage::open(&signed).unwrap();
        let xml = opc.part_str("_xmlsignatures/sig1.xml").unwrap().unwrap();
        let start = xml.find("<SignatureValue>").unwrap() + "<SignatureValue>".len();
        let mut tampered = xml.clone();
        let flipped = if &xml[start..start + 1] == "A" {
            "B"
        } else {
            "A"
        };
        tampered.replace_range(start..start + 1, flipped);
        opc.replace("_xmlsignatures/sig1.xml", tampered.into_bytes());
        let signature = &read_signatures(&opc.write().unwrap())[0];
        assert_eq!(
            signature.status,
            SignatureStatus::Invalid("the signature does not verify".into())
        );
    }

    #[test]
    fn serial_numbers_read_in_decimal() {
        assert_eq!(big_decimal(&[0x01, 0x00]), "256");
        assert_eq!(
            big_decimal(&[0xff, 0xff, 0xff, 0xff, 0xff]),
            "1099511627775"
        );
        assert_eq!(big_decimal(&[0]), "0");
    }
}
