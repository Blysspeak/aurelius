---

description: "Task list for Semantic Cluster Graph Layout"
---

# Tasks: Semantic Cluster Graph Layout

**Input**: Design documents in `/specs/001-graph-cluster-layout/`
**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/physics-module.md`, `quickstart.md`

**Tests**: Not requested by the feature spec. Verification is **manual** via `quickstart.md` (the acceptance criteria are visual). One scripted shell check (`grep`) is included as a tunability audit.

**Organization**: Tasks are grouped by user story. Foundational phase delivers the `physics.ts` module that all stories depend on.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: User story label (US1 / US2 / US3)
- File paths are absolute or repo-relative

## Path Conventions

- UI workspace: `ui/` (Vite + React + TypeScript)
- Affected source files (entire feature): `ui/src/components/graphCanvas/physics.ts` (new), `ui/src/components/graphCanvas/graphCanvas.tsx` (modified), `ui/src/vite-env.d.ts` (extended)

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Declare the new dependency surface and provide TypeScript types for the d3 force constructor we will use.

- [ ] T001 Verify `d3-force-3d` is listed under `dependencies` in `ui/package.json` (added during planning). If absent, run `cd ui && npm install d3-force-3d` and commit the resulting `package.json` + `package-lock.json` changes.
- [ ] T002 [P] Add an ambient module declaration for `d3-force-3d` covering `forceCollide` (with chained `radius`, `strength`, `iterations`) in `ui/src/vite-env.d.ts`. Mirror the surface used by `physics.ts`. No `@types/d3-force-3d` exists; this is the typed boundary.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Create the dedicated physics-configuration module. Every user story consumes it.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T003 Create `ui/src/components/graphCanvas/physics.ts` per the contract in `contracts/physics-module.md`. The module MUST export:
  - `PHYSICS` — frozen object with `charge { strength, distanceMax, theta }`, `center { strength }`, `collide { padding, strength, iterations }`, `sim { alphaDecay, velocityDecay, cooldownTicks, warmupTicks, alphaMin }`. Initial values from `data-model.md` "Entity: PhysicsConfig".
  - `LINK_BY_RELATION` (internal) and `LINK_DEFAULT` (internal) — relation→`{ distance, strength }` mapping populated per `data-model.md` "Entity: RelationForceTable".
  - `linkDistance(relation: string): number` — total function; falls back to `LINK_DEFAULT.distance`.
  - `linkStrength(relation: string): number` — total function; falls back to `LINK_DEFAULT.strength`; result in `[0, 1]`.
  - `applyForces(fg: any): void` — idempotent setup wiring `link`, `charge`, `center`, and a newly registered `collide` force per the contract's "Side effects" section. MUST NOT call `d3ReheatSimulation`. MUST NOT mutate node/edge data.
  - Inline JSDoc/comments explaining the role of each parameter group (mandatory for SC-005 / FR-008).

**Checkpoint**: `physics.ts` compiles under `cd ui && npx tsc --noEmit -p .`. The module has no consumer yet — `graphCanvas.tsx` is unchanged. Tunability acceptance for US3 is achievable from this point onward.

---

## Phase 3: User Story 1 — Cluster Readability (Priority: P1) 🎯 MVP

**Goal**: When the user opens the graph, each project's children settle into visually distinct sub-groupings by relation semantics — not a uniform ring around the hub.

**Independent Test**: Open `au view` on a database containing the boostix project (≥100 children of mixed types). Within 3 seconds, the layout stabilizes; an unprompted viewer can identify ≥ 2 visually distinct sub-clusters within the boostix cluster, and no node circles visibly overlap (`quickstart.md` Steps 1–3).

### Implementation for User Story 1

- [ ] T004 [US1] In `ui/src/components/graphCanvas/graphCanvas.tsx`, replace the body of the existing physics `useEffect` (currently keyed on `[graphData]` and applying `link/charge/center` numerics inline) with a single call to `applyForces(fgRef.current)`. Preserve the ref-presence guard. **Do NOT change the deps array yet** — that is US2's responsibility and keeping deps unchanged here means data refetches still reheat (correct fallback behavior even before US2). Remove the inline `fg.d3Force('link')?.distance(...).strength(...)`, `charge`, and `center` numerics from this file.
- [ ] T005 [US1] In `ui/src/components/graphCanvas/graphCanvas.tsx`, replace the inline simulation-prop numerics on `<ForceGraph2D>` (`d3AlphaDecay`, `d3VelocityDecay`, `cooldownTicks`, `warmupTicks`, `d3AlphaMin`) with references to `PHYSICS.sim.*` from `physics.ts`. Add the import. After this task, no numeric literal controlling simulation tempo remains in `graphCanvas.tsx`.

**Checkpoint**: `cd ui && npm run build` passes. Launch `npm run dev`, open the graph, and verify `quickstart.md` Steps 1–3:
- Step 1: clusters visible in boostix; no uniform ring; ≥ 2 sub-groupings identifiable.
- Step 2: stabilization within 3 s.
- Step 3: zero rendered overlap.

User Story 1 is functional and testable here, even if Story 2 has not started — search/filter may still reshake the layout (Story 2 fixes that).

---

## Phase 4: User Story 2 — Stable Layout Under Interaction (Priority: P2)

**Goal**: After the layout stabilizes, filtering, searching, or selecting nodes does NOT reshuffle positions. Drag/release returns the dragged node to free simulation without cascade.

**Independent Test**: With a stable layout, type into the search box and verify positions of non-matching nodes do not move. Drag a node, release it, verify it settles smoothly without ejecting neighbors (`quickstart.md` Steps 4–6).

### Implementation for User Story 2

- [ ] T006 [US2] In `ui/src/components/graphCanvas/graphCanvas.tsx`, decouple the lifecycle:
  - Change the deps array of the physics-setup `useEffect` (the one calling `applyForces` after T004) from `[graphData]` to `[]`. Keep the ref-presence guard.
  - Add a **new** `useEffect` whose body is `fgRef.current?.d3ReheatSimulation?.()` and whose deps are exactly `[nodes.length, edges.length]`. (Use the lengths from props, not from `graphData`, to avoid coupling reheat to the memoization chain.)

  After this task, the simulation is set up exactly once on mount and reheats only when the structural set of nodes or edges changes — never on `searchMatchIds`, `selectedNodeId`, `zoomToProject`, or `hoveredId` changes.

**Checkpoint**: Manual verification per `quickstart.md` Steps 4–6:
- Step 4: typing into search does not move non-matching nodes.
- Step 5: project filter does not reshuffle remaining nodes.
- Step 6: drag/release returns node to simulation, no neighbor cascade.

---

## Phase 5: User Story 3 — Tunable Physics Architecture (Priority: P3)

**Goal**: All layout parameters live in a single, well-documented module separate from the renderer. A developer changes one number in `physics.ts` and sees the effect without touching `graphCanvas.tsx`.

**Independent Test**: Audit grep finds zero raw layout numerics in the renderer; comment quality in `physics.ts` is sufficient for a newcomer to understand each parameter's role; HMR reflects a tweak to a single value (`quickstart.md` Step 8).

### Verification for User Story 3

- [ ] T007 [US3] Run `grep -E '(distance|strength|distanceMax|alphaDecay|velocityDecay|cooldownTicks|warmupTicks|alphaMin)\(' ui/src/components/graphCanvas/graphCanvas.tsx`. The output MUST contain zero raw numeric literals. Permitted: references to `PHYSICS.sim.*` or zero matches. If the grep flags a violation, return to T005 / T004 and move the leak into `physics.ts`.
- [ ] T008 [P] [US3] Open `ui/src/components/graphCanvas/physics.ts` and confirm each parameter group (`charge`, `center`, `collide`, `sim`, `LINK_BY_RELATION`) carries an inline comment block explaining (a) what it controls and (b) why the chosen value (referencing the spec topology). Add or refine comments if any group lacks this.

**Checkpoint**: Manual verification per `quickstart.md` Step 8 — change one value (e.g., `LINK_BY_RELATION.belongs_to.distance` from 80 to 200), save, observe HMR reflect the change without editing the renderer; revert.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T009 [P] Delete the working artifact `graph-after-physics-tweak.png` from the repository root (`rm graph-after-physics-tweak.png`). Required by FR-011 / `quickstart.md` Step 9.
- [ ] T010 Run the full `quickstart.md` protocol Steps 1–9 against a `npm run dev` instance. Capture an "after" screenshot for comparison against the pre-feature visual baseline (kept outside the repo, see `quickstart.md` "Reference: before state"). Record any deviations from acceptance criteria as follow-up issues.
- [ ] T011 Run `cd ui && npm run build` once more to ensure the production bundle still compiles after all edits and the bundle size has not regressed materially.
- [ ] T012 [P] Persist the architectural decision to Aurelius memory via `memory_session` summarizing: per-relation typed forces + `forceCollide` + lifecycle decoupling break the hub-and-spoke ring. Reference branch `001-graph-cluster-layout` and the `physics.ts` module path.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies. T001 must complete before T003 (Foundational needs the dep installed).
- **Foundational (Phase 2 — T003)**: Depends on T001 (dep present) and T002 (ambient types). **BLOCKS all user stories**.
- **User Story 1 (Phase 3)**: Depends on T003.
- **User Story 2 (Phase 4)**: Depends on T004 (US1's renderer wiring). T006 modifies the same `useEffect` that T004 introduces.
- **User Story 3 (Phase 5)**: T007 depends on T005 (US1 numerics removal) and structurally on T004. T008 depends only on T003.
- **Polish (Phase 6)**: T009, T012 independent. T010, T011 depend on all prior implementation tasks.

### Within Each User Story

- US1: T004 before T005 (T005 needs the import added in T004). Both in the same file — sequential.
- US2: T006 alone, depends on T004 having altered the same effect.
- US3: T008 can run anytime after T003; T007 runs after T005 and ideally after US2 too.

### Parallel Opportunities

- T002 is `[P]` — different file from T001 (T001 only verifies; T002 edits `vite-env.d.ts`).
- T008 is `[P]` — independent of the renderer changes.
- T009 is `[P]` — file deletion in repo root, no code dependency.
- T012 is `[P]` — memory write, no code dependency.
- Different file: `physics.ts` (T003) and `graphCanvas.tsx` (T004/T005/T006) — but T003 must complete first because the others import from it.

---

## Parallel Example: After Foundational (T003) is complete

Two independent tracks open up:

```bash
# Track A — wire the renderer (sequential within graphCanvas.tsx):
T004  → T005  → T006  → T007

# Track B — module quality + cleanup (no code dependency):
T008  (verify docstrings in physics.ts)
T009  (delete graph-after-physics-tweak.png)
T012  (memory_session to persist decision)

# Tracks A and B can be worked in parallel by different agents/developers.
```

---

## Implementation Strategy

### MVP First (User Story 1 only)

1. Phase 1: T001, T002 (Setup).
2. Phase 2: T003 (Foundational — creates `physics.ts`).
3. Phase 3: T004, T005 (wire renderer to `physics.ts`).
4. **STOP and validate**: run `quickstart.md` Steps 1–3. Clusters must read correctly.
5. Demo / commit MVP.

### Incremental Delivery

1. MVP (above) → first observable improvement (visual clusters).
2. + US2 (T006) → stability under interaction.
3. + US3 verification (T007, T008) → architecture audit pass.
4. + Polish (T009–T012) → production-ready state.

### Parallel Track Strategy

After T003 lands, the renderer track (T004 → T005 → T006 → T007) can run alongside the cleanup/docs track (T008, T009, T012).

---

## Notes

- This feature touches exactly three files: `physics.ts` (new), `graphCanvas.tsx` (refactor), `vite-env.d.ts` (one-block extension). Plus one file deleted (`graph-after-physics-tweak.png`).
- Acceptance is visual; `quickstart.md` is the source of truth for verification. Do not invent automated tests for force-directed layouts (brittle).
- Commit cadence: one commit per checkpoint (after Phase 2, after Phase 3, after Phase 4, after Phase 5, after Phase 6) keeps history readable and supports clean revert if a phase regresses.
- Branch is `001-graph-cluster-layout`. Final merge target is `main` after Polish.
