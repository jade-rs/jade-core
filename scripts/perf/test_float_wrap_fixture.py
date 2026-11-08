import tempfile
import unittest
from pathlib import Path
from xml.dom import minidom
from zipfile import ZipFile

from float_wrap_fixture import R, W, WP, write_fixture

FIXTURES = Path(__file__).resolve().parents[2] / "public" / "fixtures"


class FloatWrapFixtureTests(unittest.TestCase):
    def test_late_shape_preserves_original_parts_and_body(self):
        base = FIXTURES / "yokohama.docx"
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "float.docx"
            repeat = Path(directory) / "repeat.docx"
            shape = FIXTURES / "floating-square-wrap.docx"
            write_fixture(base, shape, output)
            write_fixture(base, shape, repeat)
            self.assertEqual(output.read_bytes(), repeat.read_bytes())
            with ZipFile(base) as original, ZipFile(output) as generated:
                self.assertEqual(original.namelist(), generated.namelist())
                for name in original.namelist():
                    if name != "word/document.xml":
                        self.assertEqual(original.read(name), generated.read(name), name)
                old = minidom.parseString(original.read("word/document.xml"))
                new = minidom.parseString(generated.read("word/document.xml"))
            old_body = old.getElementsByTagNameNS(W, "body")[0]
            new_body = new.getElementsByTagNameNS(W, "body")[0]
            children = lambda body: [n for n in body.childNodes if n.nodeType == n.ELEMENT_NODE]
            before, after = children(old_body), children(new_body)
            self.assertGreater(len(before), 2000)
            self.assertEqual(len(after), len(before) + 1)
            self.assertEqual(after[-1].localName, "sectPr")
            self.assertEqual([n.toxml() for n in before], [n.toxml() for n in after[:-2] + after[-1:]])
            anchors = after[-2].getElementsByTagNameNS(WP, "anchor")
            self.assertEqual(len(anchors), 1)
            self.assertEqual(len(anchors[0].getElementsByTagNameNS(WP, "wrapSquare")), 1)

    def test_relationship_dependent_shapes_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            shape = Path(directory) / "shape.docx"
            with ZipFile(FIXTURES / "floating-square-wrap.docx") as source:
                document = minidom.parseString(source.read("word/document.xml"))
            document.getElementsByTagNameNS(WP, "anchor")[0].setAttributeNS(R, "r:embed", "rId999")
            with ZipFile(shape, "w") as target:
                target.writestr("word/document.xml", document.toxml(encoding="utf-8"))
            output = Path(directory) / "output.docx"
            with self.assertRaisesRegex(ValueError, "relationships"):
                write_fixture(FIXTURES / "yokohama.docx", shape, output)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
