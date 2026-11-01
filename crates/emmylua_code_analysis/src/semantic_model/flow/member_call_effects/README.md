# Visible member call effects

The resolver follows same-file local functions and fixed member callees through
reaching assignments, actual dot/colon receivers and invocation order. A known
conflicting write invalidates a guard. Passing a receiver to an unresolved call
alone preserves the guard under the selected pragmatic call policy.

Receiver-path bindings carry table identities, not closure values. Calling a
formal callback parameter is therefore opaque even if the argument is a visible
closure. Tracking such callbacks requires callable-value bindings with their
caller environments; it is not implied by following a local function name.
Factory-returned closures likewise lack their closure-creation environments.

Dynamic callees, unresolved alias/receiver identities and interprocedural method
installation remain opaque. A rawset call invalidates its direct target but does
not install a callable value postcondition. Captured member replacements without
an invocation context, and reassigned name callees captured in another closure,
have no known runtime state and are not revived from stale initializers.

Syntax-only caches last for one immutable model snapshot. Semantic receiver and
captured-mutability caches publish only completed results. Invocation histories
share immutable binding snapshots and compare their complete semantic identities.
