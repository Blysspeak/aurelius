<!--
SYNC IMPACT REPORT
Version change: (unversioned template) → 1.0.0
Rationale: first ratification. The file previously contained only unfilled
placeholder tokens, so this is an initial adoption, not an amendment.

Principles defined (all new):
  I.   Data Durability First
  II.  One Local File, Many Processes
  III. Rust Clean Code (NON-NEGOTIABLE)
  IV.  Surgical Simplicity
  V.   Verify Before Done
  VI.  MCP Surface Stability

Sections added:
  - Quality Gates (replaces [SECTION_2_NAME])
  - Release & Versioning (replaces [SECTION_3_NAME])
  - Governance (filled)

Removed sections: none (all template placeholders were filled or dropped).

Template consistency:
  ✅ .specify/templates/plan-template.md — "Constitution Check" gate is generic
     ("[Gates determined based on constitution file]"); no edit required, the
     gate is now populated per-feature from this file.
  ✅ .specify/templates/spec-template.md — no constitution-driven mandatory
     section is added or removed by this ratification.
  ✅ .specify/templates/tasks-template.md — Principle V requires a regression
     test per bug fix; the template already carries a test task category.
  ✅ CLAUDE.md — already documents build/test commands and architecture; the
     quality gates below match the commands it lists.
  ⚠ README.md — roadmap and docs do not yet mention the backup guidance
     required by Principle I; tracked in feature 002-db-durability-hardening
     (FR-025), not a constitution follow-up.

Deferred: none.
-->

# Aurelius Constitution

Aurelius is a self-hosted knowledge graph that serves as long-term memory for a
developer and their AI agents. Everything below follows from one fact: **the data
in this system is not reproducible.** Source code can be rebuilt from git, caches
can be recomputed, but a decision recorded eight months ago exists in exactly one
place. That asymmetry sets the priorities.

## Core Principles

### I. Data Durability First

The database is the product. Availability, throughput and feature velocity are all
subordinate to not losing what is already stored.

- No operation MAY write to a database whose structural integrity has not been
  confirmed on that connection. A read error, a lock error, or a corruption error
  MUST NOT be interpreted as "the database is empty" or "the database is new".
- Every schema change MUST be atomic: the change and the version marker that
  records it commit together or not at all. A partially applied migration is a
  defect of the same severity as data loss.
- Write failures MUST propagate to the caller. `.ok()`, `unwrap_or_default()` and
  equivalent discards on a write path are prohibited.
- Any destructive step (dropping a table, rebuilding an index, deleting rows) MUST
  be reversible by transaction rollback, and MUST NOT be reachable by a code path
  that misclassified the database's state.
- The product MUST offer a supported way to snapshot a live database. If it does
  not, users will copy the file by hand, and that is the single most reliable way
  to destroy it.

**Rationale**: On 2026-07-27 a file-level copy over the live database produced a
file whose header described 181 pages while its body held 1781, and the code then
re-ran destructive migrations against it on every invocation for a day, because a
failed version read was silently treated as "brand-new database". Both halves of
that sentence are now prohibited by this principle.

### II. One Local File, Many Processes

Aurelius is a single global SQLite file on one machine, concurrently accessed by
many short-lived writer processes (CLI calls, editor hooks, git hooks) and several
long-lived servers (MCP servers, the graph viewer).

- Correct behaviour under this concurrency is the code's responsibility, never the
  user's. "Don't run two sessions at once" is not an acceptable mitigation.
- Every connection MUST wait a bounded time for contended locks instead of failing
  on first contention.
- Every connection MUST verify that the intended journaling mode is actually in
  effect, and MUST fail loudly when it is not — settings that are requested but
  silently refused are worse than settings that were never requested.
- Initialization MUST be safe when several processes race to perform it.
- The database location MUST be resolved through one shared definition. Divergent
  path logic across binaries is a correctness bug, not a style issue.

**Rationale**: The deployment guarantees contention — a hook spawns a writer on
every file edit. Code that assumes a single writer is code that loses writes.

### III. Rust Clean Code (NON-NEGOTIABLE)

- `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` and indexing that can
  panic are prohibited on runtime paths. They are permitted in `#[cfg(test)]`.
- Errors: `thiserror` for typed domain errors in library crates, `anyhow` at the
  application boundary. An error crossing a crate boundary MUST carry enough
  context to act on without reading the source.
- Errors MUST NOT be classified by matching on human-readable message text.
- Parse, don't validate: make illegal states unrepresentable in types rather than
  checking for them at each use.
- `unsafe` requires explicit prior approval.
- New dependencies require justification: what it replaces, why the standard
  library or an existing dependency is insufficient.

**Rationale**: These are the failure modes that turn a recoverable error into a
crash or a silent misbehaviour in exactly the layer that owns irreplaceable data.

### IV. Surgical Simplicity

- Write the minimum code that satisfies the requirement. No speculative
  abstraction, no configuration for a case nobody has, no handling of impossible
  states.
- Touch only what the task requires. Do not reformat, rename, or "improve"
  adjacent code in the same change. Do not delete someone else's dead code —
  mention it instead.
- If the change needs 200 lines where 50 would do, it is not done; it is a draft.

**Rationale**: Every extra line in the storage layer is a line that can be wrong
about data that cannot be regenerated.

### V. Verify Before Done

Done means demonstrated, not believed.

- A change is complete only after it has been exercised end-to-end by running the
  real binary against a real database — not by reasoning that it should work.
- Every bug fix MUST ship with an automated test that fails against the code
  before the fix and passes after it. "Tests were added" is not a claim; the
  before/after asymmetry is the evidence.
- Success MUST NOT be reported from a green feeling. Report the command that was
  run and what it printed. If a step was skipped, say it was skipped.

**Rationale**: The incident above was invisible precisely because nothing in the
project asserted that the database was still readable.

### VI. MCP Surface Stability

The MCP tool surface is a public contract consumed by AI agents in other people's
sessions, which cannot be migrated or notified.

- Tool names, their parameters and the shape of their results MUST evolve
  additively within a major version: add tools, add optional parameters, add
  fields.
- Renaming or removing a tool, making an optional parameter required, or removing
  a result field is a breaking change and requires a major version bump.
- A tool's documented description is part of the contract — agents route on it.
- Installed binaries MUST NOT expose tools that no committed source produces. If
  the shipped surface and the repository disagree, the repository is wrong and
  MUST be reconciled before the next release.

**Rationale**: A renamed tool does not raise an error the author will ever see; it
degrades someone else's agent silently, at a time of their choosing.

## Quality Gates

The following MUST all be green before a change is considered ready. Each is a
command with observable output, not a judgement call.

| Gate | Command | Applies to |
|---|---|---|
| Formatting | `cargo fmt --check` | every change |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | every change |
| Tests | `cargo test --workspace` | every change |
| UI build | `cd ui && npm run build` | changes under `ui/` |
| Real-database check | run the built binary against the live database | every change touching storage, and every release |

Additional rules:

- A change that touches the storage layer MUST be exercised against a real
  database file before it is merged, and against a deliberately damaged one when
  it claims to handle damage.
- Failing gates are fixed at the root. Suppressing a lint, deleting a failing
  test, or narrowing an assertion to make it pass is prohibited.
- New crates or binaries MUST be covered by the same gates from their first
  commit.

## Release & Versioning

- Versioning is semver, derived from conventional commits. All workspace crates
  share one version.
- Releases go through the project's release workflow: verify → bump manifests →
  update `CHANGELOG.md` → release PR → tag on the merge commit → GitHub Release.
  Never tag from a red build.
- `CHANGELOG.md` MUST record every release. Entries state what changed for the
  user, not which functions were edited.
- A release MUST NOT be tagged while the installed binary exposes functionality
  absent from the repository (see Principle VI).
- Publication to package registries happens only on explicit request.

## Governance

This constitution supersedes ad-hoc practice. Where it conflicts with habit,
convenience, or an in-flight change, this document wins.

- **Amendments** are made by editing this file, with the Sync Impact Report at the
  top updated in the same change, and dependent templates and docs brought into
  line at the same time.
- **Versioning of this document**: MAJOR for removing or redefining a principle in
  a backward-incompatible way; MINOR for adding a principle or materially
  expanding guidance; PATCH for clarifications and wording.
- **Compliance**: every plan produced by `/speckit-plan` MUST include a
  Constitution Check gate evaluated against the principles above, and every
  deviation MUST be recorded in that plan's Complexity Tracking table with the
  simpler alternative that was rejected and why. An unrecorded deviation is a
  defect.
- **Runtime guidance** for day-to-day development lives in `CLAUDE.md`. Where
  `CLAUDE.md` and this document disagree, this document governs and `CLAUDE.md`
  is corrected.

**Version**: 1.0.0 | **Ratified**: 2026-07-28 | **Last Amended**: 2026-07-28
