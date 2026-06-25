"""Tests for the cross-platform result diff tool.

Run from the repository root:
    python3 -m unittest tests.tooling.test_cross_platform_diff
"""

import os
import sys
import json
import unittest

# Make the tool importable without a package install.
_TOOL_DIR = os.path.join(
    os.path.dirname(__file__), "..", "..", "tools", "cross-platform-diff"
)
sys.path.insert(0, _TOOL_DIR)

import cross_platform_diff as cpd  # noqa: E402


class DiffValuesScalarTests(unittest.TestCase):
    def test_equal_scalars_produce_no_diff(self):
        self.assertEqual(cpd.diff_values(1, 1), [])
        self.assertEqual(cpd.diff_values("a", "a"), [])
        self.assertEqual(cpd.diff_values(True, True), [])
        self.assertEqual(cpd.diff_values(None, None), [])
        self.assertEqual(cpd.diff_values(1.5, 1.5), [])

    def test_changed_scalar_same_type_different_value(self):
        entries = cpd.diff_values("a", "b")
        self.assertEqual(len(entries), 1)
        e = entries[0]
        self.assertEqual(e["category"], "changed")
        self.assertEqual(e["path"], "$")
        self.assertEqual(e["reference_value"], "a")
        self.assertEqual(e["candidate_value"], "b")

    def test_changed_number_different_value(self):
        entries = cpd.diff_values(2024, 2025)
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["category"], "changed")

    def test_type_mismatch_number_vs_string(self):
        entries = cpd.diff_values(2024, "2024")
        self.assertEqual(len(entries), 1)
        e = entries[0]
        self.assertEqual(e["category"], "type_mismatch")
        self.assertEqual(e["reference_type"], "number")
        self.assertEqual(e["candidate_type"], "string")
        self.assertEqual(e["reference_value"], 2024)
        self.assertEqual(e["candidate_value"], "2024")

    def test_type_mismatch_bool_vs_number(self):
        # bool and number are distinct JSON types; must NOT be "changed".
        entries = cpd.diff_values(1, True)
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["category"], "type_mismatch")
        self.assertEqual(entries[0]["reference_type"], "number")
        self.assertEqual(entries[0]["candidate_type"], "boolean")

    def test_type_mismatch_object_vs_array(self):
        entries = cpd.diff_values({"a": 1}, [1, 2])
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["category"], "type_mismatch")
        self.assertEqual(entries[0]["reference_type"], "object")
        self.assertEqual(entries[0]["candidate_type"], "array")

    def test_null_is_a_value_not_missing(self):
        # null vs null is equal; null vs a value is a type mismatch.
        self.assertEqual(cpd.diff_values(None, None), [])
        entries = cpd.diff_values(None, "x")
        self.assertEqual(entries[0]["category"], "type_mismatch")
        self.assertEqual(entries[0]["reference_type"], "null")


class DiffValuesObjectTests(unittest.TestCase):
    def test_equal_objects_produce_no_diff(self):
        self.assertEqual(cpd.diff_values({"a": 1}, {"a": 1}), [])
        self.assertEqual(
            cpd.diff_values({"a": {"b": [1, 2]}}, {"a": {"b": [1, 2]}}), []
        )

    def test_missing_key_in_candidate(self):
        entries = cpd.diff_values({"a": 1, "b": 2}, {"a": 1})
        missing = [e for e in entries if e["category"] == "missing"]
        self.assertEqual(len(missing), 1)
        self.assertEqual(missing[0]["path"], "$.b")
        self.assertEqual(missing[0]["reference_value"], 2)
        self.assertNotIn("candidate_value", missing[0])

    def test_extra_key_in_candidate(self):
        entries = cpd.diff_values({"a": 1}, {"a": 1, "c": 3})
        extra = [e for e in entries if e["category"] == "extra"]
        self.assertEqual(len(extra), 1)
        self.assertEqual(extra[0]["path"], "$.c")
        self.assertEqual(extra[0]["candidate_value"], 3)
        self.assertNotIn("reference_value", extra[0])

    def test_changed_nested_value_path(self):
        entries = cpd.diff_values(
            {"book": {"title": "A"}}, {"book": {"title": "B"}}
        )
        changed = [e for e in entries if e["category"] == "changed"]
        self.assertEqual(len(changed), 1)
        self.assertEqual(changed[0]["path"], "$.book.title")
        self.assertEqual(changed[0]["reference_value"], "A")
        self.assertEqual(changed[0]["candidate_value"], "B")

    def test_nested_missing_and_extra_paths(self):
        entries = cpd.diff_values(
            {"book": {"author": "Alice", "isbn": "000"}},
            {"book": {"author": "Alice", "pages": 100}},
        )
        paths = {(e["category"], e["path"]) for e in entries}
        self.assertIn(("missing", "$.book.isbn"), paths)
        self.assertIn(("extra", "$.book.pages"), paths)

    def test_type_mismatch_nested(self):
        entries = cpd.diff_values(
            {"year": 2024}, {"year": "2024"}
        )
        tm = [e for e in entries if e["category"] == "type_mismatch"]
        self.assertEqual(len(tm), 1)
        self.assertEqual(tm[0]["path"], "$.year")


class DiffValuesArrayTests(unittest.TestCase):
    def test_equal_arrays_produce_no_diff(self):
        self.assertEqual(cpd.diff_values([1, 2, 3], [1, 2, 3]), [])

    def test_array_element_changed_uses_index_path(self):
        entries = cpd.diff_values([1, 2, 3], [1, 9, 3])
        changed = [e for e in entries if e["category"] == "changed"]
        self.assertEqual(len(changed), 1)
        self.assertEqual(changed[0]["path"], "$[1]")
        self.assertEqual(changed[0]["reference_value"], 2)
        self.assertEqual(changed[0]["candidate_value"], 9)

    def test_array_longer_candidate_reports_extra(self):
        entries = cpd.diff_values([1, 2], [1, 2, 3])
        extra = [e for e in entries if e["category"] == "extra"]
        self.assertEqual(len(extra), 1)
        self.assertEqual(extra[0]["path"], "$[2]")
        self.assertEqual(extra[0]["candidate_value"], 3)

    def test_array_shorter_candidate_reports_missing(self):
        entries = cpd.diff_values([1, 2, 3], [1, 2])
        missing = [e for e in entries if e["category"] == "missing"]
        self.assertEqual(len(missing), 1)
        self.assertEqual(missing[0]["path"], "$[2]")
        self.assertEqual(missing[0]["reference_value"], 3)

    def test_array_of_objects_diffs_nested_paths(self):
        entries = cpd.diff_values(
            [{"title": "A"}], [{"title": "B"}]
        )
        changed = [e for e in entries if e["category"] == "changed"]
        self.assertEqual(len(changed), 1)
        self.assertEqual(changed[0]["path"], "$[0].title")


class IgnoreFieldsTests(unittest.TestCase):
    def test_ignore_field_skips_matching_key_anywhere(self):
        a = {"device": "iPhone", "book": {"title": "A", "timestamp": 1}}
        b = {"device": "Pixel", "book": {"title": "A", "timestamp": 2}}
        entries = cpd.diff_values(a, b, ignore_fields={"device", "timestamp"})
        self.assertEqual(entries, [])

    def test_ignore_field_does_not_skip_unmatched(self):
        a = {"device": "iPhone", "title": "A"}
        b = {"device": "Pixel", "title": "B"}
        entries = cpd.diff_values(a, b, ignore_fields={"device"})
        changed = [e for e in entries if e["category"] == "changed"]
        self.assertEqual(len(changed), 1)
        self.assertEqual(changed[0]["path"], "$.title")


class ComparePlatformsTests(unittest.TestCase):
    def _platforms(self):
        return {
            "ios": {"book": {"title": "A", "author": "Alice"}, "year": 2024},
            "android": {"book": {"title": "B", "author": "Alice"}, "year": 2024},
            "harmony": {"book": {"title": "A"}, "year": 2024},
        }

    def test_result_shape_and_metadata(self):
        result = cpd.compare_platforms(self._platforms(), reference="ios")
        self.assertEqual(result["tool"], "cross-platform-diff")
        self.assertEqual(result["reference"], "ios")
        self.assertEqual(result["candidates"], ["android", "harmony"])
        self.assertEqual(result["ignored_fields"], [])
        self.assertIn("android", result["diffs"])
        self.assertIn("harmony", result["diffs"])
        for name in ("android", "harmony"):
            for cat in ("missing", "extra", "changed", "type_mismatch"):
                self.assertIn(cat, result["diffs"][name])

    def test_android_changed_field_categorized(self):
        result = cpd.compare_platforms(self._platforms(), reference="ios")
        android = result["diffs"]["android"]
        self.assertEqual([e["path"] for e in android["changed"]], ["$.book.title"])
        self.assertEqual(android["missing"], [])
        self.assertEqual(android["extra"], [])
        self.assertEqual(android["type_mismatch"], [])

    def test_harmony_missing_field_categorized(self):
        result = cpd.compare_platforms(self._platforms(), reference="ios")
        harmony = result["diffs"]["harmony"]
        self.assertEqual([e["path"] for e in harmony["missing"]], ["$.book.author"])
        self.assertEqual(harmony["changed"], [])

    def test_summary_counts(self):
        result = cpd.compare_platforms(self._platforms(), reference="ios")
        self.assertEqual(
            result["summary"]["android"],
            {"missing": 0, "extra": 0, "changed": 1, "type_mismatch": 0, "total": 1},
        )
        self.assertEqual(
            result["summary"]["harmony"],
            {"missing": 1, "extra": 0, "changed": 0, "type_mismatch": 0, "total": 1},
        )

    def test_ignored_fields_recorded_and_applied(self):
        platforms = {
            "ios": {"title": "A", "device": "iPhone", "timestamp": 1},
            "android": {"title": "A", "device": "Pixel", "timestamp": 2},
        }
        result = cpd.compare_platforms(
            platforms, reference="ios", ignore_fields=["device", "timestamp"]
        )
        self.assertEqual(result["ignored_fields"], ["device", "timestamp"])
        self.assertEqual(result["summary"]["android"]["total"], 0)

    def test_reference_not_in_platforms_raises(self):
        with self.assertRaises(ValueError):
            cpd.compare_platforms({"ios": {}, "android": {}}, reference="harmony")

    def test_only_reference_plus_one_candidate(self):
        result = cpd.compare_platforms(
            {"ios": {"a": 1}, "android": {"a": 2}}, reference="ios"
        )
        self.assertEqual(result["candidates"], ["android"])
        self.assertEqual(result["summary"]["android"]["changed"], 1)

    def test_no_differences_when_all_equal(self):
        platforms = {"ios": {"a": 1}, "android": {"a": 1}, "harmony": {"a": 1}}
        result = cpd.compare_platforms(platforms, reference="ios")
        for name in ("android", "harmony"):
            self.assertEqual(result["summary"][name]["total"], 0)


class RenderSummaryTests(unittest.TestCase):
    def test_summary_mentions_platforms_counts_and_paths(self):
        result = cpd.compare_platforms(
            {
                "ios": {"book": {"title": "A"}},
                "android": {"book": {"title": "B"}},
            },
            reference="ios",
        )
        text = cpd.render_summary(result)
        self.assertIn("android", text)
        self.assertIn("$.book.title", text)
        self.assertIn("changed", text)

    def test_summary_says_no_differences_when_clean(self):
        result = cpd.compare_platforms(
            {"ios": {"a": 1}, "android": {"a": 1}}, reference="ios"
        )
        text = cpd.render_summary(result)
        # Should clearly communicate there is nothing to report.
        self.assertTrue(
            "no differences" in text.lower() or "0" in text,
            msg=text,
        )

    def test_summary_lists_each_category(self):
        platforms = {
            "ios": {"a": 1, "b": 2, "c": "x", "d": 3},
            "android": {"a": 1, "b": 99, "c": 5, "e": 7},
        }
        result = cpd.compare_platforms(platforms, reference="ios")
        text = cpd.render_summary(result)
        for cat in ("missing", "extra", "changed", "type_mismatch"):
            self.assertIn(cat, text)


class CLITests(unittest.TestCase):
    import io as _io
    import tempfile as _tempfile

    def _write(self, tmpdir, name, obj):
        path = os.path.join(tmpdir, name)
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(obj, fh)
        return path

    def _run(self, argv):
        stdout = self._io.StringIO()
        stderr = self._io.StringIO()
        code = cpd.main(argv, stdout=stdout, stderr=stderr)
        return code, stdout.getvalue(), stderr.getvalue()

    def test_json_output_is_valid_json_and_exit_one_with_diffs(self):
        with self._tempfile.TemporaryDirectory() as d:
            ios = self._write(d, "ios.json", {"a": 1, "b": 2})
            android = self._write(d, "android.json", {"a": 1, "b": 3})
            harmony = self._write(d, "harmony.json", {"a": 1, "b": 2})
            code, out, err = self._run(
                ["--ios", ios, "--android", android, "--harmony", harmony,
                 "--format", "json"]
            )
        self.assertEqual(code, 1)
        result = json.loads(out)
        self.assertEqual(result["reference"], "ios")
        self.assertEqual(result["summary"]["android"]["changed"], 1)
        self.assertEqual(result["summary"]["harmony"]["total"], 0)

    def test_exit_zero_when_no_diffs(self):
        with self._tempfile.TemporaryDirectory() as d:
            ios = self._write(d, "ios.json", {"a": 1})
            android = self._write(d, "android.json", {"a": 1})
            code, out, err = self._run(
                ["--ios", ios, "--android", android, "--format", "json"]
            )
        self.assertEqual(code, 0)

    def test_summary_format_goes_to_stdout(self):
        with self._tempfile.TemporaryDirectory() as d:
            ios = self._write(d, "ios.json", {"title": "A"})
            android = self._write(d, "android.json", {"title": "B"})
            code, out, err = self._run(
                ["--ios", ios, "--android", android, "--format", "summary"]
            )
        self.assertEqual(code, 1)
        self.assertIn("Cross-platform diff summary", out)
        self.assertIn("$.title", out)
        self.assertEqual(err, "")

    def test_both_format_json_stdout_summary_stderr(self):
        with self._tempfile.TemporaryDirectory() as d:
            ios = self._write(d, "ios.json", {"title": "A"})
            android = self._write(d, "android.json", {"title": "B"})
            code, out, err = self._run(
                ["--ios", ios, "--android", android, "--format", "both"]
            )
        self.assertEqual(code, 1)
        # stdout is machine-readable JSON.
        result = json.loads(out)
        self.assertEqual(result["summary"]["android"]["changed"], 1)
        # stderr is the human summary.
        self.assertIn("Cross-platform diff summary", err)

    def test_reference_flag_changes_reference(self):
        with self._tempfile.TemporaryDirectory() as d:
            ios = self._write(d, "ios.json", {"a": 1})
            android = self._write(d, "android.json", {"a": 1})
            harmony = self._write(d, "harmony.json", {"a": 1})
            code, out, err = self._run(
                ["--ios", ios, "--android", android, "--harmony", harmony,
                 "--reference", "android", "--format", "json"]
            )
        self.assertEqual(code, 0)
        result = json.loads(out)
        self.assertEqual(result["reference"], "android")
        self.assertEqual(sorted(result["candidates"]), ["harmony", "ios"])

    def test_ignore_flag_strips_platform_fields(self):
        with self._tempfile.TemporaryDirectory() as d:
            ios = self._write(d, "ios.json",
                              {"title": "A", "device": "iPhone", "timestamp": 1})
            android = self._write(d, "android.json",
                                  {"title": "A", "device": "Pixel", "timestamp": 2})
            code, out, err = self._run(
                ["--ios", ios, "--android", android,
                 "--ignore", "device", "timestamp", "--format", "json"]
            )
        self.assertEqual(code, 0)
        result = json.loads(out)
        self.assertEqual(result["ignored_fields"], ["device", "timestamp"])
        self.assertEqual(result["summary"]["android"]["total"], 0)

    def test_missing_file_returns_error_exit(self):
        with self._tempfile.TemporaryDirectory() as d:
            ios = self._write(d, "ios.json", {"a": 1})
            code, out, err = self._run(
                ["--ios", ios, "--android", os.path.join(d, "nope.json"),
                 "--format", "json"]
            )
        self.assertEqual(code, 2)
        self.assertIn("error", err.lower())

    def test_invalid_json_returns_error_exit(self):
        with self._tempfile.TemporaryDirectory() as d:
            ios = self._write(d, "ios.json", {"a": 1})
            bad = os.path.join(d, "bad.json")
            with open(bad, "w", encoding="utf-8") as fh:
                fh.write("{not valid json")
            code, out, err = self._run(
                ["--ios", ios, "--android", bad, "--format", "json"]
            )
        self.assertEqual(code, 2)
        self.assertIn("error", err.lower())

    def test_reference_not_provided_returns_error(self):
        with self._tempfile.TemporaryDirectory() as d:
            ios = self._write(d, "ios.json", {"a": 1})
            android = self._write(d, "android.json", {"a": 1})
            # harmony is the reference but its file was not given.
            code, out, err = self._run(
                ["--ios", ios, "--android", android,
                 "--reference", "harmony", "--format", "json"]
            )
        self.assertEqual(code, 2)
        self.assertIn("error", err.lower())

    def test_no_platforms_returns_error(self):
        code, out, err = self._run(["--format", "json"])
        self.assertEqual(code, 2)


if __name__ == "__main__":
    unittest.main()
