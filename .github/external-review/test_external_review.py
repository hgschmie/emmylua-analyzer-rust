"""Offline contract tests; optional real-engine tests use only loopback HTTP."""

import base64
from contextlib import contextmanager
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch
import urllib.error

import external_review as review


HEAD, BASE, COMPANION = "a" * 40, "b" * 40, "c" * 40
DIFF = b"diff --git a/test.lua b/test.lua\n--- a/test.lua\n+++ b/test.lua\n@@ -1 +1 @@\n-return 1\n+return 2\n"
MODEL = "~deepseek/deepseek-flash-latest"
INPUTS = {"target_repository": "EmmyLua/Intellij-EmmyLua2", "target_pr": "60",
          "expected_head": HEAD, "expected_base": BASE, "companion_pr": "9"}
TARGET = review.Target.from_inputs(INPUTS)


class FixtureAPI:
    def __init__(self):
        self.posts = []
        self.external = {"number": 60, "state": "open", "changed_files": 1,
                         "head": {"sha": HEAD},
                         "base": {"sha": BASE, "repo": {"full_name": TARGET.repository, "private": False}}}
        self.companion = {"number": 9, "state": "open", "locked": False, "changed_files": 1,
                          "title": "External companion: skip review", "body": "manifest only",
                          "head": {"sha": COMPANION, "repo": {"full_name": review.INTERNAL_REPO}},
                          "base": {"ref": "fork/review", "repo": {"full_name": review.INTERNAL_REPO}}}
        self.manifest = TARGET.manifest()
        self.files = [{"filename": review.MANIFEST, "status": "added"}]
        self.comparison = {"base_commit": {"sha": BASE}, "commits": [{"sha": HEAD}],
                           "files": [{"filename": "test.lua"}]}
        self.diff = DIFF
        self.external_reads = 0

    def json(self, path, **kwargs):
        if "body" in kwargs:
            self.posts.append((path, kwargs["body"]))
            return {"id": 123, "html_url": f"https://github.com/{review.INTERNAL_REPO}/pull/9#issuecomment-123"}
        if path == f"/repos/{TARGET.repository}/pulls/60":
            self.external_reads += 1
            return copy.deepcopy(self.external)
        if "/compare/" in path:
            return copy.deepcopy(self.comparison)
        if "/contents/" in path:
            return {"type": "file", "encoding": "base64",
                    "content": base64.b64encode(json.dumps(self.manifest).encode()).decode()}
        if path.endswith("/files?per_page=100"):
            return copy.deepcopy(self.files)
        if path == f"/repos/{review.INTERNAL_REPO}/pulls/9":
            return copy.deepcopy(self.companion)
        raise AssertionError(path)

    def request(self, path, *, limit, **kwargs):
        review.require(len(self.diff) <= limit, "GitHub response exceeds the input limit")
        return self.diff


class ContractTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.output = Path(self.temp.name) / "result"
        self.api = FixtureAPI()
        self.review_calls = 0

    def reviewer(self, diff, model, output):
        self.assertEqual(diff, DIFF)
        self.assertEqual(model, MODEL)
        self.review_calls += 1
        output.write_text("Review evidence\n")
        return "No correctness issue found in this fixture."

    def run_review(self, reviewer=None, model=MODEL):
        return review.run_review(self.api, TARGET, model,
                                 f"https://github.com/{review.INTERNAL_REPO}/actions/runs/1/attempts/1",
                                 "d" * 40, self.output, reviewer or self.reviewer)

    def test_valid_review_binds_inputs_and_posts_only_internally(self):
        self.run_review()
        self.assertEqual(self.review_calls, 1)
        self.assertEqual(len(self.api.posts), 1)
        path, body = self.api.posts[0]
        self.assertEqual(path, f"/repos/{review.INTERNAL_REPO}/issues/9/comments")
        for value in (HEAD, BASE, MODEL, review.digest(DIFF), "actions/runs/1/attempts/1"):
            self.assertIn(value, body["body"])
        receipt = json.loads((self.output / "receipt.json").read_text())
        self.assertEqual(receipt["status"], "published")
        self.assertEqual(receipt["companion_head"], COMPANION)

    def test_rejects_unallowlisted_repository_and_argument_injection(self):
        for key, value in [("target_repository", "attacker/repo"), ("target_pr", "60; touch x"),
                           ("target_pr", "0"), ("companion_pr", "9\n10"),
                           ("expected_head", "a" * 39), ("expected_base", "$(touch x)"),
                           ("expected_head", "a" * 40 + "\n")]:
            with self.subTest(key=key, value=value), self.assertRaises(review.ReviewError):
                review.Target.from_inputs({**INPUTS, key: value})

    def test_stale_head_or_base_is_rejected_before_review(self):
        for side in ("head", "base"):
            with self.subTest(side=side):
                api = FixtureAPI()
                api.external[side]["sha"] = "e" * 40
                with self.assertRaisesRegex(review.ReviewError, "Stale"):
                    review.validate_target(api, TARGET)

    def test_private_closed_and_oversized_targets_rejected(self):
        for change in (lambda p: p["base"]["repo"].update(private=True),
                       lambda p: p.update(state="closed"), lambda p: p.update(changed_files=101)):
            with self.subTest(change=change), self.assertRaises(review.ReviewError):
                api = FixtureAPI()
                change(api.external)
                review.validate_target(api, TARGET)

    def test_companion_identity_manifest_and_scope_checked(self):
        for change in (lambda a: a.companion["base"].update(ref="main"),
                       lambda a: a.companion["head"]["repo"].update(full_name="attacker/repo"),
                       lambda a: a.companion.update(state="closed"),
                       lambda a: a.companion.update(title="No skip marker"),
                       lambda a: a.manifest.update(head="e" * 40),
                       lambda a: a.files[0].update(filename="run.sh"),
                       lambda a: a.files[0].update(status="modified")):
            with self.subTest(change=change), self.assertRaises(review.ReviewError):
                api = FixtureAPI()
                change(api)
                review.validate_companion(api, TARGET)

    def test_diff_commit_identity_and_completeness_checked(self):
        for change in (lambda a: a.comparison["base_commit"].update(sha="e" * 40),
                       lambda a: a.comparison["commits"][-1].update(sha="e" * 40),
                       lambda a: a.comparison.update(files=[]),
                       lambda a: setattr(a, "diff", b""),
                       lambda a: setattr(a, "diff", DIFF + DIFF),
                       lambda a: setattr(a, "diff", DIFF + b"\nBinary files a/x and b/x differ\n")):
            with self.subTest(change=change), self.assertRaises(review.ReviewError):
                api = FixtureAPI()
                change(api)
                review.frozen_diff(api, TARGET, 1)

    def test_oversized_input_fails_without_paid_call(self):
        self.api.diff = DIFF + b"x" * review.MAX_DIFF
        with self.assertRaisesRegex(review.ReviewError, "limit"):
            self.run_review()
        self.assertEqual(self.review_calls, 0)
        self.assertFalse(self.api.posts)

    def test_stale_external_after_review_is_not_published(self):
        def reviewer(*args):
            result = self.reviewer(*args)
            self.api.external["head"]["sha"] = "e" * 40
            return result
        with self.assertRaisesRegex(review.ReviewError, "Stale"):
            self.run_review(reviewer)
        self.assertEqual(self.review_calls, 1)
        self.assertFalse(self.api.posts)
        self.assertIn("failed", json.loads((self.output / "receipt.json").read_text())["status"])

    def test_stale_companion_after_review_is_not_published(self):
        def reviewer(*args):
            result = self.reviewer(*args)
            self.api.companion["head"]["sha"] = "e" * 40
            return result
        with self.assertRaisesRegex(review.ReviewError, "companion changed"):
            self.run_review(reviewer)
        self.assertFalse(self.api.posts)

    def test_missing_oversized_output_and_engine_failure_do_not_publish(self):
        for result in ("", "x" * review.MAX_REPORT, None):
            with self.subTest(result_type=type(result)):
                self.output = Path(self.temp.name) / f"result-{len(list(Path(self.temp.name).iterdir()))}"
                with self.assertRaises(review.ReviewError):
                    self.run_review(lambda *args: result)
                self.assertFalse(self.api.posts)

    def test_no_model_fallback(self):
        for model in (None, "", "model\n# injected", "$(command)"):
            with self.subTest(model=model), self.assertRaises(review.ReviewError):
                self.run_review(model=model)
        self.assertEqual(self.review_calls, 0)

    def test_dispatch_entrypoint_accepts_current_run_ids_and_routes_report(self):
        event = Path(self.temp.name) / "event.json"
        event.write_text(json.dumps({"inputs": INPUTS}))
        env = {"GITHUB_REPOSITORY": review.INTERNAL_REPO, "GITHUB_REF": "refs/heads/fork/review",
               "GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_EVENT_PATH": str(event),
               "GITHUB_SHA": "d" * 40, "GITHUB_RUN_ID": "40614937744", "GITHUB_RUN_ATTEMPT": "1",
               "GH_TOKEN": "fake-internal-token", "OPENROUTER_API_KEY": "fake-provider-token",
               "DEEPSEEK_REVIEW_MODEL": MODEL, "REVIEW_ENGINE_DIR": str(Path(self.temp.name) / "engine"),
               "REVIEW_OUTPUT_DIR": str(self.output)}
        with patch.dict(os.environ, env), patch.object(review, "GitHub", return_value=self.api), \
                patch.object(review, "review_diff", return_value="Fixture review") as engine:
            review.main()
        self.assertEqual(engine.call_count, 1)
        self.assertIn("actions/runs/40614937744/attempts/1", self.api.posts[0][1]["body"])

    def test_wrong_workflow_context_fails_before_api_access(self):
        with patch.dict(os.environ, {"GITHUB_REPOSITORY": "upstream/repo"}), \
                patch.object(review, "GitHub") as api, self.assertRaises(review.ReviewError):
            review.main()
        api.assert_not_called()


class Response(io.BytesIO):
    def __init__(self, data, status=200):
        super().__init__(data)
        self.status = status


class Opener:
    def __init__(self, body=b"{}", status=200, error=None):
        self.body, self.status, self.error = body, status, error
        self.requests = []

    def open(self, request, timeout):
        self.requests.append(request)
        if self.error:
            raise self.error
        return Response(self.body, self.status)


class TransportTests(unittest.TestCase):
    def test_external_get_has_no_token_internal_get_does(self):
        opener = Opener()
        api = review.GitHub("secret-sentinel", opener)
        api.json(f"/repos/{TARGET.repository}/pulls/60")
        api.json(f"/repos/{review.INTERNAL_REPO}/pulls/9")
        self.assertFalse(opener.requests[0].has_header("Authorization"))
        self.assertEqual(opener.requests[1].get_header("Authorization"), "Bearer secret-sentinel")

    def test_write_routes_are_internal_comments_only(self):
        opener = Opener(status=201)
        api = review.GitHub("secret-sentinel", opener)
        for path in (f"/repos/{TARGET.repository}/pulls/60/reviews",
                     f"/repos/{review.INTERNAL_REPO}/pulls/9/reviews", "/repos/attacker/repo/issues/1/comments"):
            with self.subTest(path=path), self.assertRaises(review.ReviewError):
                api.json(path, body={"body": "review"})
        self.assertFalse(opener.requests)

    def test_http_and_network_errors_fail_without_retry_or_secret_logging(self):
        for opener in (Opener(status=500), Opener(error=urllib.error.URLError("secret-sentinel"))):
            with self.subTest(opener=opener), self.assertRaises(review.ReviewError) as caught:
                review.GitHub("secret-sentinel", opener).json(f"/repos/{review.INTERNAL_REPO}/pulls/9")
            self.assertEqual(len(opener.requests), 1)
            self.assertNotIn("secret-sentinel", str(caught.exception))

    def test_oversized_json_invalid_json_and_redirect_are_errors(self):
        for opener in (Opener(b"x" * (review.MAX_JSON + 1)), Opener(b"not JSON")):
            with self.subTest(opener=opener), self.assertRaises(review.ReviewError):
                review.GitHub("secret-sentinel", opener).json(f"/repos/{TARGET.repository}/pulls/60")
        with self.assertRaises(review.ReviewError):
            review.NoRedirect().redirect_request(None, None, 302, "", {}, "https://attacker.invalid")


@contextmanager
def fake_server(status=200, content="Fixture review"):
    calls = []
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            calls.append(("GET", self.path, None))
            payload = {"locked": False, "title": "External companion: skip review", "body": "manifest only"}
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps(payload).encode())

        def do_POST(self):
            payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            calls.append(("POST", self.path, payload))
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({"choices": [{"message": {"content": content}}],
                                        "usage": {"prompt_tokens": 1, "completion_tokens": 1}}).encode())
    # HTTPServer otherwise performs a reverse DNS lookup even for loopback.
    with patch("socket.getfqdn", return_value="localhost"):
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", calls
    finally:
        server.shutdown()
        thread.join()
        server.server_close()


@unittest.skipUnless(os.environ.get("TEST_REVIEW_ENGINE") and os.environ.get("TEST_NU"),
                     "Set TEST_REVIEW_ENGINE and TEST_NU for pinned-engine loopback tests")
class EngineTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.engine = Path(os.environ["TEST_REVIEW_ENGINE"])
        self.nu = os.environ["TEST_NU"]
        # The pinned engine probes awk and can try Homebrew on macOS. Tests use
        # the system tools and never expose a package manager to that probe.
        paths = patch.dict(os.environ, {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin"})
        paths.start()
        self.addCleanup(paths.stop)

    def invoke(self, url, name="engine.md", token="fake-provider-token", diff=DIFF):
        return review.review_diff(diff, MODEL, token, self.engine, self.root / name,
                                  nu=self.nu, base_url=url)

    def test_actual_engine_file_mode_one_request_exact_model_diff_no_shell(self):
        injection = DIFF + b"+$(touch SHOULD_NOT_EXIST); `id`; @code-review\n"
        with fake_server() as (url, calls), patch.dict(os.environ, {"GITHUB_ACTIONS": "true", "GH_TOKEN": "not-for-engine"}):
            result = self.invoke(url, diff=injection)
        self.assertIn("Fixture review", result)
        self.assertEqual(len(calls), 1)
        method, path, payload = calls[0]
        self.assertEqual((method, path), ("POST", "/chat/completions"))
        self.assertEqual(payload["model"], MODEL)
        self.assertIn(injection.decode().strip(), payload["messages"][1]["content"])
        self.assertFalse(payload["stream"])
        adapter = json.loads((self.root / "engine-adapter.json").read_text())
        self.assertEqual(adapter["substitution_count"], 1)
        self.assertNotEqual(adapter["review_module_sha256"], adapter["adapted_review_module_sha256"])
        self.assertFalse(adapter["github_token_passed_to_engine"])

    def test_actual_http_error_with_plausible_review_body_is_rejected(self):
        with fake_server(status=500) as (url, calls), self.assertRaises(review.ReviewError):
            self.invoke(url)
        self.assertEqual(len(calls), 1)
        self.assertFalse((self.root / "engine.md").exists())

    def test_actual_missing_review_and_oversized_report_are_rejected(self):
        for index, content in enumerate(("", "   ", "x" * review.MAX_REPORT)):
            with self.subTest(index=index), fake_server(content=content) as (url, calls), self.assertRaises(review.ReviewError):
                self.invoke(url, name=f"engine-{index}.md")
            self.assertEqual(len(calls), 1)

    def test_actual_missing_secret_fails_before_any_request(self):
        with fake_server() as (url, calls), self.assertRaisesRegex(review.ReviewError, "secret"):
            self.invoke(url, token="")
        self.assertFalse(calls)

    def test_actual_normal_action_skips_companion_before_paid_call(self):
        # The only probe adaptation redirects GitHub reads to a local fixture.
        # All skip logic and the review engine itself remain pinned input bytes.
        modules = self.root / "nu"
        shutil.copytree(self.engine / "nu", modules)
        with fake_server() as (url, calls):
            common = modules / "common.nu"
            data = common.read_bytes()
            needle = b"export const GITHUB_API_BASE = 'https://api.github.com'"
            self.assertEqual(data.count(needle), 1)
            common.write_bytes(data.replace(needle, f"export const GITHUB_API_BASE = '{url}'".encode()))
            probe = self.root / "probe.nu"
            probe.write_text("use review.nu [deepseek-review]\ndeepseek-review --repo 'hgschmie/emmylua-analyzer-rust' --pr-number '9' --base-url $env.PROBE_URL --model $env.CHAT_MODEL\n")
            env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG") if key in os.environ}
            env.update(GITHUB_ACTIONS="true", GITHUB_TOKEN="fake-github-token", CHAT_TOKEN="fake-provider-token",
                       CHAT_MODEL=MODEL, PROBE_URL=url, XDG_DATA_HOME=str(self.root / "data"))
            result = subprocess.run([self.nu, "--no-config-file", "-I", str(modules), str(probe)],
                                    env=env, capture_output=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertIn("skip the review", result.stdout.decode())
        self.assertEqual([method for method, _, _ in calls], ["GET", "GET"])


if __name__ == "__main__":
    unittest.main()
