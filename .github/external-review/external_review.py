#!/usr/bin/env python3
"""Review a frozen public diff and publish only to its internal companion PR."""

import base64
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request


INTERNAL_REPO = "hgschmie/emmylua-analyzer-rust"
ALLOWED_REPOS = {
    "justarandomgeek/vscode-factoriomod-debug",
    "EmmyLua/Intellij-EmmyLua2",
}
ENGINE_SHA = "d8fed14d464331174c6653b419a132e83d85771d"
MANIFEST = ".github/external-review-target.json"
MAX_DIFF = 200_000
MAX_JSON = 2_000_000
MAX_REPORT = 48_000
MAX_FILES = 100
SHA = re.compile(r"[0-9a-f]{40}\Z")
NUMBER = re.compile(r"[1-9][0-9]{0,19}\Z")


class ReviewError(Exception):
    """A bounded, non-secret explanation suitable for the Actions log."""


def require(condition, message):
    if not condition:
        raise ReviewError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def number(value):
    require(isinstance(value, str) and NUMBER.fullmatch(value), "Invalid PR/run number")
    return int(value)


@dataclass(frozen=True)
class Target:
    repository: str
    pull_request: int
    head: str
    base: str
    companion: int

    @classmethod
    def from_inputs(cls, inputs):
        require(inputs.get("target_repository") in ALLOWED_REPOS, "Target is not allowlisted")
        head, base = inputs.get("expected_head"), inputs.get("expected_base")
        require(isinstance(head, str) and SHA.fullmatch(head), "Invalid expected head SHA")
        require(isinstance(base, str) and SHA.fullmatch(base), "Invalid expected base SHA")
        return cls(inputs["target_repository"], number(inputs.get("target_pr")),
                   head, base, number(inputs.get("companion_pr")))

    def manifest(self):
        return {"schema_version": 1, "repository": self.repository,
                "pull_request": self.pull_request, "head": self.head, "base": self.base}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ReviewError("Unexpected GitHub redirect")


class GitHub:
    """Never send credentials to external targets or POST outside the internal repo."""

    def __init__(self, token, opener=None):
        self.token = token
        self.opener = opener or urllib.request.build_opener(NoRedirect())

    def request(self, path, *, accept="application/vnd.github+json", body=None, limit=MAX_JSON):
        internal = path.startswith(f"/repos/{INTERNAL_REPO}/")
        require(internal or any(path.startswith(f"/repos/{repo}/") for repo in ALLOWED_REPOS),
                "Unexpected GitHub API route")
        if body is not None:
            require(re.fullmatch(rf"/repos/{re.escape(INTERNAL_REPO)}/issues/[1-9][0-9]*/comments", path),
                    "Publication is restricted to internal companion comments")
        headers = {"Accept": accept, "X-GitHub-Api-Version": "2022-11-28",
                   "User-Agent": "fork-external-pr-review"}
        if internal:
            headers["Authorization"] = f"Bearer {self.token}"
        data = None if body is None else json.dumps(body).encode()
        if data is not None:
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request("https://api.github.com" + path, data=data, headers=headers)
        try:
            with self.opener.open(request, timeout=60) as response:
                require(response.status == (200 if body is None else 201), "Unexpected GitHub HTTP status")
                result = response.read(limit + 1)
        except (urllib.error.URLError, TimeoutError, OSError):
            raise ReviewError("GitHub request failed; no automatic retry") from None
        require(len(result) <= limit, "GitHub response exceeds the input limit")
        return result

    def json(self, path, **kwargs):
        try:
            return json.loads(self.request(path, **kwargs))
        except (ValueError, UnicodeError):
            raise ReviewError("Invalid GitHub JSON response") from None


def validate_target(api, target):
    pr = api.json(f"/repos/{target.repository}/pulls/{target.pull_request}")
    require(pr.get("number") == target.pull_request and pr.get("state") == "open",
            "Target PR is not open or has the wrong identity")
    require(pr["base"]["repo"]["full_name"] == target.repository
            and pr["base"]["repo"]["private"] is False, "Target repository must be public")
    require(pr["head"]["sha"] == target.head and pr["base"]["sha"] == target.base,
            "Stale external head/base; no review will be published")
    require(type(pr.get("changed_files")) is int and 0 < pr["changed_files"] <= MAX_FILES,
            "Empty or oversized changed-file set")
    return pr["changed_files"]


def validate_companion(api, target, expected_head=None):
    path = f"/repos/{INTERNAL_REPO}/pulls/{target.companion}"
    pr = api.json(path)
    require(pr.get("number") == target.companion and pr.get("state") == "open"
            and not pr.get("locked"), "Internal companion must be open and unlocked")
    require("skip review" in ((pr.get("title") or "") + "\n" + (pr.get("body") or "")),
            "Companion must include literal 'skip review' to suppress automatic manifest review")
    require(pr["base"]["repo"]["full_name"] == INTERNAL_REPO
            and pr["head"]["repo"]["full_name"] == INTERNAL_REPO
            and pr["base"]["ref"] == "fork/review", "Invalid internal companion repository/base")
    head = pr["head"]["sha"]
    require(isinstance(head, str) and SHA.fullmatch(head), "Invalid companion head")
    require(expected_head is None or head == expected_head, "Internal companion changed during review")
    require(pr.get("changed_files") == 1, "Companion must contain only its target manifest")
    files = api.json(path + "/files?per_page=100")
    require(len(files) == 1 and files[0].get("filename") == MANIFEST
            and files[0].get("status") == "added", "Companion must add only the target manifest")
    file = api.json(f"/repos/{INTERNAL_REPO}/contents/{MANIFEST}?ref={head}", limit=16_000)
    require(file.get("type") == "file" and file.get("encoding") == "base64", "Invalid manifest content")
    try:
        manifest = json.loads(base64.b64decode(file["content"], validate=False))
    except (ValueError, UnicodeError):
        raise ReviewError("Invalid manifest JSON") from None
    require(manifest == target.manifest(), "Companion manifest does not match the requested target")
    return head


def frozen_diff(api, target, file_count):
    path = f"/repos/{target.repository}/compare/{target.base}...{target.head}"
    comparison = api.json(path)
    require(comparison["base_commit"]["sha"] == target.base
            and comparison["commits"][-1]["sha"] == target.head,
            "Comparison does not match expected commits")
    files = comparison.get("files", [])
    require(len(files) == file_count, "Comparison file list is incomplete or differs from PR")
    data = api.request(path, accept="application/vnd.github.diff", limit=MAX_DIFF)
    try:
        text = data.decode("utf-8")
    except UnicodeError:
        raise ReviewError("Diff is not UTF-8 text") from None
    require(text.startswith("diff --git ") and text.count("\ndiff --git ") + 1 == file_count,
            "Empty or incomplete comparison diff")
    require("\nGIT binary patch\n" not in text and not re.search(r"^Binary files .* differ$", text, re.M),
            "Binary changes cannot be reviewed by this text-only workflow")
    return data


def checked_process(argv, **kwargs):
    try:
        return subprocess.run(argv, check=True, timeout=600, **kwargs)
    except (subprocess.SubprocessError, OSError):
        raise ReviewError("Review engine/tool failed or timed out; no automatic retry") from None


def review_diff(diff, model, token, engine, output, *, nu="nu", base_url="https://openrouter.ai/api/v1"):
    """Invoke trusted engine code once, with only a frozen data blob and provider token."""
    require(bool(token), "Missing configured review API secret")
    revision = checked_process(["git", "-C", str(engine), "rev-parse", "HEAD"], capture_output=True).stdout
    require(revision.decode().strip() == ENGINE_SHA, "Unexpected review engine revision")
    require(not checked_process(["git", "-C", str(engine), "status", "--porcelain", "--untracked-files=no"],
                                capture_output=True).stdout, "Review engine checkout is modified")
    require(not output.exists(), "Review output already exists")
    with tempfile.TemporaryDirectory(prefix="external-review-") as temp:
        scratch = Path(temp)
        modules = scratch / "nu"
        shutil.copytree(engine / "nu", modules)
        # The pinned engine uses -e, which accepts non-2xx HTTP bodies. Keep its
        # file-output implementation, but make transport errors fail explicitly.
        module = modules / "review.nu"
        source = module.read_bytes()
        permissive = b"let response = http post -e -H $CHAT_HEADER -t application/json $url $payload"
        require(source.count(permissive) == 1, "Pinned engine HTTP adapter no longer matches")
        adapted = source.replace(permissive, permissive.replace(b"post -e -H", b"post -H"))
        module.write_bytes(adapted)
        checked_process(["git", "init", "--quiet", str(scratch)], capture_output=True)
        blob = checked_process(["git", "hash-object", "-w", "--stdin"], cwd=scratch,
                               input=diff, capture_output=True).stdout.decode().strip()
        require(SHA.fullmatch(blob), "Invalid frozen-diff blob ID")
        write_json(output.with_name("engine-adapter.json"), {
            "engine_sha": ENGINE_SHA, "review_module_sha256": digest(source),
            "adapted_review_module_sha256": digest(adapted),
            "substitution": "Remove -e from nonstreaming HTTP POST", "substitution_count": 1,
            "diff_blob": blob, "diff_sha256": digest(diff), "file_mode": True,
            "github_token_passed_to_engine": False,
        })
        # GITHUB_ACTIONS=true overrides --output in the pinned engine. Explicitly
        # force file mode, omit PR arguments, and give the engine no GitHub token.
        env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG") if key in os.environ}
        env.update({"GITHUB_ACTIONS": "false", "CHAT_TOKEN": token, "CHAT_MODEL": model,
                    "REVIEW_OUTPUT": str(output.resolve()), "REVIEW_PATCH_CMD": f"git show {blob}",
                    "REVIEW_BASE_URL": base_url, "PWD": str(scratch),
                    "XDG_DATA_HOME": str(scratch / "data"), "XDG_CONFIG_HOME": str(scratch / "config")})
        runner = Path(__file__).with_name("engine.nu").resolve()
        checked_process([nu, "--no-config-file", "-I", str(modules), str(runner)], cwd=scratch,
                        env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    require(output.is_file(), "Review engine produced no report")
    require(0 < output.stat().st_size <= MAX_REPORT, "Review output is empty or oversized")
    report = output.read_text(encoding="utf-8")
    require("## Review Detail\n" in report, "Review engine produced no review detail")
    detail = report.split("## Review Detail\n", 1)[1].split("\n## Token Usage", 1)[0]
    require(detail.strip(), "Review engine produced no review detail")
    return report


def run_review(api, target, model, run_url, workflow_sha, output, reviewer):
    require(isinstance(model, str) and re.fullmatch(r"[A-Za-z0-9~][A-Za-z0-9._~:/+\-]{0,199}", model),
            "Missing or invalid configured model; no fallback is allowed")
    output.mkdir(parents=True, exist_ok=False)
    receipt = {"target": target.manifest(), "companion_pr": target.companion,
               "configured_model": model, "run": run_url, "workflow_sha": workflow_sha,
               "engine_sha": ENGINE_SHA, "engine_adapter": "Remove -e from nonstreaming HTTP POST",
               "status": "validating"}
    write_json(output / "receipt.json", receipt)
    try:
        count = validate_target(api, target)
        companion_head = validate_companion(api, target)
        diff = frozen_diff(api, target, count)
        # Recheck mutable PR pointers after fetching the immutable comparison.
        require(validate_target(api, target) == count, "Target changed-file count changed")
        validate_companion(api, target, companion_head)
        (output / "input.diff").write_bytes(diff)
        receipt.update(diff_sha256=digest(diff), diff_bytes=len(diff), changed_files=count,
                       companion_head=companion_head, status="reviewing")
        write_json(output / "receipt.json", receipt)
        result = reviewer(diff, model, output / "engine.md")
        require(isinstance(result, str) and result.strip(), "Missing review result")
        # Revalidate before any publication. A stale run keeps evidence only.
        require(validate_target(api, target) == count, "Target changed-file count changed")
        validate_companion(api, target, companion_head)
        report = (f"# External PR review\n\n"
                  f"Target: https://github.com/{target.repository}/pull/{target.pull_request}\n\n"
                  f"Head: `{target.head}`\n\nBase: `{target.base}`\n\n"
                  f"Configured model: `{model}`\n\nRun: {run_url}\n\n"
                  f"Workflow revision: `{workflow_sha}`\n\nDiff SHA-256: `{digest(diff)}`\n\n"
                  "Scope: full text diff only; no reviewed code was executed. "
                  "This is automated feedback, not a test or approval result.\n\n---\n\n" + result)
        require(len(report.encode()) <= MAX_REPORT, "Bound review report exceeds publication limit")
        (output / "report.md").write_text(report, encoding="utf-8")
        response = api.json(f"/repos/{INTERNAL_REPO}/issues/{target.companion}/comments", body={"body": report})
        require(isinstance(response.get("id"), int) and response.get("html_url", "").startswith(
            f"https://github.com/{INTERNAL_REPO}/pull/{target.companion}#issuecomment-"),
            "Unexpected internal publication response")
        receipt.update(status="published", comment_url=response["html_url"], report_sha256=digest(report.encode()))
    except Exception:
        receipt["status"] = "failed; do not interpret as a clean review"
        raise
    finally:
        write_json(output / "receipt.json", receipt)


def main():
    env = os.environ
    require(env.get("GITHUB_REPOSITORY") == INTERNAL_REPO
            and env.get("GITHUB_REF") == "refs/heads/fork/review"
            and env.get("GITHUB_EVENT_NAME") == "workflow_dispatch", "Unexpected workflow context")
    target = Target.from_inputs(json.loads(Path(env["GITHUB_EVENT_PATH"]).read_text())["inputs"])
    require(SHA.fullmatch(env.get("GITHUB_SHA", "")), "Invalid workflow revision")
    run_id, attempt = number(env.get("GITHUB_RUN_ID")), number(env.get("GITHUB_RUN_ATTEMPT"))
    require(bool(env.get("GH_TOKEN")), "Missing internal GitHub token")
    token = env.get("OPENROUTER_API_KEY", "")
    require(bool(token), "Missing configured review API secret")
    engine = Path(env["REVIEW_ENGINE_DIR"]).resolve()
    run_review(GitHub(env["GH_TOKEN"]), target, env.get("DEEPSEEK_REVIEW_MODEL"),
               f"https://github.com/{INTERNAL_REPO}/actions/runs/{run_id}/attempts/{attempt}",
               env["GITHUB_SHA"], Path(env["REVIEW_OUTPUT_DIR"]),
               lambda diff, model, output: review_diff(diff, model, token, engine, output))


if __name__ == "__main__":
    try:
        main()
    except (ReviewError, KeyError, IndexError, TypeError, ValueError, OSError) as error:
        print(str(error) if isinstance(error, ReviewError) else "Invalid workflow data or local I/O failure", file=sys.stderr)
        sys.exit(1)
