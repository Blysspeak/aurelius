# Quickstart: Verify Semantic Cluster Graph Layout

Manual verification protocol. The acceptance criteria are visual; this script defines what to look at and what to compare against.

---

## Preconditions

- The current Aurelius database (`~/.local/share/aurelius/aurelius.db`) contains at least:
  - One project hub with ≥ 100 children of mixed types (boostix is the canonical example at ~450 children).
  - Multiple projects so we can verify "islands".
  - A handful of `solves`/`caused_by`/`subtask_of` edges to exercise topical attractors.
- The UI bundle has been rebuilt: `cd ui && npm run build` (or `npm run dev` for live reload).
- The viewer is running: `au view` on port `7175`.

## Reference: before state

The pre-feature screenshot is `graph-after-physics-tweak.png` (working artifact, deleted as part of FR-011). Hold a copy outside the repo for the before/after diff. Visually: boostix children form a near-circular ring around the hub; no sub-clustering by type or relation is visible.

---

## Verification steps

### Step 1 — Layout reads as clusters (SC-001, FR-001..FR-003)

1. Open the graph at `http://127.0.0.1:7175/` in a fresh tab.
2. Wait for the layout to stabilize (≤ 3 s, see Step 2).
3. Look at the largest project's cluster (boostix today).
4. **Pass criteria**:
   - The cluster does **not** form a uniform ring around its hub.
   - At least 2 visually distinct sub-groupings are identifiable by eye within the cluster.
   - No two rendered node circles overlap each other.
5. Hover any sub-grouping to confirm the labels are semantically related (e.g., a Task and its WorkLogs; a Problem and its Solution).

### Step 2 — Stabilization time (SC-002, FR-004)

1. Open the graph in a fresh tab with the Network tab DevTools panel open (or just count seconds).
2. Start a timer when the first nodes paint.
3. **Pass criteria**: the layout stops perceptibly moving within 3 seconds.
4. Optional instrumentation: set `cooldownTicks` lower in `physics.ts` and observe — values below ~150 cause incomplete settling, confirming 250 is in the right neighborhood.

### Step 3 — No overlap (SC-003, FR-003)

1. With the layout stable, zoom into the densest hub.
2. **Pass criteria**: no node circle visually occludes another. Edge-to-edge contact is acceptable; pixel overlap is not.

### Step 4 — Stability under search (SC-004, FR-005)

1. With the layout stable, focus the search input.
2. Type a short query that matches ~10–30 nodes (e.g., `boostix`).
3. **Pass criteria**:
   - Highlight glow appears on matched nodes.
   - Non-matched nodes **do not move** — no perceptible re-shake of positions.
4. Clear the search.
5. **Pass criteria**: positions remain stable after clearing.

### Step 5 — Stability under filter (FR-005)

1. Click a project in the sidebar to filter.
2. **Pass criteria**: remaining nodes keep their relative positions (a viewport zoom-to-fit is fine; what we care about is that *the simulation does not re-heat and reshuffle*).

### Step 6 — Drag/release (FR-007)

1. Drag any node ~200 px and release.
2. **Pass criteria**:
   - The node returns to free simulation (it is not pinned in place after release).
   - The node settles within ~1 s.
   - No cascade ripple visible across the rest of the graph.

### Step 7 — Disconnected components stay on canvas (SC-006, FR-010)

1. Identify a node with no edges (or a tiny disconnected pair) — `memory_search` for orphans, or check `memory_gc` output.
2. With the layout stable, locate that node on canvas.
3. **Pass criteria**: it sits within the visible canvas region (≤ 10× viewport extent from origin). It is **not** drifting outward.

### Step 8 — Single-file tunability (SC-005, FR-008, FR-009)

1. Open `ui/src/components/graphCanvas/physics.ts`.
2. Change `LINK_BY_RELATION.belongs_to.distance` from 80 to 200.
3. Save. Browser should hot-reload (Vite HMR).
4. **Pass criteria**: tether length visibly increases — children float farther from their hubs. No edits to `graphCanvas.tsx` required.
5. Revert.
6. **Audit grep**: `grep -E '(distance|strength|distanceMax|alphaDecay|velocityDecay|cooldownTicks|warmupTicks|alphaMin)\(' ui/src/components/graphCanvas/graphCanvas.tsx` must return zero hits, OR only references to `PHYSICS.*` constants. (FR-008)

### Step 9 — Cleanup (FR-011)

1. `ls graph-after-physics-tweak.png` from repo root.
2. **Pass criteria**: file does not exist.

---

## Regression watch — things that should NOT change

- Project node labels visible at default zoom; non-project labels visible only on hover/focus/highlight.
- Color theme of nodes and edges.
- Edge label appearing on hover.
- Search/highlight glow rendering.
- Minimap consumer (`__aurelius_positions` window global).
- Dragging a node to reposition it (without releasing) still works.

---

## Rollback

If a critical visual regression appears after merge:
- `git revert <merge-commit>`.
- The pre-feature working state on the `main` branch is the experiment captured in `graph-after-physics-tweak.png` (i.e., the in-flight `graphCanvas.tsx` modification at HEAD before this branch). Note this is itself a worse state than the original release `v1.5.0` layout, so rollback target is the release tag, not the experiment.
