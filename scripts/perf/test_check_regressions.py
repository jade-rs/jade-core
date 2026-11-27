import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("check_regressions.py")
spec = importlib.util.spec_from_file_location("check_regressions", SCRIPT)
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class RegressionGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.base, self.head = self.root / "base", self.root / "head"
        for folder in (self.base, self.head):
            folder.mkdir()
            for fixture in ("contract.docx", "nda.docx", "yokohama.docx"):
                for scenario in ("load", "layout", "repaint", "keystroke", "keystroke-marks", "export"):
                    self.write(folder, f"{scenario}@{fixture}.out", 200_000)

    def write(self, folder, name, count):
        (folder / name).write_text(f"events: Ir\nsummary: {count}\ntotals: {count}\n")

    def test_full_matrix_and_exact_positive_boundary_pass(self):
        self.write(self.head, "repaint@contract.docx.out", 201_000)
        result = gate.evaluate(self.base, self.head)
        self.assertTrue(result["passed"])
        self.assertEqual(len(result["cases"]), 18)

    def test_one_instruction_past_boundary_fails_without_rounding(self):
        self.write(self.head, "repaint@contract.docx.out", 201_001)
        self.assertFalse(gate.evaluate(self.base, self.head)["passed"])

    def test_large_improvements_cannot_cancel_one_regression(self):
        for path in self.head.glob("*.out"):
            self.write(self.head, path.name, 1)
        self.write(self.head, "repaint@contract.docx.out", 201_001)
        self.assertFalse(gate.evaluate(self.base, self.head)["passed"])

    def test_every_individual_case_is_required_on_both_sides(self):
        for folder in (self.base, self.head):
            for path in sorted(folder.glob("*.out")):
                with self.subTest(folder=folder.name, case=path.name):
                    original = path.read_text()
                    path.unlink()
                    self.assertFalse(gate.evaluate(self.base, self.head)["passed"])
                    path.write_text(original)

    def test_shared_missing_case_and_empty_base_fail(self):
        for folder in (self.base, self.head):
            (folder / "export@nda.docx.out").unlink()
        self.assertFalse(gate.evaluate(self.base, self.head)["passed"])
        for path in self.base.glob("*.out"):
            path.unlink()
        self.assertFalse(gate.evaluate(self.base, self.head)["passed"])

    def test_unexpected_cases_fail(self):
        self.write(self.head, "other@contract.docx.out", 1)
        self.assertFalse(gate.evaluate(self.base, self.head)["passed"])

    def test_invalid_or_ambiguous_counts_fail_closed(self):
        for text in ("", "events: Dr\nsummary: 1\ntotals: 1\n",
                     "events: Ir\nsummary: 0\ntotals: 0\n",
                     "events: Ir\nsummary: -1\ntotals: -1\n",
                     "events: Ir\nsummary: 100 200\ntotals: 100\n",
                     "events: Ir\nsummary: 100\ntotals: 101\n",
                     "events: Ir\nsummary: 100\nsummary: 100\ntotals: 100\n",
                     "events: Ir\nsummary: 100\n",
                     "events: Ir\nevents: Ir\nsummary: 100\ntotals: 100\n"):
            for folder in (self.base, self.head):
                with self.subTest(text=text, folder=folder.name):
                    path = folder / "repaint@contract.docx.out"
                    original = path.read_text()
                    path.write_text(text)
                    self.assertFalse(gate.evaluate(self.base, self.head)["passed"])
                    path.write_text(original)

    def test_cli_returns_failure_and_preserves_machine_readable_evidence(self):
        self.write(self.base, "repaint@contract.docx.out", 664_740)
        self.write(self.head, "repaint@contract.docx.out", 668_263)
        report = self.root / "gate.json"
        result = subprocess.run([sys.executable, str(SCRIPT), str(self.base), str(self.head),
                                 "--output", str(report)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("repaint@contract.docx", result.stdout)
        saved = json.loads(report.read_text())
        self.assertFalse(saved["passed"])
        case = next(c for c in saved["cases"] if c["case"] == "repaint@contract.docx")
        self.assertEqual((case["baseIr"], case["headIr"]), (664_740, 668_263))


class MeasurementBoundaryTests(unittest.TestCase):
    @staticmethod
    def check_symbols(symbols=None, error=None):
        from unittest.mock import patch
        shell = Path(__file__).with_name("run-callgrind.sh").read_text(encoding="utf8")
        code = shell.split("<<'PY_SYMBOL'\n", 1)[1].split("\nPY_SYMBOL\n", 1)[0]
        with patch("subprocess.check_output", return_value=symbols, side_effect=error), \
                patch.object(sys, "argv", ["boundary-check", "perf-binary"]):
            exec(compile(code, "run-callgrind.sh:boundary", "exec"), {})

    def test_accepts_one_defined_measurement_function(self):
        self.check_symbols("00000010 T jade_docx_perf_measured_region\n")

    def test_rejects_missing_duplicate_or_non_function_boundaries(self):
        for symbols in ("", "00000010 T other_function\n",
                        "00000010 D jade_docx_perf_measured_region\n",
                        "         U jade_docx_perf_measured_region\n",
                        "00000010 T jade_docx_perf_measured_region\n"
                        "00000020 t jade_docx_perf_measured_region\n"):
            with self.subTest(symbols=symbols), self.assertRaisesRegex(
                    SystemExit, "Missing unique Callgrind measurement boundary"):
                self.check_symbols(symbols)

    def test_symbol_tool_failure_stays_a_failure(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.check_symbols(error=subprocess.CalledProcessError(1, ["nm"]))


if __name__ == "__main__":
    unittest.main()
