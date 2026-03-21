import { useState, useEffect, useCallback } from 'react'
import { Header } from './components/header/header'
import { Sidebar } from './components/sidebar/sidebar'
import { GraphCanvas } from './components/graphCanvas/graphCanvas'
import { NodeDetail } from './components/nodeDetail/nodeDetail'
import { fetchGraph } from './api'
import { parseNodeType } from './types'
import type { AureliusNode, AureliusEdge } from './types'

export function App() {
  const [nodes, setNodes] = useState<AureliusNode[]>([])
  const [edges, setEdges] = useState<AureliusEdge[]>([])
  const [selectedNode, setSelectedNode] = useState<AureliusNode | null>(null)
  const [searchQuery, setSearchQuery] = useState('')
  const [activeFilters, setActiveFilters] = useState<Set<string>>(new Set())
  const [loading, setLoading] = useState(true)

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

  const filteredNodes = nodes.filter(n => {
    const type = parseNodeType(n.node_type)
    const matchFilter = activeFilters.size === 0 || activeFilters.has(type)
    const matchSearch = !searchQuery ||
      n.label.toLowerCase().includes(searchQuery.toLowerCase()) ||
      (n.note && n.note.toLowerCase().includes(searchQuery.toLowerCase()))
    return matchFilter && matchSearch
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
      />

      <GraphCanvas
        nodes={filteredNodes}
        edges={edges}
        selectedNodeId={selectedNode?.id || null}
        onSelectNode={handleSelectNode}
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
