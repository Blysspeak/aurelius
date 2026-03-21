import type { GraphData } from './types'

const BASE = '/api'

export async function fetchGraph(): Promise<GraphData> {
  const resp = await fetch(`${BASE}/graph`)
  if (!resp.ok) throw new Error(`Failed to fetch graph: ${resp.status}`)
  return resp.json()
}

export async function searchNodes(query: string): Promise<GraphData> {
  const resp = await fetch(`${BASE}/search?q=${encodeURIComponent(query)}`)
  if (!resp.ok) throw new Error(`Search failed: ${resp.status}`)
  return resp.json()
}
