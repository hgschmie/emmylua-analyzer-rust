# External public PR reviews

This fork-only workflow reviews public PRs in
`justarandomgeek/vscode-factoriomod-debug` and `EmmyLua/Intellij-EmmyLua2`.
It uses the existing `OPENROUTER_API_KEY` secret and the exact configured
`DEEPSEEK_REVIEW_MODEL`; a missing model or secret is an error. It never posts
to an upstream repository. The normal analyzer review workflow is unchanged.

Create an internal companion PR in `hgschmie/emmylua-analyzer-rust`, targeting
`fork/review`. Its only change must add `.github/external-review-target.json`:

```json
{
  "schema_version": 1,
  "repository": "EmmyLua/Intellij-EmmyLua2",
  "pull_request": 60,
  "head": "<exact 40-character external head SHA>",
  "base": "<exact 40-character external base SHA>"
}
```

Include the literal lowercase phrase **`skip review`** in the companion title
or body. The existing pinned action checks this phrase before making a model
request; this prevents a redundant automatic review of the manifest itself.
Keep the companion open for discussion and never merge its manifest into source.

Dispatch **Review external public PR** on `fork/review`, supplying
`target_repository`, `target_pr`, `expected_head`, `expected_base`, and
`companion_pr`. Use the same values as the manifest. The workflow runs only in
this fork on that branch. Each dispatch makes at most one provider request;
transport failures are not retried automatically. Check pending runs before
dispatching, and treat a rerun as a new paid request.

The helper checks public PR metadata and the manifest, then obtains the diff
from the immutable `base...head` comparison. It checks file counts and limits,
and rechecks mutable PR pointers before review and before posting. Stale inputs
fail without posting a report. Reports record the exact head, base, configured
model, workflow revision, run URL and diff hash; subsequent upstream changes do
not change what that historical report reviewed.

Only diff data is fetched from the reviewed target. It becomes a blob in a
scratch Git repository; the pinned engine receives the fixed command
`git show <validated blob SHA>`. No target source is checked out or executed.
The engine gets the provider token through its environment, no GitHub token and
no PR arguments. `GITHUB_ACTIONS=false` forces file output because the pinned
engine otherwise prioritizes posting to GitHub over its `--output` option.

The engine revision remains `d8fed14d464331174c6653b419a132e83d85771d`.
One asserted substitution in a private copy removes `-e` from its nonstreaming
HTTP POST so HTTP failures cannot masquerade as successful review bodies.
`engine-adapter.json` records original/adapted module hashes and the diff blob.
No engine config file, model substitution, command evaluation or automatic paid
retry is used. Engine stdout/stderr are not logged because they may contain
transport details; failures produce a bounded diagnostic and retained evidence.

Limits are explicit: 100 files, 200,000 diff bytes and 48,000 report bytes.
Binary diffs, missing/incomplete inputs and missing/oversized outputs fail;
nothing is silently truncated or presented as a clean review. This is diff-only
feedback, not a test run or approval. The full result is posted as one comment
on the internal companion, with input/report/receipt artifacts retained for
30 days, including available evidence on failure. Host metadata checks cannot
prevent the source from changing after the final check; the comment always
identifies the frozen revision it reviewed.

Offline checks (no API keys or paid calls):

```sh
python3 -m unittest discover -s .github/external-review -v
```

For the additional real-engine tests, set `TEST_NU` to Nushell 0.114.1 and
`TEST_REVIEW_ENGINE` to a clean checkout of the pinned engine. Those tests use
fake tokens and loopback HTTP only, including a plausible review body returned
with HTTP 500 and the normal action's manifest-skip path.
