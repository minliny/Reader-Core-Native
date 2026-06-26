#!/usr/bin/env python3
"""Tests for the BookSource corpus oracle adapter."""

import json
import contextlib
import io
import os
import stat
import sys
import tempfile
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_ROOT = os.path.abspath(os.path.join(_HERE, "..", ".."))
sys.path.insert(0, os.path.join(_ROOT, "scripts"))

import corpus_booksource_oracle as bso  # noqa: E402


EVENTS = [
    {
        "type": "result",
        "requestId": 1,
        "data": {
            "sourceId": "basic-src",
            "name": "Basic Test Source",
            "imported": True,
        },
    },
    {
        "type": "host.request",
        "requestId": 2,
        "operationId": 42,
        "capability": "http.execute",
        "params": {"url": "https://books.example.test/search?q=dune/"},
    },
    {
        "type": "result",
        "requestId": 3,
        "data": {
            "sourceId": "basic-src",
            "books": [
                {
                    "bookId": "1",
                    "title": "Dune&nbsp;",
                    "author": "Herbert",
                }
            ],
            "http": {"status": 200},
        },
    },
    {
        "type": "result",
        "requestId": 4,
        "data": {
            "sourceId": "basic-src",
            "book": {"bookId": "1", "title": "Dune", "intro": "A&nbsp;desert"},
        },
    },
    {
        "type": "result",
        "requestId": 5,
        "data": {
            "sourceId": "basic-src",
            "bookId": "1",
            "toc": [{"title": "Chapter&nbsp;1", "url": "https://books.example.test/c1/"}],
        },
    },
    {
        "type": "result",
        "requestId": 6,
        "data": {
            "sourceId": "basic-src",
            "bookId": "1",
            "chapterTitle": "Chapter 1",
            "content": "First\r\nSecond",
            "via": "rule",
        },
    },
]


class BookSourceOracleTests(unittest.TestCase):
    def test_event_stream_maps_to_five_pipeline_schema(self):
        text = "\n".join(json.dumps(event) for event in EVENTS)
        oracle = bso.oracle_from_text(text, input_id="case-1")

        self.assertEqual(oracle["type"], bso.ORACLE_TYPE)
        self.assertEqual(oracle["inputId"], "case-1")
        self.assertEqual(oracle["sourceId"], "basic-src")
        self.assertEqual(
            sorted(oracle["pipelines"].keys()),
            ["chapter", "detail", "search", "sourceImport", "toc"],
        )
        self.assertEqual(oracle["pipelines"]["search"]["status"], "ok")
        self.assertEqual(
            oracle["pipelines"]["search"]["data"]["books"][0]["title"],
            "Dune",
        )
        self.assertEqual(
            oracle["host"]["requests"][0]["operationId"],
            "<normalized>",
        )
        self.assertEqual(
            oracle["pipelines"]["toc"]["data"]["toc"][0]["url"],
            "https://books.example.test/c1",
        )

    def test_missing_pipeline_is_explicit(self):
        text = json.dumps([EVENTS[0]])
        oracle = bso.oracle_from_text(text, input_id="case-2")
        self.assertEqual(oracle["pipelines"]["sourceImport"]["status"], "ok")
        self.assertEqual(oracle["pipelines"]["search"]["status"], "missing")
        self.assertIsNone(oracle["pipelines"]["search"]["data"])

    def test_can_invoke_reader_cli_booksource_fixture(self):
        with tempfile.TemporaryDirectory(prefix="bso-cli-") as tmp:
            output = os.path.join(tmp, "out.ndjson")
            with open(output, "w", encoding="utf-8") as handle:
                handle.write("\n".join(json.dumps(event) for event in EVENTS))

            fake_cli = os.path.join(tmp, "reader-cli")
            with open(fake_cli, "w", encoding="utf-8") as handle:
                handle.write(
                    "#!/usr/bin/env python3\n"
                    "import pathlib, sys\n"
                    "assert sys.argv[1] == '--booksource-fixture'\n"
                    "assert sys.argv[2] == 'fixture.json'\n"
                    "print(pathlib.Path({0!r}).read_text(), end='')\n".format(output)
                )
            os.chmod(fake_cli, os.stat(fake_cli).st_mode | stat.S_IXUSR)

            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                rc = bso.main([
                    "--reader-cli",
                    fake_cli,
                    "--fixture",
                    "fixture.json",
                ])

        self.assertEqual(rc, 0)
        self.assertIn("booksource-five-pipeline", stdout.getvalue())

    def test_schema_requires_all_five_pipelines(self):
        schema_path = os.path.join(
            _ROOT,
            "samples",
            "corpus-booksource-oracle",
            "booksource-five-pipeline.schema.json",
        )
        with open(schema_path, "r", encoding="utf-8") as handle:
            schema = json.load(handle)
        required = schema["properties"]["pipelines"]["required"]
        self.assertEqual(
            required,
            ["sourceImport", "search", "detail", "toc", "chapter"],
        )


if __name__ == "__main__":
    unittest.main()
