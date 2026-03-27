import { useState, useEffect, useCallback, useMemo, useRef } from 'react'
import { Header } from './components/header/header'
import { Sidebar } from './components/sidebar/sidebar'
import { GraphCanvas } from './components/graphCanvas/graphCanvas'
import { NodeDetail } from './components/nodeDetail/nodeDetail'
import { Minimap } from './components/minimap/minimap'
import { fetchGraph } from './api'
import { parseNodeType, extractProject } from './types'
import type { AureliusNode, AureliusEdge } from './types'

/* eslint-disable @typescript-eslint/no-explicit-any */

export function App() {
  const [nodes, setNodes] = useState<AureliusNode[]>([])
  const [edges, setEdges] = useState<AureliusEdge[]>([])
  const [selectedNode, setSelectedNode] = useState<AureliusNode | null>(null)
  const [searchQuery, setSearchQuery] = useState('')
  const [activeFilters, setActiveFilters] = useState<Set<string>>(new Set())
  const [activeProject, setActiveProject] = useState<string | null>(null)
  const [timeFilter, setTimeFilter] = useState<'all' | 'today' | '7d' | '30d'>('all')
  const [loading, setLoading] = useState(true)
  const graphRef = useRef<any>(null)

  useEffect(() => {
    fetchGraph()
      .then(data => {
        setNodes(data.nodes)
        setEdges(data.edges)
      })
      .catch(err => console.error('Failed to fetch graph:', err))
      .finally(() => setLoading(false))
  }, [])

  // Keyboard shortcuts
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setSelectedNode(null)
      if (e.key === '/' && (e.target as HTMLElement).tagName !== 'INPUT') {
        e.preventDefault()
        document.getElementById('search-input')?.focus()
      }
    }
    window.addEventListener('keydown', handleKey)
    return () => window.removeEventListener('keydown', handleKey)
  }, [])

  const typeCounts = nodes.reduce<Record<string, number>>((acc, n) => {
    const type = parseNodeType(n.node_type)
    acc[type] = (acc[type] || 0) + 1
    return acc
  }, {})

  const projectCounts = nodes.reduce<Record<string, number>>((acc, n) => {
    const proj = extractProject(n.label)
    if (proj) acc[proj] = (acc[proj] || 0) + 1
    return acc
  }, {})

  // Time cutoff
  const timeCutoff = useMemo(() => {
    if (timeFilter === 'all') return null
    const now = Date.now()
    const ms: Record<string, number> = { today: 86400000, '7d': 7 * 86400000, '30d': 30 * 86400000 }
    return new Date(now - ms[timeFilter]).toISOString()
  }, [timeFilter])

  // Search match IDs (highlight, not filter)
  const searchMatchIds = useMemo(() => {
    if (!searchQuery) return null
    const q = searchQuery.toLowerCase()
    return new Set(
      nodes.filter(n =>
        n.label.toLowerCase().includes(q) ||
        (n.note && n.note.toLowerCase().includes(q))
      ).map(n => n.id)
    )
  }, [nodes, searchQuery])

  const filteredNodes = nodes.filter(n => {
    const type = parseNodeType(n.node_type)
    const matchFilter = activeFilters.size === 0 || activeFilters.has(type)
    const isProjectNode = type === 'project' && n.label === activeProject
    const matchProject = !activeProject || extractProject(n.label) === activeProject || isProjectNode
    const matchTime = !timeCutoff || n.created_at >= timeCutoff
    return matchFilter && matchProject && matchTime
  })

  const handleToggleFilter = useCallback((type: string) => {
    setActiveFilters(prev => {
      const next = new Set(prev)
      if (next.has(type)) next.delete(type)
      else next.add(type)
      return next
    })
  }, [])

  const handleClearFilters = useCallback(() => {
    setActiveFilters(new Set())
  }, [])

  const handleSelectProject = useCallback((proj: string | null) => {
    setActiveProject(prev => prev === proj ? null : proj)
  }, [])

  const handleSelectNode = useCallback((node: AureliusNode | null) => {
    setSelectedNode(node)
  }, [])

  const handleSelectNodeById = useCallback((id: string) => {
    const node = nodes.find(n => n.id === id)
    if (node) setSelectedNode(node)
  }, [nodes])

  if (loading) {
    return (
      <div style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100vh',
        color: '#6b7b8e',
        fontSize: 14,
        gap: 12,
      }}>
        <img src="/logo.png" alt="" style={{ width: 32, height: 32, opacity: 0.6 }} />
        Loading graph...
      </div>
    )
  }

  return (
    <>
      <Header
        totalNodes={nodes.length}
        totalEdges={edges.length}
        searchQuery={searchQuery}
        onSearchChange={setSearchQuery}
      />

      <Sidebar
        typeCounts={typeCounts}
        activeFilters={activeFilters}
        onToggleFilter={handleToggleFilter}
        onClearFilters={handleClearFilters}
        projectCounts={projectCounts}
        activeProject={activeProject}
        onSelectProject={handleSelectProject}
        timeFilter={timeFilter}
        onTimeFilterChange={setTimeFilter}
      />

      <GraphCanvas
        graphRef={graphRef}
        nodes={filteredNodes}
        edges={edges}
        selectedNodeId={selectedNode?.id || null}
        searchMatchIds={searchMatchIds}
        zoomToProject={activeProject}
        onSelectNode={handleSelectNode}
      />

      <Minimap
        graphRef={graphRef}
        selectedNodeId={selectedNode?.id || null}
        hasDetailPanel={!!selectedNode}
      />

      {selectedNode && (
        <NodeDetail
          node={selectedNode}
          edges={edges}
          allNodes={nodes}
          onClose={() => setSelectedNode(null)}
          onSelectNode={handleSelectNodeById}
        />
      )}
    </>
  )
}
