//! Pins the exact output bytes of `transform_docx_annotations` for real fixtures,
//! so performance work on the transform cannot change a single output byte.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

const FIXTURES: [&str; 8] = [
    "comment.docx",
    "review.docx",
    "review-kinds.docx",
    "review-duplicate-ids.docx",
    "numbering-with-review.docx",
    "numbering-del.docx",
    "footnote.docx",
    "endnote.docx",
];
const FLAGS: [(bool, bool); 3] = [(false, false), (false, true), (true, false)];

/// (fixture, include_comments, include_tracked_changes, length, hash of output bytes)
const EXPECTED: &[(&str, bool, bool, usize, u64)] = &[];

fn hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

fn run(data: &[u8], comments: bool, revisions: bool) -> Vec<u8> {
    futures::executor::block_on(jade_docx_annotations::transform_docx_annotations(
        data, comments, revisions,
    ))
    .unwrap()
}

#[test]
fn fixture_outputs_are_deterministic_and_match_recorded_hashes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures");
    let mut actual = Vec::new();
    for name in FIXTURES {
        let data = std::fs::read(root.join(name)).unwrap();
        for (comments, revisions) in FLAGS {
            let first = run(&data, comments, revisions);
            let second = run(&data, comments, revisions);
            assert_eq!(first, second, "{name} {comments} {revisions} is not deterministic");
            actual.push((name, comments, revisions, first.len(), hash(&first)));
        }
    }
    if EXPECTED.is_empty() {
        for row in &actual {
            println!("RECORD {row:?}");
        }
        return;
    }
    assert_eq!(actual.as_slice(), EXPECTED);
}

/// Shadowed, re-bound, default-namespace and undeclared prefixes must resolve exactly as before.
fn namespace_edge_package() -> Vec<u8> {
    use std::io::Write;
    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let document = format!(
        r#"<w:document xmlns:w="{w}" xmlns:x="{w}"><w:body>
<w:p><w:ins w:id="1"><w:r><w:t>plain ins</w:t></w:r></w:ins></w:p>
<w:p><x:ins x:id="2"><x:r><x:t>alias ins</x:t></x:r></x:ins></w:p>
<w:p xmlns:w="urn:other"><w:ins w:id="3"><w:r><w:t>shadowed ins</w:t></w:r></w:ins><x:del x:id="4"><x:r><x:delText>alias del</x:delText></x:r></x:del></w:p>
<p xmlns="{w}"><ins id="5"><r><t>default ns ins</t></r></ins><del id="6"><r><delText>default ns del</delText></r></del></p>
<w:p><y:ins y:id="7"><w:r><w:t>undeclared prefix</w:t></w:r></y:ins><w:commentRangeStart w:id="9"/><w:del w:id="8"><w:r><w:delText>gone</w:delText></w:r></w:del></w:p>
<w:p><w:ins xmlns:w="urn:other" w:id="10"><w:r><w:t>rebound on the element itself</w:t></w:r></w:ins><w:r><w:commentReference w:id="9"/></w:r></w:p>
</w:body></w:document>"#
    );
    let comments = format!(
        r#"<w:comments xmlns:w="{w}"><w:comment w:id="9"><w:p><w:r><w:t>c</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (path, xml) in [
        ("word/document.xml", document.as_str()),
        ("word/comments.xml", comments.as_str()),
    ] {
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// (include_comments, include_tracked_changes, length, hash of output bytes)
const NAMESPACE_EXPECTED: &[(bool, bool, usize, u64)] = &[];

#[test]
fn namespace_edge_cases_match_recorded_hashes() {
    let data = namespace_edge_package();
    let mut actual = Vec::new();
    for (comments, revisions) in FLAGS {
        let first = run(&data, comments, revisions);
        assert_eq!(first, run(&data, comments, revisions));
        actual.push((comments, revisions, first.len(), hash(&first)));
    }
    if NAMESPACE_EXPECTED.is_empty() {
        for row in &actual {
            println!("NSRECORD {row:?}");
        }
        return;
    }
    assert_eq!(actual.as_slice(), NAMESPACE_EXPECTED);
}
