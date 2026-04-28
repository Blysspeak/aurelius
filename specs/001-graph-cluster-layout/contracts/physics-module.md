# Contract: `physics.ts` Module

The single public interface between the layout configuration and the canvas renderer.

**Path**: `ui/src/components/graphCanvas/physics.ts`
**Consumer**: `ui/src/components/graphCanvas/graphCanvas.tsx` (only)

---

## Exports

### `PHYSICS` — readonly config object

```ts
export const PHYSICS: {
  readonly charge:  { readonly strength: number; readonly distanceMax: number; readonly theta: number }
  readonly center:  { readonly strength: number }
  readonly collide: { readonly padding: number; readonly strength: number; readonly iterations: number }
  readonly sim:     {
    readonly alphaDecay:    number
    readonly velocityDecay: number
    readonly cooldownTicks: number
    readonly warmupTicks:   number
    readonly alphaMin:      number
  }
}
```

**Contract**:
- Frozen at module load (`as const` or `Object.freeze`).
- Numerical values match `data-model.md` initial values.
- `sim.*` fields are spread into `<ForceGraph2D>` props by the renderer; their names match the props exactly.

### `linkDistance(relation: string): number`

**Contract**:
- Returns `LINK_BY_RELATION[relation].distance` if mapped, else `LINK_DEFAULT.distance`.
- Total: never throws, never returns `NaN` or non-positive numbers.

### `linkStrength(relation: string): number`

**Contract**:
- Returns `LINK_BY_RELATION[relation].strength` if mapped, else `LINK_DEFAULT.strength`.
- Total: never throws, return value is in `[0, 1]`.

### `applyForces(fg): void`

**Signature**:
```ts
export function applyForces(fg: any): void
```

**Contract**:
- `fg` is a non-null `react-force-graph-2d` ref handle exposing `d3Force(name, [force])`.
- Idempotent: calling twice on the same `fg` produces the same simulation state.
- Side effects (in this exact order):
  1. Configures the existing `'link'` force with per-link `distance` and `strength` accessors that read `link._relation`.
  2. Configures the existing `'charge'` force with the values from `PHYSICS.charge`.
  3. Configures the existing `'center'` force with `PHYSICS.center.strength`.
  4. **Registers a new force** named `'collide'` (replacing any previously registered force of that name) with radius accessor `(n) => (n._size ?? 3) + PHYSICS.collide.padding`, strength `PHYSICS.collide.strength`, and `iterations` `PHYSICS.collide.iterations`.
- Does **not** call `d3ReheatSimulation` — heating is the renderer's responsibility, separated for lifecycle reasons.
- Does **not** mutate any node or edge data.

---

## Renderer obligations (graphCanvas.tsx)

The renderer's contract on its side of the boundary:

1. **Setup once**: call `applyForces(fgRef.current)` exactly once after the ref is populated. Use a `useEffect` with empty deps plus a ref-presence guard, OR a callback ref that fires once on mount.
2. **Reheat on structural change only**: call `fgRef.current?.d3ReheatSimulation?.()` from a `useEffect` whose deps are exactly `[nodes.length, edges.length]`. No other prop is allowed to trigger reheat.
3. **Pass simulation params as props**: `d3AlphaDecay={PHYSICS.sim.alphaDecay}`, `d3VelocityDecay={PHYSICS.sim.velocityDecay}`, `cooldownTicks={PHYSICS.sim.cooldownTicks}`, `warmupTicks={PHYSICS.sim.warmupTicks}`, `d3AlphaMin={PHYSICS.sim.alphaMin}`.
4. **Continue tagging links**: each link object passed to `<ForceGraph2D>` must carry a `_relation: string` field (already done via `parseRelation`).
5. **Continue tagging nodes**: each node must carry `_size: number` (already done via `getNodeSizeDynamic`).
6. **No layout numerics in renderer file**: zero raw numbers controlling distance/strength/decay/cooldown live in `graphCanvas.tsx`. The audit grep `grep -E '(distance|strength|distanceMax|alphaDecay|velocityDecay|cooldownTicks|warmupTicks|alphaMin)' graphCanvas.tsx` should return only references to `PHYSICS.*` (or zero matches if all are passed from `physics.ts` directly).

---

## Backwards compatibility

- The shape of `<ForceGraph2D>` props consumed elsewhere in the app: unchanged.
- The exposed window globals (`__aurelius_fg`, `__aurelius_positions`): unchanged.
- The drag/release behavior (`onNodeDragEnd` clears `node.fx/fy`): unchanged.
- The minimap consumer reading `__aurelius_positions`: unaffected (collision changes positions but not the API).

## Failure modes

| Failure | Cause | Detection | Mitigation |
|---------|-------|-----------|------------|
| Force not applied | `applyForces` called before ref is populated | Empty layout, nodes piled at origin | Ref-presence guard before call |
| Type error in chained `.distance().strength()` | d3-force-3d API drift | TypeScript compile error, or runtime `TypeError` | Ambient declaration captures the chain shape; bumped via `npm install d3-force-3d` |
| Re-heat thrash | Wrong deps array on reheat `useEffect` | Visible re-shake on filter/search | Static review: deps must be `[nodes.length, edges.length]`, nothing else |
| `forceCollide` not honored | Force registered before nodes attached | Overlap persists | `applyForces` on mount + reheat on structural change ensures collide sees populated nodes |
