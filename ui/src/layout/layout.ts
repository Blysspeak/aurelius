import type { GraphModel, GroupInfo } from '../lib/model'
import { typeRadius } from '../lib/palette'

// Layout runs once per project in a Web Worker and is then frozen: the main
// thread never steps physics. Each project is laid out on its own around its
// hub, then placed in a slot reserved by packGroups(), so positions never
// depend on which projects are visible and filters never reshuffle anything.
//
// The solver is a small force layout on typed arrays: springs along topical
// edges, short-range repulsion through a uniform grid (cutoff REPEL.range),
// collision on the same pairs, and a weak pull to the project centre.
// d3-force was measured first: its Barnes-Hut charge plus collide cost
// ~175 ms per tick on the 16k-node xhub project (37 s for 200 ticks); this
// grid version is O(n) per tick because repulsion is local.

/** Rest length per relation. `belongs_to` is absent on purpose: 16k springs
 * to one hub make a ring, so the hub is pinned at the centre and CENTRE does
 * that job. Topical relations are short so sessions/tasks gather their items. */
const LINK_DISTANCE: Record<string, number> = {
  contains: 8,
  subtask_of: 10,
  solves: 8,
  caused_by: 10,
  refines: 9,
  supersedes: 9,
  related_to: 22,
}
const LINK_DISTANCE_DEFAULT = 16
const LINK_STRENGTH: Record<string, number> = { related_to: 0.08 }
const LINK_STRENGTH_DEFAULT = 0.7

/** Repulsion between nodes closer than `range`, falling to zero at `range`.
 * Items of the same session/task push each other gently (`same`), strangers
 * push hard (`other`): that contrast is what opens gaps between clusters,
 * so a project reads as clumps instead of an even disc. */
const REPEL = { strength: 10, range: 24, same: 0.15, other: 2.6 }
/** Extra gap kept between drawn circles. */
const COLLIDE_PAD = 1.2
/** Pull to the centre, divided by sqrt(group size) so big projects are not
 * crushed denser than small ones. */
const CENTRE = 0.4
const VELOCITY_DECAY = 0.42
/** Per-tick displacement cap, layout units: one bad pair cannot fling a node. */
const MAX_SPEED = 12
export const TICKS = 260

/** Expected radius of a laid-out group of `size` nodes, in layout units. */
export function groupRadius(size: number): number {
  return 4.0 * Math.sqrt(size) + 16
}

export interface GroupLayout {
  /** Global node indices of this group, in the order of `x`/`y`. */
  members: Uint32Array
  x: Float32Array
  y: Float32Array
  vx: Float32Array
  vy: Float32Array
  r: Float32Array
  pinned: number
  /** Root container (session, task) each node was seeded under. */
  cluster: Int32Array
  linkA: Uint32Array
  linkB: Uint32Array
  linkRest: Float32Array
  linkK: Float32Array
  linkBias: Float32Array
  tick: number
  // grid scratch
  cellOf: Int32Array
  order: Int32Array
}

/** Place groups on non-overlapping circles, biggest first, spiralling out. */
export function packGroups(groups: GroupInfo[]): void {
  const order = groups.map((_, i) => i).sort((a, b) => groups[b].size - groups[a].size)
  const placed: GroupInfo[] = []
  const gap = 30
  for (const gi of order) {
    const g = groups[gi]
    g.r = groupRadius(g.size)
    if (placed.length === 0) {
      g.cx = 0
      g.cy = 0
      placed.push(g)
      continue
    }
    for (let t = 0; ; t += 0.05) {
      const rad = 10 * t
      const x = Math.cos(t) * rad
      const y = Math.sin(t) * rad
      let free = true
      for (const p of placed) {
        const dx = p.cx - x
        const dy = p.cy - y
        const min = p.r + g.r + gap
        if (dx * dx + dy * dy < min * min) {
          free = false
          break
        }
      }
      if (free) {
        g.cx = x
        g.cy = y
        break
      }
    }
    placed.push(g)
  }
}

export function startGroupLayout(model: GraphModel, g: number): GroupLayout {
  const memberList: number[] = []
  for (let i = 0; i < model.n; i++) if (model.group[i] === g) memberList.push(i)
  const m = memberList.length
  const local = new Map<number, number>()
  memberList.forEach((i, k) => local.set(i, k))
  const hub = model.groups[g].hub

  const r = new Float32Array(m)
  for (let k = 0; k < m; k++) r[k] = typeRadius(model.typeNames[model.type[memberList[k]]])

  const belongsTo = model.relNames.indexOf('belongs_to')
  const containsRel = model.relNames.indexOf('contains')
  const la: number[] = []
  const lb: number[] = []
  const rel: number[] = []
  const deg = new Uint32Array(m)
  const children = new Map<number, number[]>()
  const hasParent = new Uint8Array(m)
  for (let e = 0; e < model.edgeFrom.length; e++) {
    if (model.edgeRel[e] === belongsTo) continue
    const a = local.get(model.edgeFrom[e])
    const b = local.get(model.edgeTo[e])
    if (a === undefined || b === undefined) continue
    la.push(a)
    lb.push(b)
    rel.push(model.edgeRel[e])
    deg[a]++
    deg[b]++
    if (model.edgeRel[e] === containsRel && !hasParent[b]) {
      hasParent[b] = 1
      let list = children.get(a)
      if (!list) children.set(a, (list = []))
      list.push(b)
    }
  }
  const nl = la.length
  const linkRest = new Float32Array(nl)
  const linkK = new Float32Array(nl)
  const linkBias = new Float32Array(nl)
  for (let e = 0; e < nl; e++) {
    const name = model.relNames[rel[e]]
    linkRest[e] = LINK_DISTANCE[name] ?? LINK_DISTANCE_DEFAULT
    // d3-style: a spring on a high-degree node is weaker, and the light end moves more.
    linkK[e] = (LINK_STRENGTH[name] ?? LINK_STRENGTH_DEFAULT) / Math.sqrt(Math.min(deg[la[e]], deg[lb[e]]))
    linkBias[e] = deg[la[e]] / (deg[la[e]] + deg[lb[e]])
  }

  // Seed: containers (sessions, tasks) and loose items on a sunflower spiral,
  // contained items in a small ring around their container. Starting near the
  // answer is what lets a fixed tick budget settle a 16k-node project.
  const x = new Float32Array(m)
  const y = new Float32Array(m)
  const golden = Math.PI * (3 - Math.sqrt(5))
  let acc = 0
  let j = 0
  const hubLocal = hub >= 0 ? local.get(hub)! : -1
  const placed = new Uint8Array(m)
  const cluster = new Int32Array(m).fill(-1)
  if (hubLocal >= 0) placed[hubLocal] = 1
  const queue: number[] = []
  const placeTree = (root: number) => {
    const kids = children.get(root) ?? []
    acc += 1 + kids.length
    const rad = 4.2 * Math.sqrt(acc) + 14
    const a = j++ * golden
    x[root] = Math.cos(a) * rad
    y[root] = Math.sin(a) * rad
    placed[root] = 1
    cluster[root] = root
    queue.push(root)
    while (queue.length) {
      const p = queue.pop()!
      const list = (children.get(p) ?? []).filter((c) => !placed[c])
      const ring = 4 + Math.sqrt(list.length) * 2.5
      for (let c = 0; c < list.length; c++) {
        const b = (c / list.length) * Math.PI * 2 + a + c * 0.01
        x[list[c]] = x[p] + Math.cos(b) * ring
        y[list[c]] = y[p] + Math.sin(b) * ring
        placed[list[c]] = 1
        cluster[list[c]] = root
        queue.push(list[c])
      }
    }
  }
  for (let k = 0; k < m; k++) if (!hasParent[k] && !placed[k]) placeTree(k)
  // Containment cycles or items contained by the hub itself: seed them as roots.
  for (let k = 0; k < m; k++) if (!placed[k]) placeTree(k)

  return {
    members: Uint32Array.from(memberList),
    x,
    y,
    vx: new Float32Array(m),
    vy: new Float32Array(m),
    r,
    pinned: hubLocal,
    cluster,
    linkA: Uint32Array.from(la),
    linkB: Uint32Array.from(lb),
    linkRest,
    linkK,
    linkBias,
    tick: 0,
    cellOf: new Int32Array(m),
    order: new Int32Array(m),
  }
}

function tickOnce(l: GroupLayout): void {
  const { x, y, vx, vy, r } = l
  const m = x.length
  const alpha = Math.pow(0.002, l.tick / TICKS)

  // Springs.
  for (let e = 0; e < l.linkA.length; e++) {
    const a = l.linkA[e]
    const b = l.linkB[e]
    let dx = x[b] + vx[b] - x[a] - vx[a]
    let dy = y[b] + vy[b] - y[a] - vy[a]
    const d = Math.hypot(dx, dy) || 1e-3
    const f = ((d - l.linkRest[e]) / d) * alpha * l.linkK[e]
    dx *= f
    dy *= f
    const bias = l.linkBias[e]
    vx[b] -= dx * bias
    vy[b] -= dy * bias
    vx[a] += dx * (1 - bias)
    vy[a] += dy * (1 - bias)
  }

  // Grid for short-range repulsion and collision.
  const range = REPEL.range
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  for (let i = 0; i < m; i++) {
    if (x[i] < minX) minX = x[i]
    if (y[i] < minY) minY = y[i]
    if (x[i] > maxX) maxX = x[i]
    if (y[i] > maxY) maxY = y[i]
  }
  let cell = range
  let cols = Math.floor((maxX - minX) / cell) + 1
  let rows = Math.floor((maxY - minY) / cell) + 1
  while (cols * rows > 4 * m + 64) {
    cell *= 1.5
    cols = Math.floor((maxX - minX) / cell) + 1
    rows = Math.floor((maxY - minY) / cell) + 1
  }
  const start = new Int32Array(cols * rows + 1)
  for (let i = 0; i < m; i++) {
    const c = Math.floor((y[i] - minY) / cell) * cols + Math.floor((x[i] - minX) / cell)
    l.cellOf[i] = c
    start[c + 1]++
  }
  for (let c = 0; c < cols * rows; c++) start[c + 1] += start[c]
  const fill = start.slice(0, cols * rows)
  for (let i = 0; i < m; i++) l.order[fill[l.cellOf[i]]++] = i

  const range2 = range * range
  const kSame = REPEL.strength * REPEL.same * alpha
  const kOther = REPEL.strength * REPEL.other * alpha
  for (let i = 0; i < m; i++) {
    const ci = l.cellOf[i]
    const cx = ci % cols
    const cy = (ci - cx) / cols
    for (let oy = -1; oy <= 1; oy++) {
      const ry = cy + oy
      if (ry < 0 || ry >= rows) continue
      for (let ox = -1; ox <= 1; ox++) {
        const rx = cx + ox
        if (rx < 0 || rx >= cols) continue
        const c = ry * cols + rx
        for (let s = start[c]; s < start[c + 1]; s++) {
          const jn = l.order[s]
          if (jn <= i) continue
          let dx = x[jn] - x[i]
          let dy = y[jn] - y[i]
          let d2 = dx * dx + dy * dy
          if (d2 >= range2) continue
          if (d2 < 1e-6) {
            dx = (((i * 7919 + jn) % 13) - 6) * 0.01 + 0.01
            dy = (((jn * 104729 + i) % 11) - 5) * 0.01
            d2 = dx * dx + dy * dy
          }
          const d = Math.sqrt(d2)
          // Repulsion fades linearly to zero at the cutoff, so the cutoff
          // leaves no visible seam; the 1.5 floor keeps near-coincident
          // pairs from producing huge kicks.
          const kk = l.cluster[i] === l.cluster[jn] ? kSame : kOther
          let f = (kk * (1 - d / range)) / Math.max(d, 1.5) / d
          const min = r[i] + r[jn] + COLLIDE_PAD
          if (d < min) f += ((min - d) / d) * 0.35
          const fx = dx * f
          const fy = dy * f
          vx[i] -= fx
          vy[i] -= fy
          vx[jn] += fx
          vy[jn] += fy
        }
      }
    }
  }

  const c = (CENTRE / Math.sqrt(m + 1)) * alpha
  for (let i = 0; i < m; i++) {
    vx[i] -= x[i] * c
    vy[i] -= y[i] * c
    vx[i] *= 1 - VELOCITY_DECAY
    vy[i] *= 1 - VELOCITY_DECAY
    const sp = Math.hypot(vx[i], vy[i])
    if (sp > MAX_SPEED) {
      vx[i] *= MAX_SPEED / sp
      vy[i] *= MAX_SPEED / sp
    }
    x[i] += vx[i]
    y[i] += vy[i]
  }
  if (l.pinned >= 0) {
    x[l.pinned] = 0
    y[l.pinned] = 0
    vx[l.pinned] = 0
    vy[l.pinned] = 0
  }
  l.tick++
}

/** Advance at most `n` ticks; true when the layout is finished. */
export function stepGroupLayout(l: GroupLayout, n: number): boolean {
  for (let s = 0; s < n && l.tick < TICKS; s++) tickOnce(l)
  return l.tick >= TICKS
}

/** Global positions of a group, shrunk to fit its reserved slot if needed. */
export function placeGroup(l: GroupLayout, g: GroupInfo): Float32Array {
  const m = l.x.length
  let cx = 0
  let cy = 0
  if (l.pinned < 0 && m > 0) {
    for (let i = 0; i < m; i++) {
      cx += l.x[i]
      cy += l.y[i]
    }
    cx /= m
    cy /= m
  }
  let extent = 1
  for (let i = 0; i < m; i++) extent = Math.max(extent, Math.hypot(l.x[i] - cx, l.y[i] - cy) + l.r[i])
  const scale = Math.min(1, g.r / extent)
  const xy = new Float32Array(m * 2)
  for (let i = 0; i < m; i++) {
    xy[i * 2] = g.cx + (l.x[i] - cx) * scale
    xy[i * 2 + 1] = g.cy + (l.y[i] - cy) * scale
  }
  return xy
}
