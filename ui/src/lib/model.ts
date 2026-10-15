import type { GraphPayload } from './types'
import { extractProject, parseNodeType, parseRelation } from './types'

export const UNASSIGNED = 'No project'

export interface GroupInfo {
  name: string
  /** Index of the project node, or -1 when the group has no hub. */
  hub: number
  size: number
  /** Centre and radius reserved for this group in the packed overview. */
  cx: number
  cy: number
  r: number
}

/**
 * Columnar view of the graph: what the renderer and layout need, without the
 * notes and data blobs. Built once in the worker; the raw payload stays there.
 */
export interface GraphModel {
  n: number
  ids: string[]
  labels: string[]
  type: Uint8Array
  typeNames: string[]
  group: Uint16Array
  groups: GroupInfo[]
  degree: Uint32Array
  edgeFrom: Uint32Array
  edgeTo: Uint32Array
  edgeRel: Uint8Array
  relNames: string[]
  /** Group of the most recently updated node: the view opens on it. */
  defaultGroup: number
}

function intern(names: string[], index: Map<string, number>, name: string): number {
  let i = index.get(name)
  if (i === undefined) {
    i = names.length
    names.push(name)
    index.set(name, i)
  }
  return i
}

export function buildModel(payload: GraphPayload): GraphModel {
  const { nodes, edges } = payload
  const n = nodes.length
  const ids = new Array<string>(n)
  const labels = new Array<string>(n)
  const type = new Uint8Array(n)
  const typeNames: string[] = []
  const typeIndex = new Map<string, number>()
  const byId = new Map<string, number>()

  for (let i = 0; i < n; i++) {
    const node = nodes[i]
    ids[i] = node.id
    labels[i] = node.label
    type[i] = intern(typeNames, typeIndex, parseNodeType(node.node_type))
    byId.set(node.id, i)
  }

  const projectType = typeIndex.get('project') ?? -1
  const relNames: string[] = []
  const relIndex = new Map<string, number>()
  const belongsTo = intern(relNames, relIndex, 'belongs_to')

  const from: number[] = []
  const to: number[] = []
  const rel: number[] = []
  const degree = new Uint32Array(n)
  // A node belongs to the first project it has a belongs_to edge to.
  const owner = new Int32Array(n).fill(-1)
  for (const e of edges) {
    const a = byId.get(e.from_id)
    const b = byId.get(e.to_id)
    if (a === undefined || b === undefined || a === b) continue
    const r = intern(relNames, relIndex, parseRelation(e.relation))
    from.push(a)
    to.push(b)
    rel.push(r)
    degree[a]++
    degree[b]++
    if (r === belongsTo && type[b] === projectType && owner[a] < 0) owner[a] = b
  }

  // Groups: one per project name (case-insensitive, labels and prefixes
  // disagree on case), then prefix-only projects, then everything else.
  const groups: GroupInfo[] = []
  const groupByName = new Map<string, number>()
  const groupOf = (name: string, hub: number): number => {
    const key = name.toLowerCase()
    let g = groupByName.get(key)
    if (g === undefined) {
      g = groups.length
      groups.push({ name, hub, size: 0, cx: 0, cy: 0, r: 0 })
      groupByName.set(key, g)
    } else if (groups[g].hub < 0 && hub >= 0) {
      groups[g].hub = hub
    }
    return g
  }

  const group = new Uint16Array(n)
  for (let i = 0; i < n; i++) if (type[i] === projectType) group[i] = groupOf(labels[i], i)
  let unassigned = -1
  for (let i = 0; i < n; i++) {
    if (type[i] === projectType) continue
    let g: number
    if (owner[i] >= 0) g = group[owner[i]]
    else {
      const prefix = extractProject(labels[i])
      if (prefix) g = groupOf(prefix, -1)
      else {
        if (unassigned < 0) unassigned = groupOf(UNASSIGNED, -1)
        g = unassigned
      }
    }
    group[i] = g
  }
  for (let i = 0; i < n; i++) groups[group[i]].size++

  let defaultGroup = 0
  let newest = ''
  for (let i = 0; i < n; i++) {
    const g = group[i]
    if (g === unassigned || groups[g].size < 2) continue
    const t = nodes[i].updated_at
    if (t > newest) {
      newest = t
      defaultGroup = g
    }
  }

  return {
    n,
    ids,
    labels,
    type,
    typeNames,
    group,
    groups,
    degree,
    edgeFrom: Uint32Array.from(from),
    edgeTo: Uint32Array.from(to),
    edgeRel: Uint8Array.from(rel),
    relNames,
    defaultGroup,
  }
}
