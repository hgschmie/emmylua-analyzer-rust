# Workspace diagnostic scheduling replaces UP-008 (2026-10-14)

The active work tree no longer includes UP-008's per-edit global refresh sender.
The signed contribution `fix/workspace-diagnostic-scheduling` at `7306538d` is based
on upstream/main `96545f45`. It is applied here with a signed provenance-preserving
cherry-pick; the unrelated upstream HashMap refactor is not imported. Other local
analyzer patches remain unchanged.

UP-008 remains on `fix/cross-file-diagnostic-refresh` (`2cfe165c`). The full previous
work tree is retained at `backup/intellij-modules-before-workspace-diagnostics-20261014`
(`df28f68a`). Its removal is an explicit revert commit, making rollback reviewable.

Workspace pulls now observe unsaved edits, external changes, removals, renames,
saves and reloads. Results use open-document versions, clear removed files and
retry stale/incomplete passes without cancelling the parent request. Pending
work stops before shutdown acknowledgement. Ordinary edits do not send global
refresh requests; configuration refreshes respect client capabilities. LSP4IJ
UP-044 is required in our IntelliJ setup to consume these workspace reports.

The standalone branch passes 226 server tests, Clippy/fmt and seven protocol
scenarios. The edit regression times out on the unpatched upstream binary.
Combined validation and package details are recorded in the workspace handoff
and upstream/FIX_INVENTORY.md. No PR or push is implied by this local integration.

Older checkpoints below are historical.

# Overnight upstream refresh (2026-10-14)

Work is based on upstream/main `7096fef53ce3a06db5d06c90cb3b6bc8d81f5f16`.
UP-022/#1285, UP-024/#1289 and UP-027/#1291 are merged upstream; their local
copies are omitted from the replay. The remaining 27 contribution branches are
rebased with their prerequisites and integrated. Production source and tests
are identical to the validated build150 baseline; only ancestry and this handoff
change. Original refs are in `refs/archive/analyzer-refresh-20261014/`.

UP-025/#1290 was closed as superseded by #1288. UP-008/#1283 was closed without
merge: the maintainer prefers complete workspace diagnostics. No replacement is
in this upstream snapshot, so the existing refresh fix remains local-only.
#1286 and #1292 remain the two open analyzer PRs. No new PRs are created.

The frozen LTN request_processor.lua snapshot remains the zero-diagnostic
acceptance baseline with the original configuration. Final validation/publication
and build151 evidence are in workspace `upstream/ANALYZER_REFRESH_2026-10-14.md`
and analyzer `build/investigations/analyzer-refresh-20261014/`. Standalone branch
checks exclude the unchanged long linear-file stress case; the full combined
suite runs it once. No application defaults or mod source are changed.

Older checkpoints below are historical.

# Current upstream refresh (2026-10-13)

The development tree is reconstructed on upstream/main
`a0ce1512cb4f44b6e1f6a21f57a08febb0dbc754`. Thirty retained fix branches are
integrated, plus the existing Factorio compatibility changes and interaction
tests. Original refs are preserved under `refs/archive/analyzer-refresh-20261013/`.

New independent fixes: UP-042 (`fix/aliased-dynamic-member-keys`) expands
string aliases in dynamic member-key diagnostics; UP-043
(`fix/integer-alias-return-types`) permits integer aliases in broad number
return slots. Both reproduce on upstream; neither depends on the arithmetic fix.

UP-025 is superseded by upstream #1288; only its twelve regression tests remain.
UP-026 is absorbed into the asymmetric-field patch. The old tuple backport and
pre-refresh union-API adapter are omitted. Imported function reads no longer
need UP-018 on upstream, but its redeclaration checks are retained.

Dependencies: association → key-aware metatables → navigation → presentation;
nullable dictionaries → optional/union dictionary keys (shared implementation);
class/array compatibility → inline call assertions. Read/write fields no longer
carry the metatable series as an incidental prerequisite. All runtime options
retain their previous defaults. Work-only tests cover feature combinations,
including factory returns and directional write contracts.

Validation before packaging: 2,616 combined workspace/all-feature tests pass,
plus all twelve retained inline-assertion regressions and twenty dictionary
integration tests after separating standalone fixtures. Combined Clippy and
formatting pass. Debug and release server replay both report zero diagnostics
for the frozen LTN request_processor.lua and interface.lua, with the module's
existing unnecessary-assert suppression and no additional disabled diagnostics.
Original mod files are read-only. No interactive IDE installation was performed.

The canonical contribution/test/push matrix and final package details are in
the workspace documents `upstream/FIX_INVENTORY.md` and
`upstream/ANALYZER_REFRESH_2026-10-13.md`, with machine-readable evidence under
`build/investigations/analyzer-refresh-20261013/` in the primary analyzer checkout.
EmmyLua2 build 150 is the refreshed package target; FDE remains 0.7.8-dev.

Older sections below are historical.

# EmmyLua IntelliJ development handoff

## Current checkpoint: October 13, 2026 — named union parameter aliases

UP-041 is independent of the dictionary and numeric-arithmetic fixes.
Standalone branch `fix/union-alias-parameters` starts at upstream `a0ce1512`;
signed fix `20c29828` is integrated as signed cherry-pick `f0413972` with
provenance. It addresses the two LTN `script.raise_event` warnings in
request_processor.lua:811 and :910.

The parameter checker expands named union aliases with the existing alias
resolver, requires every argument alternative to match an accepted alternative,
and reuses explicit-union leaf checks. Active type-pair tracking and a depth
bound prevent recursive aliases from looping. General compatibility, casts,
assignments and strict subtype checks retain their previous behavior. The
work branch already exposed the alias helper; no integration adapter is needed.

Five regression groups cover enum/class values, alias chains, argument unions,
nullable and incompatible values, literal alternatives and recursive aliases.
Standalone validation passes all 2,417 workspace tests, Clippy and formatting.
Tests run serially after an earlier unrelated formatter benchmark exceeded its
10 ms threshold under parallel execution. All 2,596 combined workspace tests,
Clippy and formatting pass. Packaged validation is recorded in the workspace
handoff and EmmyLua2's PATCHED_BUILD.md.

EmmyLua2 build 148 is the delivery; FDE retains version/archive 0.7.8-dev with
its development analyzer pin updated. Keep source fixes separate from packaging.
No new PR or push was requested. Reproduction snapshots and protocol captures:
`build/investigations/event-type-union-20261013/` in the main analyzer repository.

## Previous checkpoint: October 13, 2026 — nullable writes and numeric aliases

Inventory IDs: UP-016 (`fix/nullable-table-index`) and UP-040
(`fix/numeric-alias-arithmetic`). Keep the canonical workspace inventory at
`upstream/FIX_INVENTORY.md` current after upstream tasks.

The old dictionary branch already accepts nullable writes. Signed `ba8080c2`
adds its missing regression coverage; the fault was in the semantic-rewrite port.
Work correction `4bee6720` distinguishes dictionary write targets (`T | nil`)
from read/value queries, preserving required object/class fields and directional
write contracts. The non-nil arm must still match T. Both strict.tableIndex
settings, table/object/generic-alias dictionaries, invalid values and required
fields are covered. The standalone branch remains on its pre-refactor base and
needs porting before submission; do not cherry-pick the entire work branch.

Arithmetic fix `bb9e92f7` is independently based on fresh upstream `a0ce1512`.
It recognizes integer aliases in addition, subtraction and multiplication,
including alias chains, generic aliases and integer-only unions. Mixed/nullable
operands do not become integers; division/power and nominal operator overloads
retain their behavior. Alias traversal is bounded. Signed work cherry-pick
`34a147b2` records provenance and avoids importing unrelated upstream changes.

Validation: arithmetic standalone passes all 1,453 code-analysis tests, including
five new groups. The combined work workspace passes all 2,591 tests with all
features, including the two dictionary regression groups and an LTN-shaped
integration test. Both current implementations pass workspace/all-target Clippy
with warnings denied and formatting checks. The old dictionary branch passes
all 11 table-index tests. Generated Chinese stdlib translations from the full
test run were restored to their original tracked contents.

Original mods stay read-only. Fixed source snapshots, before/after raw LSP probes
and test logs are under `build/investigations/numeric-assignments-20261013/` in the
main repository. Packaging/runtime results are recorded in EmmyLua2's
`PATCHED_BUILD.md` and the workspace handoff. Build 147 is the intended EmmyLua2
delivery; FDE retains its version/archive, with only its development analyzer
pin updated. New commits remain local; no PR or push was requested.

## Previous checkpoint: October 13, 2026 — LTN length, guards and constructors

Seven local fix branches address the four request_processor.lua reports. Each starts
on freshly fetched upstream/main `119d0e10`; inline assertion arguments explicitly
stacks prerequisites, as described below. No upstream PR or push is authorized for
this batch. Original mods remain read-only; the fixed replay input is under
`build/investigations/ltn-batch-20261013/source` (37 Lua files plus a manifest).

| Report / cause | Standalone branch / signed commit | Signed work backport |
| --- | --- | --- |
| `#selected_trains` returns the operand type | `fix/builtin-length-result` / `383bcf93`, `2ed70489` | `27cd62db`, `c4acfd51` |
| Actual argument aliases fail call matching | `fix/aliased-call-arguments` / `0dbba984` | `157872f5` |
| Member discriminant comparison fails to expand an alias | `fix/aliased-member-discriminants` / `a325c369` | `c26a0d23` |
| Computed collection reads lose the preceding guard | `fix/computed-index-guards` / `817fdd25` | `ae218be4` |
| Constructor validation selects an incompatible union arm | `fix/union-table-constructor-checks` / `a5935c2d` | `9b964a2d` |
| Numeric-index classes are rejected for array parameters | `fix/class-index-array-compatibility` / `498c8f49` | `b68587d5` |
| Call argument cache and shape checks ignore inline assertions | `fix/inline-assertion-arguments` / `83e7e455` | `fee55890` |

Integration uses signed cherry-picks with provenance to avoid importing unrelated
upstream changes. Adapters retain this work branch's Arc/into_vec APIs and existing
read/write, strict dictionary, enum and callback fixes. Test append conflicts
retain both sides. The new argument assertion gate also wraps union shape checks.

The inline assertion branch contains PR #1290's scope fixes (`36821c98`, `0c436790`,
cherry-picks of `5aedfb81`, `74d683da`) and the numeric-index class prerequisite
(`7512adec`, from `498c8f49`). Scope isolation prevents an assertion on an outer
constructor leaking into a nested call. The array fix preserves existing test
#573 when its class assertion starts being honored. Submit these prerequisites
first; they are already integrated in the work branch.

Computed guards are restricted to stable local access paths and ordinary arrays
or dictionaries; writes, possible mutations, calls, joins, loops and closures
invalidate their use. Standalone array regressions reproduce upstream; work-only
`30b5d9f6` adds strict dictionary/array flag combinations, nested guarded lookups,
stale guards and asymmetric read/write contracts. `1ae5063f` checks asserted
recursive union constructors alongside ordinary constructors and bad arguments.

The first packaged replay exposed two required follow-ups. A declared aliased
argument was rejected before discriminant narrowing: formal aliases expanded to
string while actual aliases remained opaque. Existing assignment compatibility
now handles aliases at the leaf matcher, preserving generic identity, rejecting
wrong/nullable/mixed/cyclic aliases and retaining nominal class compatibility.
This is independent upstream behavior, exposed by the work branch's precise
dictionary keys. The original upstream mod replay had unknown keys and hid it.
Five focused tests and the combined real-mod replay restore SignalID? before the
guard and SignalID on its guarded member access, clearing line 618.

The second length projection path inferred an unannotated member's expected type
from the coarse operand shell. It now uses full expression inference for length
assignments (including parentheses), preserving explicit annotations and custom
operators. Two regression groups and real-mod replay clear init.lua:289. The
completed standalone length branch passes all 2,410 tests, Clippy and formatting.

The length fix preserves custom len operators and safe array[#array] reads after
nonempty guards, including existing regression #886. Union checking validates a
complete alternative, rather than combining fields from different alternatives;
unfiltered trial diagnostics prevent disabled warnings from selecting an invalid
arm. Class-to-array compatibility requires a real numeric index signature with
compatible elements; a lone [1] field, string-only signature or len operator is
insufficient. Inline assertions use the asserted contract; unrelated nested or
sibling errors and incompatible asserted parameter types still report errors.

Standalone workspace/all-feature totals: initial length 2,408; aliases/guards 2,405 each;
union 2,407 (final alias refinement 2,406 plus the previously passed slow stress
case); class arrays 2,406; inline arguments with prerequisites 2,425; aliased call arguments 2,407. Clippy with
all targets/features and warnings denied, plus rustfmt, pass. Per-fix build logs
and INVESTIGATION.md live in their exclusive build/checkouts directories.

All 2,583 final combined workspace/all-feature tests pass. The work-only guard tests
needed two allocation-free string comparisons to satisfy Clippy; their targeted
rerun and final workspace/all-target/all-feature Clippy plus rustfmt pass.
Evidence is in build/logs/ltn-batch-*. Signatures for all source fixes/backports
were verified. Target is EmmyLua2 build 145 only; FDE remains 0.7.8-dev with its existing archive unchanged.
Final package replay, archive checks and plugin tests belong in Emmy
PATCHED_BUILD.md and FDE docs/ACCEPTANCE.md. Custom binary remains macOS arm64 only.

Known separate issue: both literal diagnostic passes check every on_event overload
at init.lua:328/337. This predates this batch. The preliminary replay changes
1,147 misleading diagnostics into 466 at the same already-broken calls, rather
than introducing new failing locations. A correct follow-up needs joint overload
selection and negative tests; no overload patch or PR was created. Evidence is
build/investigations/ltn-batch-20261013/OVERLOAD_FOLLOWUP.md. Uncast LocalisedString
constructors containing nullable values still receive type diagnostics. Correct
call inference also exposes an undefined-field warning at request_processor.lua:879
for prototypes[item_info.type], whose declared SignalIDType is broader than just
item/fluid. No further change is included for that separate index/type-contract issue.

## Previous checkpoint: October 13, 2026 — callback return aliases and tuple tails

LTN `tools.printmsg` rejects a valid `msg_func = fun():LocalisedString` callback,
including the constructor return at request_processor.lua502. The failure
reproduces on current upstream119d0e10 without our local patches. The higher-order
function solver misses assignment semantics through union aliases; table-to-tuple
checking also treats the variadic tail as a mandatory single field.

- Local-only fix/callback-return-type, based on freshly fetched upstream/main
  119d0e10; signed/verified13cef7ff. Checkout build/checkouts/callback-return.
- Signed backport4acdc110 onto work/intellij-modules. The appended-test
  conflict retains both sets of tests. Two union iterators use this branch's
  into_vec().iter() instead of upstream iter().
- Upstream prerequisite3e9bc8d6 (tuple arguments use assignment element checks)
  was missing from the work baseline; signed backport3fde3155 includes it and
  applies the same element context to the new variadic tail. Its two existing
  regressions are preserved. No broad upstream refactor was imported.
- No dependency on the pending strict dictionary, read/write, or enum fixes.

The solver expands alias pairs with cycle/depth guards, checks every actual union
alternative against an allowed expected alternative, and uses assignment checks
for leaf values. Table constructors allow an empty repeated tuple tail and check
all present numeric tail members; fixed slots and fixed packs retain their checks.
Six regression groups cover recursive message tables, scalar/alias returns,
invalid headers and tails, nullable returns, callback parameter variance and cycles.
Standalone all2408 workspace/all-feature tests, Clippy(-Dwarnings) and fmt pass.
Combined all2544 workspace/all-feature tests pass; Clippy/fmt evidence is
recorded in build/logs/callback-return-* and the packaging notes. The standalone server removes all8msg_func warnings on37copied LTN files;
final no-regression comparison must use the combined packaged server.

Build target EmmyLua2 144 only; FDE stays0.7.8-dev, development pin/tests updated
without changing its archive. Nothing pushed or installed. Original mods readonly.
The ignored standalone build/INVESTIGATION.md records evidence and existing
limitations (generic call-checker fallback and coarse inline-return assertion
inference are separate). Final package checks belong in Emmy PATCHED_BUILD.md
and FDE docs/ACCEPTANCE.md.


## Previous checkpoint: October 13, 2026 — nominal enum member compatibility

`defines.rail_direction.front/back` retains its precise class type but was
incorrectly rejected as an argument to a `defines.rail_direction` parameter.
Upstream refactor3474c99d omitted enum member-value compatibility; work-only
3f6ecb0f preserves inline API member classes and exposes that omission. Plain
upstream hides the direct-access case by collapsing the member to the enum, but
explicitly typed members and predeclared class-valued enums reproduce it there.

- Local-only `fix/nominal-enum-member-compatibility`, based on freshly fetched
  upstream/main cb05b0c0; signed/verified c6446351.
- Checkout `build/checkouts/nominal-enum-members`; ignored investigation and
  future PR description under build/. Do not push/open a PR yet.
- Signed/verified provenance-preserving backport dd161a18. The only conflict
  retained this work branch's existing into_vec() iteration instead of upstream's
  iter(); no other source adaptation or dependency on strict dictionary indexing.

The checker resolves the enum's actual doc owner and initializer in its defining
file, accepting its nominal member classes and their subclasses. It preserves
private class identity and rejects unrelated classes, other enums, nil and
recursive/unresolved enum members. Union arguments must have every alternative
accepted, so Member|nil does not become valid for a required enum. Primitive enum
value checks remain in their existing paths. Seven regression groups cover
calls/assignments, local/global/nested and cross-file enums, optional parameters,
inheritance and negative cases. The two initial reproductions fail before fixing.

Standalone: all2409 workspace/all-feature tests plus workspace/all-target/
all-feature Clippy(-Dwarnings) and rustfmt pass. Combined work passes all2536
workspace/all-feature tests and the same Clippy/rustfmt checks. Logs are in
build/logs/rail-enum-{workspace-tests,clippy,fmt}. Packaging target is EmmyLua2
build143 only. FDE stays0.7.8-dev and its archive is preserved; only its development
pin and integration evidence change. Package and real-mod protocol results are
recorded in Emmy PATCHED_BUILD.md and FDE docs/ACCEPTANCE.md.

## Previous checkpoint: October 13, 2026 — unbound-table field diagnostics

User reported UndefinedField at LTN request_processor.lua:475/482/490 on
`seen_train[train.id] = true`. The key still has its declared uint32 type. The
checker incorrectly bound the unannotated `seen_train = {}` to the nearest
preceding class (ltn.RequestProcessor); its existing conditional-access exception
hid the corresponding warning in the `if not seen_train[train.id]` reads.

- Standalone local-only branch `fix/unbound-table-field-diagnostics` on freshly
  fetched upstream/main `cb05b0c0`, signed/verified `fb18e8d6`.
- Checkout `build/checkouts/unbound-table-fields`; ignored future description
  `build/PR_DESCRIPTION.md`. Do NOT push or open a PR yet.
- Signed/verified backport `3cbe6449` on work/intellij-modules. Test-file append
  conflict resolved by retaining both sets of tests; production fix applies
  unchanged. This does not depend on strict dictionary indexing or other local
  patches.

The checker now uses the existing doc-owner identity rather than a name or
nearest-position heuristic. Three grouped regressions cover the numeric-index
loop, unrelated enum/alias and same-name/case collisions, plus retained errors
for genuinely annotated classes/enums with different runtime variable names.
Two positive regressions fail before the fix. All 2,405 standalone workspace
all-feature tests, workspace/all-target/all-feature Clippy (-D warnings), and
rustfmt pass. Combined work also passes all 2,529 workspace/all-feature tests and the same
Clippy/rustfmt checks. Generated std-zh_CN translations were restored after testing.

Packaging target: EmmyLua2 build142 only. FDE stays0.7.8-dev with its existing
archive; only its development analyzer pin and integration evidence are updated.
Current package/protocol evidence belongs in Emmy PATCHED_BUILD.md and FDE
ACCEPTANCE.md, with parent agent updating the root workspace HANDOFF.md.


## Previous checkpoint: October 13, 2026 — computed-index assignment flow

User reported NeedCheckNil at LTN request_processor.lua:432 immediately after
`free_trains_by_priority[key] = free_trains_by_priority[key] or {...}`. The
rewritten flow projection does not associate a nonliteral computed key with the
preceding write. This also reproduces on unmodified upstream with Group[] and
default strict.arrayIndex, independent of our strict.tableIndex feature. Upstream
currently does not preserve nullable generic dictionary values consistently, so
the standalone reproduction intentionally uses arrays.

- Local-only branch `fix/computed-index-assignment-flow`, based on freshly fetched
  upstream/main `cb05b0c0`, signed `9fdb3dde`.
- Checkout `build/checkouts/computed-index-flow`; future submission description
  is ignored `build/PR_DESCRIPTION.md`. Do NOT push or open a PR yet.
- Signed backport `3ee94cbc` on work/intellij-modules. Flow dispatch conflict was
  resolved by retaining existing directional/dictionary handlers and adding the
  new helper before them. No feature dependency or other source adaptation.

The helper tracks local owner/key declaration identities through bounded
straight-line flow. A definite nonnil write removes nil from the original
array/dictionary element type, preserving named element docs and navigation.
Unknown/nullable writes do not narrow. Calls, alias-capable indexed writes,
reassigned/shadowed locals, multiple assignments, joins/loops and closure
boundaries conservatively stop the trace. Nominal read/write classes are left to
existing flow handling. Five tests cover safe reads, exact Group type and unsafe
cases. The original array reproduction fails before the patch.

Standalone validation: all 2,407 workspace tests with all features, workspace/
all-target Clippy (-D warnings), and rustfmt pass. Work adds two integration tests
for strict.tableIndex on/off, strict.arrayIndex on/off, exact PriorityGroup type
and retained unsafe lookup diagnostics. Both integration tests pass. The combined workspace passes all 2,526 tests
with all features, workspace/all-target/all-feature Clippy (-D warnings) and
rustfmt. Test-generated std-zh_CN files were restored.

This work also integrates reviewed PR follow-ups from the previous task:
- PR1291: 1550a05a -> e538efd3, builtin dictionary identity/stdlib origin guard
  and user-defined table<T>/compound regression coverage.
- PR1292: b1045520 -> 6ad16119, borrowed enum declaration, clarified comments and
  same-file/dynamic/conditional regression coverage.
The initial dictionary/enum fixes described below are now draft PRs1291/1292;
those original no-push notes are historical. The new computed-index fix remains
local-only at the user's explicit request.

Packaging targets EmmyLua2 build141 and FDE0.7.8-dev; final validation and archive
paths are recorded in PATCHED_BUILD.md, FDE docs/ACCEPTANCE.md and root HANDOFF.md.

## Previous checkpoint: October 13, 2026 — dictionary constructors and enum tables

Two independent diagnostic fixes are prepared on local branches based directly
on freshly fetched upstream main `cb05b0c0`. Neither branch was pushed and no PR
was opened, per the user's request to defer further upstream submissions.

- `fix/dictionary-constructor-fields`, signed `cdf32d10`, checkout
  `build/checkouts/dictionary-constructor`: do not treat `table<K,V>` as the
  standard library table class's closed record surface. Matches the existing
  assignment checker's dictionary policy, without adding new dictionary key/value
  validation. Tests use the real standard library and cover local/global,
  computed/literal keys, alias/call/reassignment, and retained class/generic-class
  missing/injected-field warnings. Two regressions fail before fixing.
- `fix/cross-file-enum-members`, signed `7727a58f`, checkout
  `build/checkouts/cross-file-enum`: resolve enum runtime declarations in their
  owning files, then query their table constructors' members. Enum values and
  misspelled fields remain invalid, including conditional accesses. Value syntax
  in the existing fallback is also resolved in its own file. Positive cross-file
  regression fails before fixing; negative tests cover enum-typed globals,
  parameters and shadowing locals.

Signed backports `5c39ca41` and `14ac6b9e` integrate both fixes without conflicts,
retaining all existing patches and the Arc baseline. No compatibility glue was
needed. Branch-local ignored `build/PR_DESCRIPTION.md` files hold future PR text;
keep packaging and internal documentation out of the independent fix branches.
Both faulty checks originate in upstream rewrite `3474c99d`.

Standalone validation: all 639 dictionary and 638 enum checker/type-check tests
pass, plus workspace/all-target/all-feature Clippy with warnings denied and
rustfmt. Combined validation passes all 2,515 workspace tests with all features,
the same Clippy checks and rustfmt. Test-generated std-zh_CN files were restored.
When sharing a Cargo target across worktrees, touch crate entrypoint mtimes to
avoid stale test artifacts and verify new tests actually run; one initial enum
baseline found zero tests, and the corrected run reproduced the failure.

Packaging targets EmmyLua2 `0.30.0-140-IDEA262-patched-modules` and FDE
`0.7.7-dev`, pinning this committed documentation tip. Final package validation
is recorded in EmmyLua2 `PATCHED_BUILD.md`, FDE `docs/ACCEPTANCE.md` and workspace
`HANDOFF.md`. Custom server changes remain macOS arm64 only.

Evidence: work `build/logs/constructor-enum-*`, standalone `build/logs/`, main
analyzer `build/investigations/constructor-enum/probe.py`. It copies all 37 LTN
Lua files, checks constants/request_processor/cache diagnostics, and records
hover. Baseline packaged139 reproduces all 17 constants constructor warnings
and the reported enum warnings. Original mod files and ordinary IDE profile are
untouched. Nothing from this task is pushed.

## Previous checkpoint: October 13, 2026 — table rendering and inline assertions

Two independent upstream fixes address the LogisticTrainNetwork reports:

- [PR #1289](https://github.com/EmmyLuaLs/emmylua-analyzer-rust/pull/1289),
  `fix/render-explicit-generic-arguments`: signed commits `47d734a5` and
  review follow-up `eb8004e2`. The renderer preserves supplied generic arguments
  even when the standard-library declaration has no generic parameters, fixing
  `table<>` display. Existing defaults, aliases and diagnostics remain covered.
- [PR #1290](https://github.com/EmmyLuaLs/emmylua-analyzer-rust/pull/1290),
  `fix/inline-as-expression-scope`: signed commits `5aedfb81` and review
  follow-up `74d683da`. Inline `@as` applies only to its attached expression,
  retaining assignment assertions without casting constructor fields, call
  arguments or unrelated later reads. This fixes false FreeTrain mismatches.

Both independent branches start at upstream main `cb05b0c0`. Both draft PRs
were pushed and their initial bot reviews addressed before integration. Preserve
current PR status. Signed cherry-picks `71c30d1c`, `6c341ed2`, `f5ee2ad1` and
`18ea33d9` bring only these fixes onto the existing development baseline, leaving
upstream's unrelated Arc-to-Rc conversion out. Integration retains the earlier
named-cast fallback and uses Arc for this baseline's closure types.

Work-only compatibility commit `581ba186` removes ambient inline assertions from
our read/write-field indexed-read walker as well. Its regression checks explicit
assertions and unaffected directional/optional reads. Keep this integration glue
out of the independent PRs. All prior internal patches remain present.

Validation: all 2,510 combined workspace tests with all features pass, as do
workspace/all-target/all-feature Clippy with warnings denied and rustfmt.
Standalone original fixes passed 2,405 and 2,411 workspace tests respectively;
review follow-ups (comments/tests only) pass all five renderer and twelve inline
assertion regressions plus Clippy/fmt. Eight inline regressions fail on unchanged
upstream. Test-generated standard-library translations were restored.

Packaging targets EmmyLua2 `0.30.0-139-IDEA262-patched-modules` and FDE
`0.7.6-dev`, both pinning this work branch's committed documentation tip. Final
package validation is recorded in EmmyLua2 `PATCHED_BUILD.md`, FDE
`docs/ACCEPTANCE.md` and workspace `HANDOFF.md`. Custom server patches remain
macOS arm64 only. Ordinary IDE installations and original mods are untouched.

Evidence: `build/logs/table-annotations-*` here; standalone checkout `build/logs/`
and main repository `build/investigations/table-annotations/`. The real-mod
probe copies all 37 Lua files and uses generated Factorio API definitions. Before
fixing, it reproduces table<> display and three false constructor-field warnings.
The separate call-return inference limitations explored during investigation
are not expanded by this correction; direct expression/declaration/member query
regressions define the tested scope. Only contribution branches are pushed.

## Previous checkpoint: October 12, 2026 — flow-aware union member arguments

[PR #1286](https://github.com/EmmyLuaLs/emmylua-analyzer-rust/pull/1286) contains
original `430fe1de` and review follow-up `272d3096` on the independent
`fix/flow-aware-union-member-arguments` branch. Signed merges `b62f2f41` and
`3862d97d` integrate them here. The missing-member argument check now queries
the owner's flow type after casts, guards and assignments, rather than its
original declared union. Eleven regressions cover narrowing/widening, residual
unions, nested/recursive owners, actual field mismatches and branch isolation.
A declared-nonunion fast path is intentionally absent: casts can widen it to a
union with missing members, and the regression confirms that distinction.

Combined validation exposed a compatibility gap with PR #1285: ambiguous union
members have no unique member ID, so skipping all named expression casts lost
their immediate cast. Follow-up `4dbecfe5` on `fix/named-cast-expression-scope`
restores it only for the exact static member path and resolved root declaration,
when ordinary member tracing has no unique target. Signed merge `525a6f0f`
integrates it here. Three regressions cover owner/path isolation, dot/string keys,
shadowing, owner/member reassignment and sixteen-level recursive paths. Both
contribution branches were pushed independently; both PRs are ready for review
after draft creation. Keep their current review status.

Validation passed all 2,395 standalone PR #1285 workspace tests and all 2,492
combined workspace tests with all features, workspace/all-target Clippy with
warnings denied and rustfmt. The initial PR #1286 source passed all 2,391
standalone tests; its comment/test-only review follow-up passed all eleven
focused regressions and the same lint checks. All earlier internal patches
remain present. Test-generated standard-library translations were restored.

A copied real-mod LSP probe verified the original LogisticTrainNetwork argument
sites at lines 365/366 with no target diagnostics. Packaging targets EmmyLua2
`0.30.0-138-IDEA262-patched-modules` and FDE `0.7.5-dev`; both lockfiles identify
this work branch's committed documentation tip. Final package validation is
recorded in EmmyLua2 `PATCHED_BUILD.md`, FDE `docs/ACCEPTANCE.md` and workspace
`HANDOFF.md`. The custom server remains macOS arm64 only.

The immediate ambiguous-member fallback does not add persistence after an
intervening statement; that limitation was reproduced before PR #1285 as well.
A separate pre-existing later-member-write inference issue also remains outside
this correction. Full investigation and evidence are in the named-cast checkout's
`build/investigations/union-owner/REPORT.md` and `build/logs/union-owner-*`.
Combined evidence is in `build/logs/union-argument-*`, including the preserved
initial recursive-case failure and passing final workspace run. Protocol evidence
is in the main repository's `build/investigations/union-member-argument/`.

## Previous checkpoint: October 12, 2026 — PR #1285 review follow-up

Draft [PR #1285](https://github.com/EmmyLuaLs/emmylua-analyzer-rust/pull/1285)
now includes signed follow-up `3b0d68319ea3c206425f74ef5ed259d3a000a166`.
It was pushed normally to `fix/named-cast-expression-scope`; the PR remains a
draft. The source branch contains only the original fix and review follow-up.
Signed merge `697f751d` brings the exact follow-up into `work/intellij-modules`
without conflicts or other changes.

The VM now represents index operands as `Static(member)` or `Dynamic(syntax)`
and asserts stack shape in debug builds. Flow comments and the local-type test
helper are clearer. Eight regressions cover the original cases plus named owner
casts, keyless statement casts, valid nil/never key results, nested computed keys
and recursive dependencies. Flow queries compute initially and cache by syntax
and offset; ordinary inference has an in-progress cycle guard. Unknown/Any still
fall back to the VM's useful key type; valid nil/never results are retained.
Fresh isolated workspaces for the strictness cases cost under a second together.

The standalone branch passes 2,392 workspace tests with all features, workspace
Clippy with warnings denied, and rustfmt. The combined work branch passes all
2,478 workspace tests, the same Clippy checks and rustfmt. Evidence is recorded in
`build/logs/cast-review-{workspace-tests,clippy,fmt}.log`; standalone evidence is
in `build/checkouts/named-cast/build/logs/review-*`.

Packaging targets EmmyLua2 `0.30.0-137-IDEA262-patched-modules` and FDE
`0.7.4-dev`. Both pins identify this work branch's committed documentation tip.
The previous build 136 / FDE 0.7.3-dev was completed and validated before this
review follow-up. Only the macOS arm64 server carries internal patches. Final
package results live in EmmyLua2 `PATCHED_BUILD.md`, FDE `docs/ACCEPTANCE.md` and
workspace `HANDOFF.md`. Keep packaging/docs off the upstream fix branch.

## Previous checkpoint: October 12, 2026 — named cast expression scope

The standalone branch `fix/named-cast-expression-scope` contains signed commit
`b9a3d37552b0eccde8ff9a146b0f25eba9ffec26` directly on freshly fetched upstream
main `4702ac2764e0acb38550e1a9e3679126db858e61`. Its checkout is
`build/checkouts/named-cast`. Keep local documentation and packaging out of it.
Draft [PR #1285](https://github.com/EmmyLuaLs/emmylua-analyzer-rust/pull/1285)
is open for user review. The branch was pushed to the user fork; retain draft
status until the user requests otherwise. Its body matches the checkout's
ignored `build/PR_DESCRIPTION.md`.

The fix prevents named `---@cast` effects from being applied as inline casts to
unrelated expressions. Dynamic key inference now uses the key's flow-sensitive
type, preserving casts on member keys such as `position.index`. The original
LogisticTrainNetwork example now gives `ltn.FreeTrain?` for the lookup and
`ltn.FreeTrain` after `assert`, without changing the mod or its casts.

Signed merge `51288958` applies this branch to `work/intellij-modules`, including
upstream's two intervening commits (ignore rules and removal of the
`skip_table_fields_check` attribute). The merge preserves our build ignore rule,
all local test modules and the existing dynamic-key implementation, using the
standalone fix's guard against replacing useful VM types with unknown/any.
The target-aware named-cast check and new regression tests are identical to the
standalone branch. All earlier internal patches remain present.

Validation:

- Standalone: two new regressions reproduced the upstream failure before the
  fix; all five final regression tests pass. All 2,389 workspace tests with all
  features pass (analysis and remaining crates run separately).
- Combined work branch: all 2,475 workspace tests with all features pass,
  including dictionary, optional-key and separate read/write-type regressions.
- Both branches: workspace Clippy (all targets/features, warnings denied),
  rustfmt, whitespace checks and a debug server build pass.
- Direct LSP probes confirm the original lookup/assert types on the standalone
  fix and combined work branch, with an additional work-branch run disabling
  strict array indexing. No live IDE test was needed for this source patch.

Evidence: each checkout's `build/logs/named-cast-*`; generated input and complete
LSP responses are in the main repository's
`build/investigations/cast-index/{fixed-upstream,fixed-work,fixed-work-array-off}`.
Tests regenerate standard-library translations; these were restored before
committing. The shared `build/pr-1266-target/debug/emmylua_ls` reflects the combined source.
Packaging this fix targets EmmyLua2 `0.30.0-136-IDEA262-patched-modules` and FDE
`0.7.3-dev`. Both analyzer lockfiles identify this work branch's final committed
revision; packaging validation is recorded in EmmyLua2 `PATCHED_BUILD.md`, FDE
`docs/ACCEPTANCE.md`, and the workspace `HANDOFF.md`. The custom release server
remains macOS arm64 only.


## Previous checkpoint: October 12, 2026 — diagnostic-refresh review integration

`work/intellij-modules` now merges `fix/cross-file-diagnostic-refresh` through
signed merge `49a9ed3f`. The standalone PR #1283 contains original patch
`8cbc71ca` and review follow-up `1c47a240`; the original was already present
internally as `03b54453`. The merge adds exactly the follow-up's ten-file diff.
All other internal patches and the upstream `dbb68f47` baseline are preserved.

The follow-up cancels refreshes before shutdown acknowledgement and on context
close, prevents retained update tasks from restarting them, and avoids retaining
the client from timer tasks. Close/removal, rename and configuration reload now
use the same capability-gated scheduler. Tests cover debounce timing with a
paused clock, client lifetime, mutation paths, capability gating and shutdown.
The protocol harness bounds process cleanup; this patch does not fix the existing
general stdio-worker teardown behavior.

Packaging targets EmmyLua2 `0.30.0-135-IDEA262-patched-modules` and FDE
`0.7.2-dev`. Both lockfiles identify the final internal analyzer commit. Evidence
uses `build/logs/refresh-review-*` in the active worktrees and FDE, with final
results in workspace `HANDOFF.md`, EmmyLua2 `PATCHED_BUILD.md`, and FDE
`docs/ACCEPTANCE.md`. The upstream PR branch is unchanged by this integration.
Only the macOS arm64 server is rebuilt with the internal patches.

Combined validation passed all 2,473 workspace tests with all features, rustfmt
and workspace Clippy (all targets/features, warnings denied). Generated
standard-library translations were restored after the tests before packaging.

## Previous checkpoint: October 12, 2026 — PR #1266 merged

The active branch `work/intellij-modules` is now based on upstream main
`dbb68f470f1bd7f7bd7761dc661242e794340be8`. Upstream merged PR #1266's exact
commit `ef3cf3c8`; it is part of the base rather than an outstanding local patch.
Upstream also merged callable-table and tuple/dictionary compatibility fixes
(PRs #1279 and #1280). All 20 remaining internal commits were replayed.

One conflict in `FactsBuilder::collect_setmetatable_calls` was resolved by
retaining our scan of all calls. This includes upstream's declaration/member
initializers as well as our standalone local-table calls. The other 19 patches
replayed unchanged. `git range-diff` confirms that scope, and all rebased commits
have verified signatures. The pre-rebase tip `78c3ea62` is preserved as
`backup/internal-before-merged-module-rebase-2026-10-12`; all independent fix
branches retain their previous tips. Use those branches when preparing the next
upstream contribution, after reviewing their changes against the current base.

All 2,471 workspace tests passed with all features, including the new upstream
callable/tuple/dictionary regressions. Rustfmt and workspace Clippy (all targets,
all features, warnings denied) passed.

This rebuild targets EmmyLua2 `0.30.0-134-IDEA262-patched-modules` and FDE
`0.7.1-dev`. Their lockfiles identify the final analyzer commit. Test/build
results are recorded in the workspace `HANDOFF.md`, FDE's `docs/ACCEPTANCE.md`,
and each repository's `build/logs/merged-module-*`. The active worktree, shared
caches and arm64-only packaging limitation remain as described below.

## Previous checkpoint: October 11, 2026

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

The integration also retains inline annotated API table members and nested event
class identities. Table-field functions keep their parameter/overload annotations,
overload matching accepts either member of an event-type/string-name union, and
nested callback returns do not leak into their enclosing factory's return type.
The regression covers numeric, named and string event callback registration.
These compatibility changes remain internal, separate from PR #1266.

Platform testing additionally exposed erased signatures for modules exporting
anonymous factories (`This, Framework = require('lib.init')()`), omitted return
types in inline-field hovers, and aliases inside union call parameters
(`uint32|string`). Internal follow-up preserves factory signatures, renders the
return type and uses the alias-aware checker when matching concrete alias
parameters. Regressions cover multiple returns, API return inference and hover,
and rejection of incompatible arguments. All 1,463 default-feature code-analysis
and 246 language-server tests pass with this follow-up.

The final navigation/hover follow-up retains annotated class identities in module
exports, including through factories, and shows known workspace-assigned fields
when hovering an open global `table`. It leaves the table's broad type contract
unchanged. All 1,463 default-feature code-analysis and 248 server tests pass;
final platform/full-workspace results are recorded in the workspace handoff.

Validation of this port: all 2,451 workspace tests passed with all features,
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
