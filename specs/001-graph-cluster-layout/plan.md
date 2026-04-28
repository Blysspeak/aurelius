# Implementation Plan: Semantic Cluster Graph Layout

**Branch**: `001-graph-cluster-layout` | **Date**: 2026-04-28 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/001-graph-cluster-layout/spec.md`

## Summary

Replace the current uniform-force layout in `graphCanvas.tsx` with a relation-typed force model. `belongs_to` becomes a long, soft tether to the project hub; topical relations (`contains`, `subtask_of`, `solves`, `caused_by`, `blocks`, `related_to`, `uses`, `depends_on`) become short, strong attractors that visibly cluster siblings into sub-groups. Add a `forceCollide` so nodes never overlap. Move all force parameters into a dedicated `physics.ts` module so the renderer no longer carries layout numerics. Decouple force setup from `graphData` updates to prevent re-heat on filter/search/selection.

## Technical Context

**Language/Version**: TypeScript 5.8 (strict mode)
**Primary Dependencies**: React 19, `react-force-graph-2d` 1.26 (built on `force-graph` → `d3-force-3d` 3.0)
**Storage**: N/A (UI-only feature; reads from existing graph API)
**Testing**: Visual verification via `npm run dev` + Playwright snapshot for regression spotting (no automated test framework currently configured for the UI crate; manual quickstart steps documented)
**Target Platform**: Web (modern browsers with hardware-accelerated 2D canvas), served by `au view` on port 7175
**Project Type**: Web frontend (React + Vite)
**Performance Goals**: Layout stabilization ≤ 3 s on ~1,200 nodes / ~2,300 edges (SC-002); 60 fps interaction once cooled down
**Constraints**: No backend changes; no new heavy deps; ambient-only TypeScript surface for `d3-force-3d` (no `@types/d3-force-3d` published)
**Scale/Scope**: Up to ~1,200 nodes / ~2,300 edges in current production database; design must absorb 2× growth without quadratic blow-up (Barnes-Hut θ tuning + bounded `distanceMax`)

## Constitution Check

*Constitution file is an unfilled template (`[PROJECT_NAME] Constitution`).* No project-specific principles to gate against. Default engineering norms apply (single source of truth for parameters, no untested public API change, smallest possible diff). PASS.

## Project Structure

### Documentation (this feature)

```text
specs/001-graph-cluster-layout/
├── plan.md                  # This file
├── spec.md                  # Feature spec (already written)
├── research.md              # Phase 0 — research notes & decisions
├── data-model.md            # Phase 1 — config entities & relation→force mapping
├── quickstart.md            # Phase 1 — manual verification steps
├── contracts/
│   └── physics-module.md    # Phase 1 — exported API contract of physics.ts
├── checklists/
│   └── requirements.md      # Spec quality checklist (already written)
└── tasks.md                 # Phase 2 (created later by /speckit-tasks)
```

### Source Code (repository root)

```text
ui/                                            # Vite + React + TS (UI crate)
├── package.json                               # add: d3-force-3d (already installed)
├── src/
│   ├── vite-env.d.ts                          # ambient module decl for d3-force-3d
│   ├── components/
│   │   └── graphCanvas/
│   │       ├── graphCanvas.tsx                # MODIFIED: import physics, refactor lifecycle
│   │       ├── graphCanvas.module.css         # untouched
│   │       └── physics.ts                     # NEW: force config + applyForces()
│   ├── theme.ts                               # untouched (size/color theme stays)
│   └── types.ts                               # untouched
└── ... (rest of UI unchanged)

graph-after-physics-tweak.png                  # DELETE (working artifact)
```

**Structure Decision**: Single-frontend project. All work lives in `ui/src/components/graphCanvas/`. No new top-level directories. The new file `physics.ts` co-locates with the canvas component — same folder, same module owner — but stays import-only (no React, no DOM) so it can be unit-tested or hot-reloaded without remounting the canvas.

## Approach

### Force model

Two force *roles*, distinguished by the relation type of each edge:

| Role | Relations | Distance | Strength | Intent |
|------|-----------|---------:|---------:|--------|
| **Structural tether** | `belongs_to` | 80 | 0.15 | Long, soft pull keeping a child loosely attached to its project hub. Not load-bearing for sibling layout. |
| **Topical attractor (strong)** | `contains`, `subtask_of` | 30–35 | 0.6–0.7 | Tight binding for parent–child work structure (Task → WorkLog, parent → subtask). |
| **Topical attractor (medium)** | `solves`, `caused_by`, `uses`, `depends_on` | 40–45 | 0.45–0.55 | Pull problems and their solutions, cause/effect, and dependency relations close together. |
| **Topical attractor (loose)** | `blocks`, `related_to` | 50 | 0.4 | Weak topical link — visible proximity, but does not dominate. |
| **Default** | any unmapped | 50 | 0.4 | Safe fallback for future relation types. |

Three additional global forces:
- **Charge** (n-body repulsion): `strength = -140`, `distanceMax = 400`, `theta = 0.9` — bounded radius keeps Barnes-Hut cheap on large graphs; `theta = 0.9` widens the approximation cone (faster).
- **Center**: `strength = 0.02` — small but non-zero. Anchors disconnected components to the viewport without squeezing clusters into the middle.
- **Collide** (NEW): `radius = node._size + 2`, `strength = 0.75`, `iterations = 2` — prevents visual overlap. This is what breaks the equilibrium ring around hubs.

Simulation parameters: `alphaDecay = 0.035`, `velocityDecay = 0.55`, `cooldownTicks = 250`, `warmupTicks = 80`, `alphaMin = 0.003`. Bounded cooldown (down from `Infinity`) ensures the simulation actually stops; `velocityDecay` 0.55 (down from the experiment's 0.78) keeps motion responsive without jitter.

### Module API: `physics.ts`

Single dedicated module. Exports:
- `PHYSICS` — frozen config object (charge, center, collide, sim).
- `linkDistance(relation: string): number`
- `linkStrength(relation: string): number`
- `applyForces(fg)` — idempotent setup that wires every force into a `react-force-graph-2d` ref.

Renderer calls `applyForces(fgRef.current)` once on mount. No physics numerics live in `graphCanvas.tsx`.

### Lifecycle fix

Current code re-runs the physics setup `useEffect` on every `graphData` change and calls `d3ReheatSimulation` — that re-shakes the layout when the user types into search or toggles a filter (because `graphData` is a `useMemo` over `nodes`/`edges`/`degreeMap`).

New lifecycle:
- `useEffect(applyForces, [])` — setup once after the canvas ref is assigned (we use a ref-presence guard, not a deps array race).
- `useEffect(reheat, [nodes.length, edges.length])` — reheat **only** on structural set change (additions/removals), not on highlight/filter/selection prop churn.

### Why this satisfies each FR

| FR | Mechanism |
|----|-----------|
| FR-001 (`belongs_to` as tether) | Mapping table assigns `belongs_to` long distance + low strength. |
| FR-002 (topical attraction) | Mapping table assigns topical relations short distance + high strength. |
| FR-003 (no overlap) | New `forceCollide` keyed on `node._size`. |
| FR-004 (≤ 3 s stabilization on 1.2k/2.3k) | Bounded `distanceMax`, Barnes-Hut θ = 0.9, finite `cooldownTicks = 250`, ~80 warmup ticks. |
| FR-005 (no re-heat on filter/search) | Setup `useEffect` deps = `[]`; reheat deps = `[nodes.length, edges.length]`. |
| FR-006 (re-heat on structural change) | Same — reheat fires on size deltas. |
| FR-007 (drag returns to simulation) | Existing `onNodeDragEnd` clears `node.fx/fy`; preserved unchanged. |
| FR-008 (single config module) | All numerics live in `physics.ts`. Renderer imports nothing else for layout. |
| FR-009 (data-driven mapping) | `LINK_BY_RELATION` table + `LINK_DEFAULT` fallback. |
| FR-010 (no infinite drift) | `forceCenter` strength 0.02 (non-zero) + bounded `distanceMax`. |
| FR-011 (cleanup) | Delete `graph-after-physics-tweak.png` in the implementation tasks. |

### What is intentionally *not* changing

- Node painting, label rules, focus/hover highlight, edge label hover — all unchanged.
- Color theme, dynamic node sizing — unchanged.
- Sidebar, filtering UI, project zoom — unchanged.
- No new dependency beyond `d3-force-3d` (already a transitive dep, now declared explicitly).

## Complexity Tracking

> No constitution violations to justify.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| — | — | — |
