# Archived analyzer refresh probes

This branch preserves the uncommitted investigation checkout from the
2026-10-20/21 analyzer refresh against upstream/main bb7c7d4c.
It overlays local regression tests onto upstream and adapts their harness to
upstream APIs. Some tests are expected to fail because the corresponding fixes
are deliberately absent. The tableIndex configuration field is a probe harness
shim, not an implementation of dictionary-index behavior.

This is evidence, not a production fix or a passing integration branch. It is
not merged into work/intellij-modules or any plugin build. The actual fixes and
validation are tracked independently in the workspace's upstream/FIX_INVENTORY.md
and upstream/memory/ANALYZER_REFRESH_2026-10-21.md.

Archived on 2026-10-21 at the user's request to commit and push all outstanding
changes. No analyzer behavior was changed while making this snapshot.
