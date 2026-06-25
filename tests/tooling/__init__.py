"""Integration tests for dev-time tooling under tools/.

Each tool ships its own test module here (e.g. test_fixture_manifest.py).
Run a single suite with:

    python3 -m unittest tests.tooling.test_fixture_manifest -v

Run everything:

    python3 -m unittest discover -s tests/tooling -p 'test_*.py' -v
"""
