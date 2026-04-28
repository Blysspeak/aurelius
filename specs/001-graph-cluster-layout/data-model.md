# Phase 1 Data Model: Layout Configuration

This feature does not introduce persisted data — it changes only the in-memory layout configuration consumed by the canvas renderer. The "data model" here is the typed configuration surface exposed by the new `physics.ts` module.

---

## Entity: `LinkParams`

In-memory record describing how the simulation should treat a single relation type.

| Field | Type | Constraints | Description |
|-------|------|-------------|-------------|
| `distance` | `number` | `> 0`, units = canvas pixels | Target edge length the simulation pulls toward. |
| `strength` | `number` | `0 ≤ s ≤ 1` | How aggressively the link pulls endpoints toward `distance`. |

## Entity: `RelationForceTable`

The table that maps each known relation type to its `LinkParams`, with a single named default for unmapped relations.

| Field | Type | Description |
|-------|------|-------------|
| `LINK_BY_RELATION` | `Record<string, LinkParams>` | Keys are relation strings emitted by `parseRelation()` from `ui/src/types.ts` (e.g., `"belongs_to"`, `"solves"`). |
| `LINK_DEFAULT` | `LinkParams` | Fallback applied to any relation not present in `LINK_BY_RELATION`. |

**Validation rule (compile-time)**: every relation string in the project's `Relation` taxonomy that appears in production data should be either explicitly mapped or covered by the documented default. New relation types added later automatically fall through to `LINK_DEFAULT` — safe by construction, no runtime errors.

**Initial population** (mirrors plan.md):

| Relation | Distance | Strength | Role |
|----------|---------:|---------:|------|
| `belongs_to` | 80 | 0.15 | Structural tether |
| `contains` | 35 | 0.6 | Topical (strong) |
| `subtask_of` | 30 | 0.7 | Topical (strong) |
| `solves` | 40 | 0.55 | Topical (medium) |
| `caused_by` | 40 | 0.5 | Topical (medium) |
| `uses` | 45 | 0.45 | Topical (medium) |
| `depends_on` | 45 | 0.45 | Topical (medium) |
| `blocks` | 50 | 0.4 | Topical (loose) |
| `related_to` | 50 | 0.4 | Topical (loose) |
| *(default)* | 50 | 0.4 | Fallback |

## Entity: `PhysicsConfig`

Frozen object holding all global force parameters. One instance, one source of truth.

| Field | Shape | Initial Value | Description |
|-------|-------|---------------|-------------|
| `charge.strength` | `number` | `-140` | n-body repulsion (negative = repulsive). |
| `charge.distanceMax` | `number` | `400` | Cap repulsion radius — bounded n-body cost. |
| `charge.theta` | `number` | `0.9` | Barnes-Hut approximation cone (higher = faster, less accurate). |
| `center.strength` | `number` | `0.02` | Pull toward viewport center; small so it does not collapse clusters. |
| `collide.padding` | `number` | `2` | Extra px added to each node's rendered radius for collision. |
| `collide.strength` | `number` | `0.75` | How rigidly nodes resist overlap (`0..1`). |
| `collide.iterations` | `number` | `2` | Solver iterations per tick (more = cleaner separation). |
| `sim.alphaDecay` | `number` | `0.035` | Per-tick decay of simulation temperature. |
| `sim.velocityDecay` | `number` | `0.55` | Per-tick velocity damping. |
| `sim.cooldownTicks` | `number` | `250` | Hard cap on total ticks before stop. |
| `sim.warmupTicks` | `number` | `80` | Headless ticks before first paint. |
| `sim.alphaMin` | `number` | `0.003` | Stop threshold for simulation temperature. |

## Entity: `LayoutNode` (read-only consumer view)

The renderer attaches `_size` and `_relation` to nodes/links. The physics module reads them but never mutates the underlying `AureliusNode`/`AureliusEdge`.

| Field | Source | Used by |
|-------|--------|---------|
| `node._size` | `getNodeSizeDynamic()` in `theme.ts` | `forceCollide` radius accessor |
| `link._relation` | `parseRelation()` in `types.ts` | `linkDistance` / `linkStrength` lookup |

## State transitions

The simulation has two externally visible states relevant to this feature:

```text
        ┌───────────────────────────────────────────────────┐
        │                                                   │
        ▼                                                   │
   IDLE (post-cooldown)                                     │
        │                                                   │
        │  nodes.length or edges.length changes             │
        ▼                                                   │
   REHEATED ──── ticks until alpha < alphaMin ──────────────┘
                  or cooldownTicks reached
```

Filter/search/selection events do **not** cause a transition. Drag/release transitions through a transient unpinning ("node returned to simulation") but does not re-heat the global temperature.
