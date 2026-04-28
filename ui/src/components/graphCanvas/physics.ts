import { forceCollide } from 'd3-force-3d'

/**
 * Force-directed layout tuned for hub-and-spoke topology.
 *
 * Aurelius graphs have a few project hubs with 50-500 children connected
 * via `belongs_to`. Naive uniform forces collapse children into a tight
 * ring around each hub. Two ideas fix this:
 *
 *   1. Map relation types to distinct distance/strength. `belongs_to` is
 *      a structural tether (long, soft) while topical relations like
 *      `contains`, `solves`, `subtask_of` are tight clusters (short, strong).
 *      Topical edges then dominate sibling layout, breaking the ring.
 *
 *   2. Add a collision force keyed on rendered node radius. Without it,
 *      siblings settle on the equilibrium ring of the charge force; with
 *      it, they jostle into irregular clumps that read as clusters.
 */

interface LinkParams {
  distance: number
  strength: number
}

const LINK_BY_RELATION: Record<string, LinkParams> = {
  // Hub tether: short enough that children visibly orbit their project,
  // soft enough that topical edges still control sibling layout within the orbit.
  belongs_to: { distance: 50, strength: 0.3 },

  // Topical: short & tight so semantically related nodes form sub-clusters.
  contains:    { distance: 35, strength: 0.6 },
  subtask_of:  { distance: 30, strength: 0.7 },
  solves:      { distance: 40, strength: 0.55 },
  caused_by:   { distance: 40, strength: 0.5 },
  blocks:      { distance: 50, strength: 0.4 },
  related_to:  { distance: 50, strength: 0.4 },
  uses:        { distance: 45, strength: 0.45 },
  depends_on:  { distance: 45, strength: 0.45 },
}

const LINK_DEFAULT: LinkParams = { distance: 50, strength: 0.4 }

export const linkDistance = (relation: string): number =>
  (LINK_BY_RELATION[relation] ?? LINK_DEFAULT).distance

export const linkStrength = (relation: string): number =>
  (LINK_BY_RELATION[relation] ?? LINK_DEFAULT).strength

export const PHYSICS = {
  // n-body repulsion. Kept deliberately weak — forceCollide already prevents
  // overlap, so charge only needs to provide gentle separation. Strong charge
  // over a 1k+ node graph overpowers `belongs_to` and detaches children from
  // their hubs (children of boostix end up floating far from the boostix node).
  charge: {
    strength: -70,
    distanceMax: 220,
    theta: 0.9,
  },
  // Small but non-zero — keeps disconnected components from drifting,
  // without squeezing clusters into a central blob.
  center: {
    strength: 0.02,
  },
  // Collision is what breaks the hairball: without it, hub-and-spoke
  // children settle on the equilibrium ring of the charge force.
  collide: {
    padding: 2,        // px added on top of each node's rendered radius
    strength: 0.75,    // 0..1, how rigidly nodes resist overlap
    iterations: 2,     // d3 default; more = cleaner separation, more cost
  },
  // Simulation tempo — bounded cooldown is mandatory: leaving cooldownTicks
  // at Infinity (the framework default) wastes CPU forever; too low and
  // the layout freezes before convergence on a 1k+ node graph.
  sim: {
    alphaDecay: 0.035,
    velocityDecay: 0.55,
    cooldownTicks: 250,
    warmupTicks: 80,
    alphaMin: 0.003,
  },
} as const

/* eslint-disable @typescript-eslint/no-explicit-any */

/**
 * Apply the full force configuration to a react-force-graph instance.
 * Idempotent — safe to call on remount or structural reheat.
 */
export function applyForces(fg: any): void {
  fg.d3Force('link')
    ?.distance((l: any) => linkDistance(l._relation ?? ''))
    .strength((l: any) => linkStrength(l._relation ?? ''))

  fg.d3Force('charge')
    ?.strength(PHYSICS.charge.strength)
    .distanceMax(PHYSICS.charge.distanceMax)
    .theta(PHYSICS.charge.theta)

  fg.d3Force('center')?.strength(PHYSICS.center.strength)

  fg.d3Force(
    'collide',
    forceCollide()
      .radius((n: any) => (n._size ?? 3) + PHYSICS.collide.padding)
      .strength(PHYSICS.collide.strength)
      .iterations(PHYSICS.collide.iterations),
  )
}
