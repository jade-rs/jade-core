use jade_opc::OpcPackage;
use std::io::{Cursor, Write};

fn package() -> OpcPackage {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in [
        ("normal.xml", &b"<a>caf\xc3\xa9</a>"[..]),
        ("bom.xml", &b"\xef\xbb\xbf<a/>"[..]),
        ("bad.xml", &b"<a>\xff\xfe</a>"[..]),
        ("empty.xml", &b""[..]),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    OpcPackage::open(&writer.finish().unwrap().into_inner()).unwrap()
}

fn same(package: &OpcPackage, name: &str) {
    let borrowed = package.part_text(name).map(|text| text.map(str::to_owned));
    let owned = package.part_str(name);
    match (borrowed, owned) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "{name}"),
        (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string(), "{name}"),
        (a, b) => panic!("{name}: {a:?} vs {b:?}"),
    }
    let borrowed = package
        .required_part_text(name)
        .map(str::to_owned)
        .map_err(|e| e.to_string());
    let owned = package.required_part_str(name).map_err(|e| e.to_string());
    assert_eq!(borrowed, owned, "required {name}");
}

#[test]
fn borrowed_and_owned_accessors_agree() {
    let package = package();
    for name in [
        "normal.xml",
        "bom.xml",
        "bad.xml",
        "empty.xml",
        "missing.xml",
    ] {
        same(&package, name);
    }
    assert_eq!(
        package.part_text("normal.xml").unwrap(),
        Some("<a>café</a>")
    );
    assert_eq!(package.part_text("missing.xml").unwrap(), None);
    assert!(package.part_text("bad.xml").is_err());
    assert!(package.required_part_text("missing.xml").is_err());
}

#[test]
fn a_leading_bom_is_kept_by_both_accessors() {
    let package = package();
    assert_eq!(package.part_text("bom.xml").unwrap(), Some("\u{feff}<a/>"));
    assert_eq!(
        package.part_str("bom.xml").unwrap().as_deref(),
        Some("\u{feff}<a/>")
    );
}
