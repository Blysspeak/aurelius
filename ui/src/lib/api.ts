import type { GraphPayload } from './types'

export const GRAPH_URL = '/api/graph'

/** Full-text search on the server (FTS5 in aurelius-core), up to 50 nodes. */
export async function searchNodes(query: string, signal?: AbortSignal): Promise<string[]> {
  const resp = await fetch(`/api/search?q=${encodeURIComponent(query)}`, { signal })
  if (!resp.ok) throw new Error(`Search failed with status ${resp.status}`)
  const data = (await resp.json()) as GraphPayload
  return data.nodes.map((n) => n.id)
}
