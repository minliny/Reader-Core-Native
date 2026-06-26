"""Structural validation for P2-S4 remote reading error-path fixtures.

These tests verify the three structured error-path fixtures (host timeout,
JS-unsupported, network-fail) are well-formed and document the expected
error propagation through the remote-reading pipeline. The e2e spec suite
is a specification artifact — reader-cli --host-replay-suite does not yet
support ``hostError`` steps, so these tests validate structure only, not
runtime replay. The host-replay format fixtures (samples/host-replay/) are
validated against the ``reader-host-replay/1`` schema.
"""

import json
import os
import unittest

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
E2E_SPEC_PATH = os.path.join(
    REPO_ROOT, "tests", "fixtures", "host_replay", "error_paths_e2e_suite.json"
)
HOST_REPLAY_DIR = os.path.join(REPO_ROOT, "samples", "host-replay")


def _load_json(path):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


class ErrorPathE2eSuiteStructureTests(unittest.TestCase):
    """Validate tests/fixtures/host_replay/error_paths_e2e_suite.json."""

    def setUp(self):
        self.suite = _load_json(E2E_SPEC_PATH)

    def test_top_level_fields(self):
        self.assertIn("description", self.suite)
        self.assertIn("steps", self.suite)
        self.assertEqual(self.suite.get("errorPathSpecVersion"), 1)
        self.assertEqual(len(self.suite["steps"]), 3)

    def test_step_names_cover_three_error_paths(self):
        names = [step["name"] for step in self.suite["steps"]]
        self.assertEqual(
            names,
            [
                "book.search.host-timeout",
                "book.search.js-unsupported",
                "book.search.network-fail",
            ],
        )

    def test_each_step_has_command_and_host_request(self):
        for step in self.suite["steps"]:
            self.assertIn("description", step, f"{step['name']} missing description")
            self.assertIn("completionRequestId", step)
            self.assertIn("command", step)
            self.assertEqual(step["command"]["method"], "book.search")
            self.assertIn("expectHostRequest", step)
            self.assertEqual(step["expectHostRequest"]["capability"], "http.execute")
            self.assertIn("expectResult", step)

    def test_host_timeout_step_uses_host_error(self):
        step = self.suite["steps"][0]
        self.assertIn("hostError", step)
        self.assertNotIn("hostResult", step)
        error = step["hostError"]["error"]
        self.assertEqual(error["code"], "HTTP_TRANSPORT_TIMEOUT")
        self.assertTrue(error["retryable"])
        self.assertEqual(error["details"]["phase"], "connect")
        self.assertEqual(error["details"]["timeoutMillis"], 10000)
        result_error = step["expectResult"]["error"]
        self.assertEqual(result_error["code"], "HTTP_TRANSPORT_TIMEOUT")
        self.assertTrue(result_error["retryable"])

    def test_js_unsupported_step_uses_host_result_with_rule_error(self):
        step = self.suite["steps"][1]
        self.assertIn("hostResult", step)
        self.assertNotIn("hostError", step)
        self.assertEqual(step["hostResult"]["status"], 200)
        result_error = step["expectResult"]["error"]
        self.assertEqual(result_error["code"], "JS_RUNTIME_UNAVAILABLE")
        self.assertFalse(result_error["retryable"])
        rule = step["expectResult"]["rule"]
        self.assertEqual(rule["kind"], "javascript")
        self.assertEqual(rule["phase"], "search")
        source_rules = step["command"]["params"]["source"]["rules"]["search"]
        self.assertEqual(source_rules[0]["kind"], "javascript")

    def test_network_fail_step_uses_host_error(self):
        step = self.suite["steps"][2]
        self.assertIn("hostError", step)
        self.assertNotIn("hostResult", step)
        error = step["hostError"]["error"]
        self.assertEqual(error["code"], "HTTP_NETWORK_FAILURE")
        self.assertTrue(error["retryable"])
        self.assertEqual(error["details"]["phase"], "dns")
        self.assertEqual(error["details"]["errno"], "ENOTFOUND")
        self.assertEqual(error["details"]["attempts"], 3)
        result_error = step["expectResult"]["error"]
        self.assertEqual(result_error["code"], "HTTP_NETWORK_FAILURE")

    def test_each_step_command_carries_source_and_search_request(self):
        for step in self.suite["steps"]:
            params = step["command"]["params"]
            self.assertIn("sourceId", params)
            self.assertIn("searchRequest", params)
            self.assertIn("source", params)
            search_req = params["searchRequest"]
            self.assertIn("url", search_req)
            self.assertIn("method", search_req)
            self.assertIn("retry", search_req)
            self.assertIn("session", search_req)
            self.assertIn("rules", params["source"])


class HostReplayErrorFixtureTests(unittest.TestCase):
    """Validate samples/host-replay/ error fixtures against reader-host-replay/1."""

    def test_timeout_fixture_is_valid_error_replay(self):
        fixture = _load_json(os.path.join(HOST_REPLAY_DIR, "003-error-timeout.json"))
        self.assertEqual(fixture["format"], "reader-host-replay/1")
        self.assertEqual(fixture["outcome"], "error")
        self.assertEqual(fixture["error"]["code"], "HTTP_TRANSPORT_TIMEOUT")
        self.assertTrue(fixture["error"]["retryable"])
        self.assertIn("details", fixture["error"])
        self.assertIn("timeout", fixture.get("tags", []))

    def test_network_fail_fixture_is_valid_error_replay(self):
        fixture = _load_json(os.path.join(HOST_REPLAY_DIR, "008-network-fail.json"))
        self.assertEqual(fixture["format"], "reader-host-replay/1")
        self.assertEqual(fixture["outcome"], "error")
        self.assertEqual(fixture["error"]["code"], "HTTP_NETWORK_FAILURE")
        self.assertTrue(fixture["error"]["retryable"])
        self.assertEqual(fixture["error"]["details"]["phase"], "dns")
        self.assertEqual(fixture["error"]["details"]["errno"], "ENOTFOUND")
        self.assertIn("network-fail", fixture.get("tags", []))

    def test_error_fixtures_use_example_test_hosts_only(self):
        """Error fixtures must not reference real domains (sanitization guard)."""
        for name in ("003-error-timeout.json", "008-network-fail.json"):
            fixture = _load_json(os.path.join(HOST_REPLAY_DIR, name))
            url = fixture["request"]["url"]
            self.assertIn(
                ".example.test",
                url,
                f"{name} references non-example.test host: {url}",
            )
            host_in_details = fixture["error"].get("details", {}).get("host", "")
            if host_in_details:
                self.assertIn(
                    ".example.test",
                    host_in_details,
                    f"{name} error.details.host is not example.test: {host_in_details}",
                )


if __name__ == "__main__":
    unittest.main()
