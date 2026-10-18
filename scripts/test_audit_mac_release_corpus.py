"""Focused checks for the release corpus audit's failure conditions."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
import warnings
from zipfile import ZipFile


SCRIPT = Path(__file__).with_name("audit-mac-release-corpus.py")
SPEC = importlib.util.spec_from_file_location("audit_mac_release_corpus", SCRIPT)
assert SPEC and SPEC.loader
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


class CorpusAuditTests(unittest.TestCase):
    def test_structural_sequence_detects_reordered_elements(self) -> None:
        before = b'<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:bookmarkStart w:id="1"/><w:bookmarkEnd w:id="1"/></w:document>'
        after = b'<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:bookmarkEnd w:id="1"/><w:bookmarkStart w:id="1"/></w:document>'
        self.assertNotEqual(audit.structural_sequence(before), audit.structural_sequence(after))

    def test_run_property_order_is_ignored_only_in_order_check(self) -> None:
        before = b'<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:rPr><w:rFonts w:ascii="A"/><w:sz w:val="12"/></w:rPr></w:document>'
        after = b'<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:rPr><w:sz w:val="12"/><w:rFonts w:ascii="A"/></w:rPr></w:document>'
        self.assertNotEqual(audit.structural_sequence(before), audit.structural_sequence(after))
        self.assertEqual(audit.structural_sequence(before, False), audit.structural_sequence(after, False))

    def test_property_order_is_ignored_but_property_changes_are_not(self) -> None:
        def document(paragraph_properties: str) -> bytes:
            return (
                '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
                f"<w:p><w:pPr>{paragraph_properties}</w:pPr></w:p></w:document>"
            ).encode()

        before = document('<w:jc w:val="center"/><w:spacing w:after="0"/>')
        moved = document('<w:spacing w:after="0"/><w:jc w:val="center"/>')
        changed = document('<w:spacing w:after="120"/><w:jc w:val="center"/>')
        self.assertEqual(audit.structural_sequence(before, False), audit.structural_sequence(moved, False))
        self.assertNotEqual(audit.structural_sequence(before, False), audit.structural_sequence(changed, False))

    def test_paragraph_order_is_still_checked(self) -> None:
        def document(*texts: str) -> bytes:
            body = "".join(f'<w:p><w:pPr><w:pStyle w:val="{text}"/></w:pPr></w:p>' for text in texts)
            return (
                '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
                f"<w:body>{body}</w:body></w:document>"
            ).encode()

        self.assertNotEqual(
            audit.structural_sequence(document("A", "B"), False),
            audit.structural_sequence(document("B", "A"), False),
        )

    def test_text_typed_beside_an_object_may_take_its_own_run(self) -> None:
        def document(runs: str) -> bytes:
            return (
                '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
                f"<w:body><w:p>{runs}</w:p></w:body></w:document>"
            ).encode()

        properties = '<w:rPr><w:noProof/><w:kern w:val="0"/></w:rPr>'
        text_box = f'<w:r>{properties}<w:pict/></w:r>'
        before = document(text_box)
        typed = document(text_box + f'<w:r>{properties}<w:t>Z</w:t></w:r>')
        self.assertNotEqual(audit.structural_sequence(before), audit.structural_sequence(typed))
        self.assertEqual(
            audit.structural_sequence(before),
            audit.structural_sequence(audit.without_run_typed_beside_object(typed, "Z")),
        )

    def test_a_new_run_beside_text_is_still_a_structural_change(self) -> None:
        def document(runs: str) -> bytes:
            return (
                '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
                f"<w:body><w:p>{runs}</w:p></w:body></w:document>"
            ).encode()

        properties = '<w:rPr><w:b/></w:rPr>'
        before = document(f'<w:r>{properties}<w:t>A</w:t></w:r>')
        split = document(f'<w:r>{properties}<w:t>A</w:t></w:r><w:r>{properties}<w:t>Z</w:t></w:r>')
        self.assertNotEqual(
            audit.structural_sequence(before),
            audit.structural_sequence(audit.without_run_typed_beside_object(split, "Z")),
        )

    def test_repeated_revision_ids_may_be_renumbered_but_unique_ones_must_stay(self) -> None:
        def document(*ids: str) -> bytes:
            body = "".join(f'<w:ins w:id="{value}" w:author="A"/>' for value in ids)
            return (
                '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
                f"<w:p>{body}</w:p></w:document>"
            ).encode()

        self.assertEqual(audit.revision_id_errors(document("0", "0", "5"), document("0", "6", "5")), [])
        self.assertEqual(
            audit.revision_id_errors(document("4", "5"), document("4", "9")),
            ["changed revision IDs"],
        )
        self.assertEqual(
            audit.revision_id_errors(document("0", "0"), document("0", "0")),
            ["repeated revision IDs"],
        )

    def test_shrunken_fixture_corpus_fails_before_export(self) -> None:
        original = audit.FIXTURES
        try:
            with tempfile.TemporaryDirectory() as directory:
                audit.FIXTURES = Path(directory)
                with self.assertRaisesRegex(SystemExit, "fixture corpus shrank"):
                    audit.main()
        finally:
            audit.FIXTURES = original

    def test_missing_internal_relationship_target_is_reported(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            package_path = Path(directory) / "edited.docx"
            with ZipFile(package_path, "w") as package:
                package.writestr("word/document.xml", "<document/>")
                package.writestr(
                    "word/_rels/document.xml.rels",
                    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
                    '<Relationship Id="rId1" Target="media/missing.png"/>'
                    '<Relationship Id="rId2" Target="https://example.com" TargetMode="External"/>'
                    '</Relationships>',
                )
            with ZipFile(package_path) as package:
                self.assertEqual(
                    audit.missing_relationship_targets(package),
                    ["word/_rels/document.xml.rels: missing internal target media/missing.png (word/media/missing.png)"],
                )

    def test_missing_content_type_is_reported_in_source_and_export(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.docx"
            edited = Path(directory) / "edited.docx"
            declared = (
                '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
                '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
                '<Override PartName="/word/styles.xml" ContentType="application/xml"/>'
                '</Types>'
            )
            lost = declared.replace('<Override PartName="/word/styles.xml" ContentType="application/xml"/>', "")
            for path, content_types in ((source, declared), (edited, lost)):
                with ZipFile(path, "w") as package:
                    package.writestr("[Content_Types].xml", content_types)
                    package.writestr("word/styles.xml", "<styles/>")
            with ZipFile(source) as original, ZipFile(edited) as exported:
                self.assertEqual(audit.parts_without_content_type(original), set())
                self.assertEqual(audit.parts_without_content_type(exported), {"word/styles.xml"})
            self.assertIn("edited parts without content types", audit.audit(source, edited)[0])
            self.assertIn("source parts without content types", audit.audit(edited, source)[0])

    def test_duplicate_part_is_reported_before_comparing_content(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "duplicate.docx"
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", UserWarning)
                with ZipFile(path, "w") as package:
                    package.writestr("word/document.xml", "<document/>")
                    package.writestr("word/document.xml", "<different/>")
            self.assertIn("source duplicate parts", audit.audit(path, path)[0])

    def test_malformed_xml_is_reported_before_parsing_content_types(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "malformed.docx"
            with ZipFile(path, "w") as package:
                package.writestr("[Content_Types].xml", "<Types>")
            self.assertIn("source invalid XML in [Content_Types].xml", audit.audit(path, path)[0])


if __name__ == "__main__":
    unittest.main()
