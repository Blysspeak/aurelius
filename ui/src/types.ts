export interface AureliusNode {
  id: string
  node_type: string | Record<string, string>
  label: string
  note: string | null
  source: string
  data: Record<string, unknown>
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

export interface GraphData {
  nodes: AureliusNode[]
  edges: AureliusEdge[]
}

export interface GraphStats {
  totalNodes: number
  totalEdges: number
  typeCounts: Record<string, number>
}

export function parseNodeType(nt: string | Record<string, string>): string {
  if (typeof nt === 'string') return nt.replace(/"/g, '')
  if ('custom' in nt) return nt.custom
  return Object.keys(nt)[0] || 'unknown'
}

export function parseRelation(rel: string | Record<string, string>): string {
  if (typeof rel === 'string') return rel.replace(/"/g, '')
  return Object.keys(rel)[0] || 'related'
}
