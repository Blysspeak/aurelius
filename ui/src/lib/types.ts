// Shapes returned by `au view` (crates/au/src/view.rs): /api/graph and
// /api/search both answer { nodes, edges } with serde-serialised core types.

export interface AureliusNode {
  id: string
  node_type: string | Record<string, string>
  label: string
  note: string | null
  source: string
  data: Record<string, unknown> | null
  created_at: string
  updated_at: string
  memory_kind: string
  last_accessed_at: string
  access_count: number
  content_hash: string | null
}

export interface AureliusEdge {
  id: string
  from_id: string
  to_id: string
  relation: string | Record<string, string>
  weight: number
  created_at: string
}

export interface GraphPayload {
  nodes: AureliusNode[]
  edges: AureliusEdge[]
}

/** `"decision"` or `{ "custom": "document" }` -> type name. */
export function parseNodeType(nt: AureliusNode['node_type']): string {
  if (typeof nt === 'string') return nt.replace(/"/g, '')
  if ('custom' in nt) return nt.custom
  return Object.keys(nt)[0] || 'unknown'
}

export function parseRelation(rel: AureliusEdge['relation']): string {
  if (typeof rel === 'string') return rel.replace(/"/g, '')
  if ('custom' in rel) return rel.custom
  return Object.keys(rel)[0] || 'related'
}

/** Project name from a label like "[my-project] Some text". */
export function extractProject(label: string): string | null {
  const m = label.match(/^\[([^\]]+)\]/)
  return m ? m[1] : null
}

/** Label without its "[project] " prefix, for display inside a project view. */
export function stripProject(label: string): string {
  return label.replace(/^\[[^\]]+\]\s*/, '')
}
