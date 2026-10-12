# EmmyLua IntelliJ development handoff

## Current checkpoint: October 11, 2026

The active checkout is
`upstream/emmylua-analyzer-rust/build/checkouts/intellij-work`, branch
`work/intellij-modules`. It is based on upstream main
`1c8589bbc8e60eb6f40bd89000082a53a10803a7` (0.30.0), with PR #1266's single
signed commit `ef3cf3c899c36172ff52d7b49abe6409f469e6f7` and the internal patches.
The PR branch `fix/workspace-relative-module-resolution` was pushed to the
user's fork. Keep internal patches and packaging off that branch.

The pre-rebase internal branch is preserved at
`backup/internal-before-main-rebase-2026-10-11` (`a7ea9de6`). The old PR series is
preserved at `backup/pr-1266-before-squash-2026-10-11` (`07cbc7ce`). Independent
fix branches still hold their original patches; this operation updates the
internal development baseline, not each upstream submission branch.

Upstream's semantic database rewrite removed the old compilation/db_index and
semantic/infer implementation. The internal port retains these behaviors:

- Caller-root import resolution and explicitly mapped aliases from PR #1266.
- Cross-file diagnostic refresh and preceding same-file global navigation.
- Stable local metatable association, key-aware `__index` inference, definition
  navigation, backing-member documentation and function highlighting.
- Separate property read/write types, generic projections, callback context and
  guard invalidation after writes. Reads retain their read contract.
- Opt-in `strict.tableIndex`, required object fields, writes/nil deletion and
  guards; optional/union dictionary keys, generic aliases and bounded recursion.
- Imported function reads within assignment targets are not redefinitions;
  actual reassignment and `function module.member(...)` still diagnose duplicates.

Integration code now lives mainly in `semantic_model/{dictionary,directional,
index_function,flow,member}.rs`, `semantic_db/facts.rs`, `check/checker/`, and
`emmylua_ls/src/handlers/`. Existing regression cases were retained and adapted
to new semantic identities and upstream presentation ranges. Additional glue
preserves class index signatures, anonymous-object directional contracts,
numeric-loop ownership and dictionary initializer checking in the new engine.

Validation of this port: all 2,450 workspace tests passed with all features,
including the upstream slow test. Workspace Clippy (all targets/features,
warnings denied) and rustfmt passed. Plugin/protocol build results are recorded
in the workspace `HANDOFF.md` and FDE's `docs/ACCEPTANCE.md`.

Shared Cargo cache: `$WORKSPACE/dev/cache/cargo`; verification target:
`$WORKSPACE/upstream/emmylua-analyzer-rust/build/pr-1266-target`. New evidence is
under the analyzer repository's `build/logs/internal-rebase-*`. The old paths
and version-specific statements below are historical.

Packaging uses EmmyLua2's `work/patched-build` checkout under
`upstream/Intellij-EmmyLua2-worktrees/patched-build`, build 133. Its
`analyzer.lock` and FDE's `emmy-analyzer.lock` record the final analyzer commit.
Only macOS arm64 is rebuilt with internal patches; other bundled binaries
remain upstream 0.25.1. FDE is on `intellij-factorio/main`, version 0.7.0-dev.
Use `scripts/build-module-patch.sh` then FDE's `scripts/build.sh test buildPlugin`.
The usual archives are under the main repositories' `build/distributions/`.

## Historical September checkpoint

Checkpoint: September 27, 2026. Implementation and runtime validation were done
September 25. This document is local development documentation on
`work/intellij-modules`, not part of the upstream resolver PR.

## Goal and current result

We are building a Java IntelliJ plugin for Factorio under the namespace
`de.softwareforge.factorio`, reusing the Node Factorio Modding Tool Kit CLI and
language server. EmmyLua2 supplies Lua intelligence; LSP4IJ supplies the LSP/DAP
integration. IntelliJ project/module creation and existing-mod import are
implemented in the separate `intellij-factorio` repository.

The remaining multi-module import blocker required a generic EmmyLua Rust server
fix. Two attached mods containing `helper.lua` previously competed for the same
module entry. Short imports could select the other mod's file, and named
cross-mod imports failed when overlapping roots stripped the mapped prefix.
The fix now passes direct LSP and actual IntelliJ navigation/completion checks.

The user explicitly authorized a PR-ready server patch and its incorporation
into the development baseline. Additional patches are allowed as a last resort;
there is no blanket prohibition. Configuration alternatives were tested first.
The task is no longer waiting for approval to implement this fix.

## Workspace, repositories and branches

Local workspace: `/Users/henning/ai/fmtk` (called `$WORKSPACE` below).
All paths in this table are relative to it.

| Repository | Working branch | Relevant checkpoint |
| --- | --- | --- |
| `spike/emmy-analyzer-source` (this repository) | `work/intellij-modules` | `75629641053563189106266dc53739c8346f391a`: signed resolver backport onto tag `0.24.0` (`bb4f4f0ad7a7aff0b9a5f59e44626cb0497ecd84`) |
| Same Rust repository, upstream review | `fix/workspace-relative-module-resolution` | `966a16d7` (abbreviated): signed generic fix based on upstream main `aaaca68425d9362876228649b0b8d92f07654daa`, version 0.25.1 |
| `Intellij-EmmyLua2` | `work/patched-build` | `3152701`: pinned patched server packaging; also contains the earlier JVM `aarch64`/`arm64` selection fix |
| `intellij-factorio` | `work/initial-plugin` | `14177e5`: pins the patched analyzer and records multi-module acceptance; module implementation was `42126dc` |
| `vscode-factoriomod-debug` | `work/intellij-toolkit` | `6dc3400`: toolkit baseline, including previous LSP and CLI fixes |

Current Rust remotes, read locally on September 27:

- `origin`: `git@github.com:hgschmie/emmylua-analyzer-rust.git` (user fork).
- `upstream`: `git@github.com:EmmyLuaLs/emmylua-analyzer-rust.git`.
- `work/intellij-modules` tracks `origin/work/intellij-modules`; before this
  documentation change, both pointed at `75629641`.
- The local PR branch exists separately. No PR URL or submission status has
  been verified in this handoff. Do not repeat the earlier claim that nothing
  has been pushed: the development remote-tracking branch now exists.
- Earlier records called the Rust remote `CppCXY/emmylua-analyzer-rust`; use the
  current configured remotes above and verify before fetching/pushing.

Start with `git status -sb`, `git remote -v`, and `git branch -avv`. Preserve user
changes. Do not rebase the published development branch or overwrite remote
branches as an incidental cleanup. Keep upstream fixes free of local packaging,
Factorio-specific configuration, this handoff, and unrelated version bumps.

## Patch implementation and semantics

Main implementation:
[LuaModuleIndex](../crates/emmylua_code_analysis/src/db_index/module/mod.rs).

- `file_workspace_roots` records each file's most specific containing configured
  root, honoring workspace import filters. `WorkspaceId` is insufficient because
  multiple source roots share `WorkspaceId::MAIN`.
- `find_module_from(module_path, file_id)` adds caller context. Matching files in
  that root are preferred during exact or fuzzy lookup. Context-free
  `find_module` remains available.
- Existing exact, mapped, then fuzzy lookup order and library fallback remain.
  This is root preference, not strict module isolation: an absent local module
  can still resolve through the existing global/library fallback.
- Normal canonical names still use shortest-root extraction. Explicit
  `moduleMap` rewrites from enclosing roots are additionally indexed as aliases.
  Arbitrary unmapped enclosing-root names are not added.
- `file_module_nodes` tracks all tree nodes for each file. Aliases share the
  original file's export type, semantic identity and visibility. Removal/reindex
  cleans all nodes and fuzzy-index entries; clear resets the added indexes.
- Explicit module-name annotations retain the caller's recorded root.
- Caller context is propagated through dependency collection, annotations,
  inference, diagnostics, semantic declarations, definitions, document links,
  references, descriptions, and require-path completion. Search for
  `find_module_from` to locate the call sites. Member completion benefits from
  the corrected inferred import type.

There is no Factorio-specific naming in the Rust patch. Generic configuration
semantics are documented in [emmyrc_json_EN.md](config/emmyrc_json_EN.md).

Regression tests:

- [Module-index tests](../crates/emmylua_code_analysis/src/db_index/module/test.rs):
  both indexing orders, same-named root directories, nested callers, overlapping
  roots, aliases, rename/replacement, removal/clear, fuzzy lookup, shared-library
  fallback, and scope retention after explicit module naming.
- [Compilation tests](../crates/emmylua_code_analysis/src/compilation/test/module_test.rs):
  two roots infer distinct exported values through identical short imports.
- Downstream protocol harness: `$WORKSPACE/intellij-factorio/scripts/test-module-services.py`.

## Reproduction and why configuration was insufficient

Create roots `/mods`, `/mods/a`, `/mods/b`, with a different export in each
`a/helper.lua` and `b/helper.lua`. Require `helper` from both control files. Add
this map and require `package_b.helper` from a:

```json
{"workspace":{"moduleMap":[{"pattern":"^([ab])[.](.*)$","replace":"package_$1.$2"}]}}
```

Unpatched: one local import resolves to the other root, depending on indexing
order; the qualified alias is unresolved when shortest-root extraction wins.
Patched: each local import uses its root and the named import reaches b.

Four configuration experiments were run with the original binary: source roots,
parent libraries, packages, and strict source roots. Parent libraries/packages
made named imports work, but short names still collided. None passed both
requirements. The upstream module-index implementation at `aaaca684` had the
same defect. Evidence is in `spike/emmy-config-tests/results.json`; the local
experiment script is `spike/emmy-config-matrix.py`.

## Reproducing Rust builds and tests

Tested environment: macOS 27, Apple M2 Max; Homebrew Rust/Cargo 1.98.1; IDEA
2026.2.3 IU-262.10968.63 / JBR 25.0.4; build JDK Temurin 25 targeting the plugin's
configured Java level; Node 26.9.0; Factorio 2.1.20; LSP4IJ 0.21.0.

The source declares `stable` in `rust-toolchain.toml`. Use the lockfile and check
installed versions before diagnosing new compiler warnings. Do not install a
new toolchain into the user's home as an incidental step.

Run from a shell, keeping caches and explicit output inside the workspace:

```sh
export WORKSPACE=/Users/henning/ai/fmtk
export CARGO_HOME="$WORKSPACE/spike/cache/cargo"
export CARGO_TARGET_DIR="$WORKSPACE/spike/emmy-target"
export TMPDIR="$WORKSPACE/spike/tmp"
mkdir -p "$CARGO_HOME" "$TMPDIR"
cd "$WORKSPACE/spike/emmy-analyzer-source"
cargo test --locked -p emmylua_code_analysis -p emmylua_ls
cargo clippy --locked -p emmylua_code_analysis -p emmylua_ls --tests
cargo build --locked --release -p emmylua_ls
```

For changes, run `cargo fmt --all` and `git diff --check`. A focused first pass
can use `cargo test --locked -p emmylua_code_analysis --lib db_index::module`.
Revalidate upstream and backport separately if either changes. Do not assume
one passing build validates both versions.

Recorded results:

| Source | Analysis tests | Server tests | Other |
| --- | --- | --- | --- |
| Upstream PR branch | 1,096 passed | 213 passed, 1 ignored | Clippy passed, warnings in unchanged files |
| 0.24.0 development backport | 1,082 passed | 213 passed, 1 ignored | Native arm64 release build passed |
| Factorio plugin | 22 passed | N/A | Plugin Verifier Compatible; 4 deprecated, 20 experimental, 4 internal API usages |

These are dated results, not a claim that new commits or remote updates have
been tested. Existing warnings included the parser's unused `node::*` import
and Clippy suggestions in unchanged parser/formatter/flow/test code.

## Source pins and the documentation-only HEAD change

Both downstream pins currently contain the executable-source commit
`75629641053563189106266dc53739c8346f391a`:

- `$WORKSPACE/Intellij-EmmyLua2/analyzer.lock`
- `$WORKSPACE/intellij-factorio/emmy-analyzer.lock`

This handoff commit advances the development branch without changing executable
source. The packaging script compares analyzer **HEAD** to its pin, not just
source contents. Therefore a direct packaging invocation from the documentation
HEAD will intentionally stop with a revision mismatch. The existing built ZIP
still identifies the correct tested source; the pins were not silently advanced
for this documentation task.

To reproduce the exact existing package, start with a clean Rust checkout,
build the pinned revision detached, then restore the development branch. This
subshell restores the branch even if the build fails:

```sh
(
  set -e
  WORKSPACE=/Users/henning/ai/fmtk
  ANALYZER="$WORKSPACE/spike/emmy-analyzer-source"
  test -z "$(git -C "$ANALYZER" status --porcelain)"
  test "$(git -C "$ANALYZER" branch --show-current)" = work/intellij-modules
  trap 'git -C "$ANALYZER" switch work/intellij-modules' EXIT
  git -C "$ANALYZER" switch --detach "$(cat "$WORKSPACE/Intellij-EmmyLua2/analyzer.lock")"
  bash "$WORKSPACE/Intellij-EmmyLua2/scripts/build-module-patch.sh"
)
```

For actual code changes, make a new signed development commit, deliberately
update both pins after review, rebuild the EmmyLua2 package, and then rebuild
and test the Factorio plugin. Do not bypass the guard or claim an old source pin
for a new binary. Do not cherry-pick this handoff into the upstream PR.

## Plugin packaging and isolated IDE

The local recipe above uses:

- EmmyLua2 branch `work/patched-build`, script `scripts/build-module-patch.sh`.
- Native server at `spike/emmy-target/release/emmylua_ls`.
- Workspace-local Gradle caches, `JAVA_TOOL_OPTIONS` home/tmp redirects, installed
  IDEA SDK at `/Users/henning/Applications/IntelliJ IDEA.app` and JDK at
  `/Library/Java/JavaVirtualMachines/temurin-25.jdk/Contents/Home`.
- Version `0.24.0-115-IDEA262-patched-modules`, unchanged plugin ID.
- `-PlocalAnalyzerPath=...` replaces the **darwin-arm64 server only**. Other
  platform binaries in the archive remain upstream 0.24.0. Do not advertise
  those platforms as containing the resolver fix.
- Embedded `server/darwin-arm64/analyzer.lock`; extraction verifies its value
  and compares SHA-256 of the packaged server to the built release binary.

Artifact:
`Intellij-EmmyLua2/build/distributions/IntelliJ-EmmyLua2-0.24.0-115-IDEA262-patched-modules.zip`.
Verified extracted dependency:
`spike/emmylua2-module-build/verified/IntelliJ-EmmyLua2`.

Then, from `$WORKSPACE`:

```sh
bash intellij-factorio/scripts/build.sh test buildPlugin verifyPlugin
# The isolated IDE must be closed before preparation.
python3 intellij-factorio/scripts/prepare-test-ide.py
bash intellij-factorio/scripts/launch-test-ide.sh
```

`build.sh` checks the extracted analyzer pin and toolkit pin. Preparation replaces
owned dependency directories, avoiding stale versioned JARs. Do not revert to
copying new JARs over old directories: that caused the old plugin descriptor to
load alongside the new files. Startup logs should identify EmmyLua2
`0.24.0-115-IDEA262-patched-modules`.

The isolated profile is `spike/plugin-ide/{config,plugins,system,log,home,tmp}`;
its project is `spike/plugin-fixture`. External test modules are
`spike/module-ui/wizard-probe` and `spike/module-ui/import-probe`; their helpers
return `wizard_marker` and `import_marker`. Real mods under `~/factorio` are not
fixtures and must not be modified. The preparation script rewrites the copied
Factorio run configuration; explicitly restore its `factorioModule` association
if repeating module-specific DAP tests.

The everyday IDE has not been updated by this work. Do not install there as an
incidental test. UI tests need an unlocked desktop. Use the available
computer-use skill and `@oai/sky` via node_repl; refresh accessibility state after
actions. Prefix-based completion and Cmd-B navigation supplied the final UI
acceptance evidence.

## Protocol harness and evidence

From `$WORKSPACE`, this creates disposable fixtures and captures traffic beneath
the selected evidence root:

```sh
EMMY_LS="$WORKSPACE/spike/emmy-target/release/emmylua_ls" \
FACTORIO_TEST_ROOT="$WORKSPACE/spike/emmy-development-protocol" \
python3 intellij-factorio/scripts/test-module-services.py
```

The harness checks local definitions, distinct inferred exports, named cross-mod
definition/type completion, and FMTK locale completion/create/delete in both
external roots. It exits nonzero if any check fails. `EMMY_TEST_CONFIG` can
supply an alternate JSON configuration; normally leave it unset. The default
server path is the isolated profile's installed binary, unless `EMMY_LS` is set.
Keep `FACTORIO_TEST_ROOT` inside the allowed workspace.

Existing single-mod regression, after isolated-profile preparation:

```sh
python3 intellij-factorio/scripts/test-language-services.py
```

It uses the existing copied `spike/plugin-protocol` fixture, local cache/std
resources, installed isolated server and bundled toolkit CLI. It checks API,
event, dependency-edit and locale lifecycle behavior. Do not assume it works
from an otherwise empty checkout without the local fixtures/dependencies.

Local evidence (not necessarily tracked or present in another clone):

| Workspace-relative path | Contents |
| --- | --- |
| `spike/emmy-config-tests/results.json` | Failed configuration-only alternatives |
| `spike/emmy-patch-test.log` | Upstream analysis tests |
| `spike/emmy-upstream-ls-tests.log`, `spike/emmy-upstream-clippy.log` | Upstream server tests/lints |
| `spike/emmy-development-tests.log`, `spike/emmy-development-release.log` | Backport tests/release build |
| `spike/emmy-upstream-protocol.log`, `spike/emmy-development-protocol.log` | Passing multi-root checks for each source base |
| `spike/emmy-development-single-mod.log` | Existing workflow regression |
| `spike/emmylua2-module-build.log`, `spike/emmylua2-module-recipe.log` | Packaging and reproducible extraction |
| `spike/factorio-emmy-module-build.log` | Downstream build/tests/verifier |
| `spike/emmy-module-ui-evidence/` | AX captures of both local completions/definitions and cross-mod completion/definition; `ui-protocol.jsonl` captures IDE requests/responses |
| `spike/emmy-actual-config.log` | Direct completion requests using the actual test project's configuration |
| `spike/emmy-multi-root-PR.md` | Draft PR description; its old remote naming is superseded by this handoff |
| `intellij-factorio/docs/ACCEPTANCE.md` | Full downstream acceptance history and qualifications |

A temporary Python stdio proxy captured the UI protocol. It was removed, its
`.real` backup deleted, and the exact release binary restored and hash-checked.
No proxy should ship. All owned IDE/server test processes were closed at the
last completed acceptance run; verify current processes rather than assuming
that remains true.

## Limitations and next steps

1. **Client completion display:** empty-prefix completion did not consistently
   appear in IntelliJ even when the traced server returned the correct member.
   Typing a member prefix and invoking completion worked in both roots and for
   the named import. This patch fixes resolution; it does not claim to fix that
   client display/cache behavior.
2. **Earlier rapid-open/edit race:** a separate EmmyLua didOpen/didChange ordering
   issue remains documented in downstream acceptance. A short settle after open
   was used in existing protocol tests. Do not conflate it with module resolution
   or claim this patch fixes it.
3. **Generic scope:** global/library fallback is preserved; this is not isolated
   Lua global namespaces per mod. Per-module toolchains, LuaLS and profiling are
   not part of this patch. Windows/Linux/Intel runtime acceptance has not run.
4. **Upstream review:** check whether a PR has been filed and whether upstream has
   moved before preparing a follow-up. The review branch uses upstream main;
   the development branch intentionally stays on 0.24.0. Address review changes
   on the PR branch, test, sign, and backport them; keep downstream pins/builds
   consistent. Do not push, submit, merge, or publish merely because a handoff
   suggests a next step.
5. **Independent architecture fix:** the earlier IntelliJ EmmyLua2 JVM `aarch64`
   selector fix is separate from this Rust PR. The user previously submitted
   that PR. This handoff does not establish its current upstream merge status.

For a fresh continuation, inspect all affected worktrees and pins, read the
acceptance qualifications, choose the correct branch for the intended change,
and reproduce the smallest relevant failing test before editing. Broaden tests
when changing inference/indexing or client integration, not for this docs-only
handoff. Update evidence and limitations candidly, and leave the development
checkout on `work/intellij-modules`.

## Permissions and workflow carried from the session

The user authorized reading `~/factorio` and following symlinks. Explicit writes
and project outputs must stay within `~/ai/fmtk`; writing elsewhere requires
asking first. Incidental OS/app runtime writes are allowed. Keep Cargo/Gradle
caches and test fixtures under the workspace. Do not repurpose HOME or CODEX_HOME.
Use copied mods and isolated saves. No live publishing, everyday IDE install,
or third-party messages are implied by the development task.

Signed commits are established practice; `git -c commit.gpgsign=true commit`
and `git verify-commit HEAD` worked with the user's configured signing agent.
Do not change signing keys/configuration. No sub-agent delegation was requested.
Use concise progress updates; ask only for genuinely missing information or an
action outside existing authorization. Follow current session permissions if
these historical notes conflict with newer user instructions.
