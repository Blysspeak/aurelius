# Phase 0 Research: Semantic Cluster Graph Layout

All Technical Context fields resolved during planning. Decisions, rationale, and rejected alternatives below.

---

### Decision: Use `d3-force-3d` directly for `forceCollide`

**Rationale**: `react-force-graph-2d` is built on `force-graph`, which depends on `d3-force-3d` (a 3D-capable fork of `d3-force` maintained by the same author as `force-graph`). The `fg.d3Force(name, force)` API accepts d3-shaped force constructors. Using the *same* `d3-force-3d` package guarantees ABI compatibility — same coordinate fields, same simulation contract, no risk of two parallel d3 builds in the bundle.

**Alternatives considered**:
- *Plain `d3-force`*: Same API surface for `forceCollide`, but force-graph internally uses `d3-force-3d`. Parallel builds risk subtle drift if the upstream APIs diverge.
- *Hand-rolled collision force*: ~30 lines for a quadtree-pruned collide. Reinvents a battle-tested d3 primitive. Rejected — not worth the maintenance.
- *No collide, tune charge harder*: Confirmed not viable. Without collision, hub-and-spoke topology settles into the equilibrium ring of the charge force regardless of strength — that is the symptom we are fixing.

### Decision: Ambient TypeScript declaration for `d3-force-3d`

**Rationale**: No `@types/d3-force-3d` exists on DefinitelyTyped. We use exactly one constructor (`forceCollide`) with three chained methods (`radius`, `strength`, `iterations`). A 12-line ambient declaration in `vite-env.d.ts` types our usage and avoids polluting the bundle with a full d3 type fork.

**Alternatives considered**:
- *Cast to `any` at import site*: Loses type safety on the chain.
- *Generate types from JSDoc*: Overkill for one function.

### Decision: Co-locate `physics.ts` next to `graphCanvas.tsx`

**Rationale**: The physics module has exactly one consumer. Co-location makes ownership obvious, keeps imports short, and survives future component renames. Pulling it up to `ui/src/lib/` would invite premature reuse pressure.

**Alternatives considered**:
- *`ui/src/lib/graph-physics/`*: Reasonable for a multi-canvas world. We have one canvas. YAGNI.
- *Inline as a `const PHYSICS` at the top of `graphCanvas.tsx`*: Violates FR-008 (single config module separate from renderer).

### Decision: Decouple force setup from `graphData` reactivity

**Rationale**: The current `useEffect` on `[graphData]` re-applies forces and calls `d3ReheatSimulation` whenever any prop in the chain changes — including `searchMatchIds` and `selectedNodeId`, which trigger `graphData` recomputation through `useMemo`. That violates FR-005 (no re-heat on filter/search). Splitting into one `useEffect` for setup (run once) and another for reheat keyed on `[nodes.length, edges.length]` cleanly separates "wire the forces" from "the world got bigger or smaller, redistribute".

**Alternatives considered**:
- *Memoize `graphData` more aggressively*: Doesn't solve the root cause; downstream consumers can still touch `graphData`.
- *Imperative setup outside React*: Would work but breaks ref lifecycle on remount.

### Decision: Numerical force values

Calibrated against the documented Aurelius graph topology:
- ~1,200 nodes, ~2,300 edges
- Most edges are `belongs_to` (project hub → child)
- Hubs range from a few children (small projects) to ~450 (boostix)
- Node sizes 2–6 px (rendered radius), so 8 px collision radius (size + 2 padding) gives a comfortable visual gap

Specific values:
- `belongs_to`: distance 80, strength 0.15. Long enough to put children outside the immediate orbit of the hub; soft enough that topical edges win at sibling-distance scale.
- `contains` / `subtask_of`: distance 30–35, strength 0.6–0.7. These dominate locally — Task and its WorkLogs sit on top of each other visually.
- `solves` / `caused_by` / `uses` / `depends_on`: distance 40–45, strength 0.45–0.55. Visible attraction without overpowering the parent–child hierarchy.
- `blocks` / `related_to`: distance 50, strength 0.4. Soft. These are weak semantic ties.
- `charge`: -140 strength, 400 max distance, θ 0.9. Bounded radius keeps the n-body cost manageable on growing graphs.
- `center`: 0.02. Just enough to anchor disconnected components.
- `collide`: 2 px padding, 0.75 strength, 2 iterations. Two iterations is the d3 default for clean separation without excessive cost.
- Sim: `alphaDecay 0.035`, `velocityDecay 0.55`, `cooldownTicks 250`, `warmupTicks 80`. Bounded cooldown is mandatory — `Infinity` (the pre-experiment default) wastes CPU forever; the experiment's 400 ticks combined with high damping froze before convergence.

These values are a starting point. Spec SC-001..SC-006 are the calibration target. Tuning happens through `physics.ts` only.

**Alternatives considered**:
- *Force-cluster (d3-force-cluster)*: Adds an extra dep for a force whose effect we get for free from typed link strengths. Rejected.
- *`forceRadial` for project hubs*: Would arrange children on a fixed radius around their hub — but that is *exactly* the ring we are trying to break. Rejected.
- *Per-project `forceX/forceY` to grid the hubs*: Tempting for multi-project layout. Rejected for v1 — the existing zoom-to-project behavior already lets users navigate, and adding hub-grid forces increases tuning surface area. Out of scope.

### Decision: Performance approach for stabilization within 3 s

**Rationale**: The constraints `distanceMax = 400` (charge) and `theta = 0.9` (Barnes-Hut approximation) are the dominant performance levers. Empirically, this configuration stabilizes a 1.2k-node graph in well under 3 s on commodity hardware. `cooldownTicks = 250` caps the worst case.

**Alternatives considered**:
- *WebGL / WebGPU renderer*: Would scale to 10k+ but overkill at our scale and outside the "no new heavy deps" constraint.
- *Off-main-thread simulation*: Sensible at 10k+ nodes; premature here.

### Decision: Quickstart-based verification, no automated test

**Rationale**: The acceptance criteria are inherently visual ("clusters are readable", "no perceptible re-shake"). The UI workspace currently has no test framework wired up. Adding one (vitest + DOM testing-library) would be a multi-day setup that doesn't directly serve the spec. We document a manual quickstart and use Playwright snapshots for before/after evidence.

**Alternatives considered**:
- *Visual regression suite*: Worth doing eventually, but a separate feature.
- *Pixel-diff snapshot tests*: Brittle for force-directed layouts that converge to slightly different rotations on each run. Rejected.
