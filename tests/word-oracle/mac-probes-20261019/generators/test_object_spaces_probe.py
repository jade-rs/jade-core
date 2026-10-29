"""Validate probe inputs; Word-exported PDFs remain the layout oracle."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from xml.etree import ElementTree as ET
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
WP = "{http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing}"


def xml(path, part):
    with zipfile.ZipFile(path) as archive:
        return ET.fromstring(archive.read(part))


class ObjectSpaceProbe(unittest.TestCase):
    def generate(self, out, *args):
        subprocess.run([sys.executable, str(HERE / "make-object-spaces-probe.py"),
                        str(ROOT / "public/fixtures/image-inline.docx"), out, *args], check=True)

    def test_default_keeps_the_six_observed_paragraphs(self):
        with tempfile.TemporaryDirectory() as out:
            self.generate(out)
            doc = xml(Path(out) / "word-mac-probe-object-spaces.docx", "word/document.xml")
            paragraphs = doc.find(W + "body").findall(W + "p")
            self.assertEqual(len(paragraphs), 6)
            self.assertEqual(["".join(p.itertext()) for p in paragraphs[:3]],
                             ["Hello World", "Hello  World", "Hello World "])
            self.assertEqual(len(doc.findall(".//" + WP + "inline")), 5)
            self.assertEqual(len(doc.findall(".//" + WP + "anchor")), 1)

    def test_space_run_pair_has_identical_content_and_opposite_balance_setting(self):
        with tempfile.TemporaryDirectory() as out:
            self.generate(out, "--space-runs")
            out = Path(out)
            manifest = json.loads((out / "word-mac-probe-object-space-runs.cases.json").read_text())
            self.assertEqual(len(manifest["paragraphs"]), 14)
            self.assertEqual([case["text"] for case in manifest["paragraphs"][6:]], [
                "Hello  \ufffcWorld", "Hello   \ufffcWorld", "Hello\ufffc  World",
                "Hello\ufffc   World", "Hello   \ufffcWorld", "Hello\ufffc   World",
                "Hello   \ufffcWorld", "Hello   World",
            ])
            documents = []
            for variant, balance in [("balanced", True), ("unbalanced", False)]:
                path = out / f"word-mac-probe-object-space-runs-{variant}.docx"
                doc = xml(path, "word/document.xml")
                settings = xml(path, "word/settings.xml")
                self.assertEqual(settings.find(".//" + W + "balanceSingleByteDoubleByteWidth") is not None, balance)
                paragraphs = doc.find(W + "body").findall(W + "p")
                self.assertEqual(len(paragraphs), 14)
                for paragraph, case in zip(paragraphs, manifest["paragraphs"]):
                    tokens = []
                    for run in paragraph.findall(W + "r"):
                        for child in run:
                            if child.tag == W + "t":
                                self.assertEqual(child.get("{http://www.w3.org/XML/1998/namespace}space"), "preserve")
                                tokens.append(child.text or "")
                            elif child.tag == W + "drawing":
                                tokens.append("\ufffc")
                    self.assertEqual("".join(tokens), case["text"], case["id"])
                # Drawing IDs must be unique so Word need not repair the probe.
                ids = [p.get("id") for p in doc.findall(".//" + WP + "docPr")]
                self.assertEqual(len(ids), len(set(ids)))
                for extent in doc.findall(".//" + WP + "extent"):
                    self.assertEqual((extent.get("cx"), extent.get("cy")), ("254000", "254000"))
                documents.append(ET.tostring(doc))
            self.assertEqual(*documents)


if __name__ == "__main__":
    unittest.main()
