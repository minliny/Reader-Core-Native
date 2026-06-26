#!/usr/bin/env python3
"""BookSource corpus oracle adapter.

Consumes JSON/NDJSON output from ``reader-cli --booksource-fixture`` and
normalizes it into the canonical BookSource five-pipeline oracle shape used by
the corpus diff/release-gate tooling.

The adapter accepts either an existing output file/stdin or invokes a
``reader-cli`` binary directly. It deliberately does not run Core business
logic itself; it only maps already-emitted CLI events into stable JSON.
"""

import argparse
import json
import os
import subprocess
import sys

import corpus_canonicalize as cc


TOOL_NAME = "corpus-booksource-oracle"
TOOL_VERSION = "1.0"
SCHEMA_VERSION = 1
ORACLE_TYPE = "booksource-five-pipeline"
DEFAULT_INPUT_ID = "booksource-fixture"
DEFAULT_CLI_FLAG = "--booksource-fixture"

PIPELINE_METHODS = {
    "sourceImport": "source.import",
    "search": "book.search",
    "detail": "book.detail",
    "toc": "book.toc",
    "chapter": "chapter.content",
}

PIPELINE_ORDER = ["sourceImport", "search", "detail", "toc", "chapter"]


class OracleError(Exception):
    """Raised when oracle input cannot be parsed or normalized."""


def _empty_pipeline(name):
    return {
        "method": PIPELINE_METHODS[name],
        "status": "missing",
        "data": None,
    }


def _project_data(pipeline, data):
    if pipeline == "sourceImport":
        return {
            "sourceId": data.get("sourceId"),
            "name": data.get("name"),
            "imported": data.get("imported"),
        }
    if pipeline == "search":
        out = {
            "sourceId": data.get("sourceId"),
            "books": data.get("books", []),
        }
        if "http" in data:
            out["http"] = data.get("http")
        return out
    if pipeline == "detail":
        return {
            "sourceId": data.get("sourceId"),
            "book": data.get("book"),
        }
    if pipeline == "toc":
        return {
            "sourceId": data.get("sourceId"),
            "bookId": data.get("bookId"),
            "toc": data.get("toc", []),
        }
    if pipeline == "chapter":
        return {
            "sourceId": data.get("sourceId"),
            "bookId": data.get("bookId"),
            "chapterTitle": data.get("chapterTitle"),
            "content": data.get("content"),
            "via": data.get("via"),
        }
    return data


def _classify_result_data(data):
    if not isinstance(data, dict):
        return None
    if data.get("imported") is True:
        return "sourceImport"
    if "books" in data:
        return "search"
    if "book" in data:
        return "detail"
    if "toc" in data:
        return "toc"
    if "content" in data and "chapterTitle" in data:
        return "chapter"
    return None


def _load_json_or_ndjson(text):
    stripped = text.strip()
    if not stripped:
        raise OracleError("empty reader-cli output")

    if stripped[0] in "[{":
        try:
            return json.loads(stripped)
        except json.JSONDecodeError:
            # Fall through to NDJSON parsing; a reader-cli event stream begins
            # with "{" too, but contains one JSON object per line.
            pass

    events = []
    for index, line in enumerate(text.splitlines(), start=1):
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError as err:
            raise OracleError("invalid JSON on line {0}: {1}".format(index, err))
    if not events:
        raise OracleError("reader-cli output contained no JSON events")
    return events


def parse_reader_cli_output(text):
    value = _load_json_or_ndjson(text)
    if isinstance(value, dict) and value.get("type") == ORACLE_TYPE:
        return value
    if isinstance(value, dict) and isinstance(value.get("events"), list):
        return value["events"]
    if isinstance(value, list):
        return value
    raise OracleError("expected a BookSource oracle object or reader-cli event list")


def build_oracle_document(value, input_id=DEFAULT_INPUT_ID):
    """Build the canonical oracle document from parsed CLI output."""
    if isinstance(value, dict) and value.get("type") == ORACLE_TYPE:
        return cc.canonicalize(value)

    if not isinstance(value, list):
        raise OracleError("reader-cli output must be an event list")

    pipelines = {name: _empty_pipeline(name) for name in PIPELINE_ORDER}
    host_requests = []
    errors = []

    for event in value:
        if not isinstance(event, dict):
            continue
        event_type = event.get("type")
        if event_type == "result":
            data = event.get("data", {})
            pipeline = _classify_result_data(data)
            if pipeline is None:
                continue
            pipelines[pipeline] = {
                "method": PIPELINE_METHODS[pipeline],
                "status": "ok",
                "data": _project_data(pipeline, data),
            }
        elif event_type == "host.request":
            host_requests.append({
                "requestId": event.get("requestId"),
                "operationId": event.get("operationId"),
                "capability": event.get("capability"),
                "params": event.get("params", {}),
            })
        elif event_type == "error":
            errors.append({
                "requestId": event.get("requestId"),
                "error": event.get("error", {}),
            })

    source_id = None
    for name in PIPELINE_ORDER:
        data = pipelines[name].get("data")
        if isinstance(data, dict) and data.get("sourceId"):
            source_id = data["sourceId"]
            break

    doc = {
        "schemaVersion": SCHEMA_VERSION,
        "type": ORACLE_TYPE,
        "tool": TOOL_NAME,
        "version": TOOL_VERSION,
        "inputId": input_id or DEFAULT_INPUT_ID,
        "sourceId": source_id,
        "pipelines": pipelines,
        "host": {
            "requests": host_requests,
        },
        "errors": errors,
    }
    return cc.canonicalize(doc)


def oracle_from_text(text, input_id=DEFAULT_INPUT_ID):
    return build_oracle_document(parse_reader_cli_output(text), input_id=input_id)


def _read_input(path):
    if path == "-":
        return sys.stdin.read()
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return handle.read()
    except FileNotFoundError:
        raise OracleError("input file not found: {0}".format(path))
    except OSError as err:
        raise OracleError("cannot read input {0}: {1}".format(path, err))


def _invoke_reader_cli(reader_cli, fixture, cli_flag):
    if not fixture:
        raise OracleError("--fixture is required when --reader-cli is used")
    cmd = [reader_cli, cli_flag or DEFAULT_CLI_FLAG, fixture]
    try:
        completed = subprocess.run(
            cmd,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
        )
    except OSError as err:
        raise OracleError("failed to execute reader-cli: {0}".format(err))
    if completed.returncode != 0:
        raise OracleError(
            "reader-cli exited {0}: {1}".format(
                completed.returncode, completed.stderr.strip() or "(no stderr)"
            )
        )
    return completed.stdout


def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog=TOOL_NAME,
        description=(
            "Convert reader-cli --booksource-fixture JSON/NDJSON output into "
            "the canonical BookSource five-pipeline oracle JSON."
        ),
    )
    parser.add_argument(
        "input",
        nargs="?",
        default="-",
        help="reader-cli output JSON/NDJSON file, or '-' for stdin.",
    )
    parser.add_argument(
        "--reader-cli",
        default=None,
        help="Invoke this reader-cli binary instead of reading input.",
    )
    parser.add_argument(
        "--fixture",
        default=None,
        help="Fixture path passed to reader-cli when --reader-cli is used.",
    )
    parser.add_argument(
        "--cli-flag",
        default=DEFAULT_CLI_FLAG,
        help="reader-cli fixture flag to use (default: --booksource-fixture).",
    )
    parser.add_argument(
        "--input-id",
        default=DEFAULT_INPUT_ID,
        help="Stable corpus input id to embed in the oracle output.",
    )
    parser.add_argument(
        "-o", "--output",
        default=None,
        help="Path to write canonical oracle JSON (default: stdout).",
    )
    return parser.parse_args(argv)


def main(argv=None):
    if argv is None:
        argv = sys.argv[1:]
    args = parse_args(argv)

    try:
        if args.reader_cli:
            text = _invoke_reader_cli(args.reader_cli, args.fixture, args.cli_flag)
        else:
            text = _read_input(args.input)
        oracle = oracle_from_text(text, input_id=args.input_id)
    except OracleError as err:
        sys.stderr.write("error: {0}\n".format(err))
        return 2

    out = cc.serialize(oracle) + "\n"
    if args.output:
        os.makedirs(os.path.dirname(os.path.abspath(args.output)), exist_ok=True)
        with open(args.output, "w", encoding="utf-8") as handle:
            handle.write(out)
    else:
        sys.stdout.write(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
