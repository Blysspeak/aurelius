import { useCallback, useEffect, useMemo, useState } from 'react'
import ForceGraph2D from 'react-force-graph-2d'
import type { AureliusNode, AureliusEdge } from '../../types'
import { parseNodeType, parseRelation } from '../../types'
import { getNodeColor, getNodeSizeDynamic } from '../../theme'
import { applyForces, PHYSICS } from './physics'
import styles from './graphCanvas.module.css'

/* eslint-disable @typescript-eslint/no-explicit-any */

interface GraphCanvasProps {
  nodes: AureliusNode[]
  edges: AureliusEdge[]
  selectedNodeId: string | null
  searchMatchIds: Set<string> | null
  zoomToProject: string | null
  graphRef: React.MutableRefObject<any>
  onSelectNode: (node: AureliusNode | null) => void
}

// ─── Component ──────────────────────────────────────────────────────
export function GraphCanvas({ nodes, edges, selectedNodeId, searchMatchIds, zoomToProject, graphRef, onSelectNode }: GraphCanvasProps) {
  const fgRef = graphRef
  const [hoveredId, setHoveredId] = useState<string | null>(null)

  const nodeMap = useMemo(() => new Map(nodes.map(n => [n.id, n])), [nodes])

  // Degree map for dynamic sizing
  const degreeMap = useMemo(() => {
    const m = new Map<string, number>()
    edges.forEach(e => {
      m.set(e.from_id, (m.get(e.from_id) || 0) + 1)
      m.set(e.to_id, (m.get(e.to_id) || 0) + 1)
    })
    return m
  }, [edges])

  const graphData = useMemo(() => {
    const nodeIds = new Set(nodes.map(n => n.id))

    return {
      nodes: nodes.map(n => {
        const type = parseNodeType(n.node_type)
        return {
          id: n.id,
          label: n.label,
          _type: type,
          _color: getNodeColor(type),
          _size: getNodeSizeDynamic(type, degreeMap.get(n.id) || 0),
          _note: n.note,
        }
      }),
      links: edges
        .filter(e => nodeIds.has(e.from_id) && nodeIds.has(e.to_id))
        .map(e => ({
          source: e.from_id,
          target: e.to_id,
          _weight: e.weight,
          _relation: parseRelation(e.relation),
        })),
    }
  }, [nodes, edges, degreeMap])

  // Active highlight: selected OR hovered + neighbors
  const activeId = selectedNodeId || hoveredId
  const highlightIds = useMemo(() => {
    if (!activeId) return null
    const ids = new Set<string>([activeId])
    for (const e of edges) {
      if (e.from_id === activeId) ids.add(e.to_id)
      if (e.to_id === activeId) ids.add(e.from_id)
    }
    return ids
  }, [activeId, edges])

  const handleNodeClick = useCallback((node: any) => {
    onSelectNode(nodeMap.get(node.id) || null)
  }, [onSelectNode, nodeMap])

  const handleNodeHover = useCallback((node: any) => {
    setHoveredId(node?.id || null)
  }, [])

  // ─── Custom node renderer ────────────────────────────────────────
  const paintNode = useCallback((node: any, ctx: CanvasRenderingContext2D, globalScale: number) => {
    const { x, y, _color: color, _size: baseSize, _type: type, label, id } = node
    if (x == null || y == null) return

    // Share node positions with minimap via window
    const positions = ((window as any).__aurelius_positions || ((window as any).__aurelius_positions = [])) as any[]
    positions.push({ id, x, y, _color: color, _type: type })

    const isSelected = id === selectedNodeId
    const isHovered = id === hoveredId
    const isFocused = isSelected || isHovered
    const isHighlighted = highlightIds ? highlightIds.has(id) : true
    const dimmed = highlightIds && !isHighlighted

    // Search highlight layer
    const isSearchMatch = searchMatchIds ? searchMatchIds.has(id) : false
    const searchDimmed = searchMatchIds && !isSearchMatch

    const size = isFocused ? baseSize * 1.6 : baseSize

    // Search match glow
    if (searchMatchIds && isSearchMatch) {
      const gradient = ctx.createRadialGradient(x, y, size * 0.5, x, y, size * 4)
      gradient.addColorStop(0, `${color}60`)
      gradient.addColorStop(1, `${color}00`)
      ctx.beginPath()
      ctx.arc(x, y, size * 4, 0, 2 * Math.PI)
      ctx.fillStyle = gradient
      ctx.fill()
    }

    // Outer glow for focused node
    if (isFocused && !searchDimmed) {
      const gradient = ctx.createRadialGradient(x, y, size * 0.5, x, y, size * 3)
      gradient.addColorStop(0, `${color}40`)
      gradient.addColorStop(1, `${color}00`)
      ctx.beginPath()
      ctx.arc(x, y, size * 3, 0, 2 * Math.PI)
      ctx.fillStyle = gradient
      ctx.fill()
    }

    // Neighbor glow
    if (highlightIds && isHighlighted && !isFocused && !searchDimmed) {
      ctx.beginPath()
      ctx.arc(x, y, size + 2, 0, 2 * Math.PI)
      ctx.fillStyle = `${color}20`
      ctx.fill()
    }

    // Main circle
    ctx.beginPath()
    ctx.arc(x, y, size, 0, 2 * Math.PI)
    const alpha = searchDimmed ? '12' : dimmed ? '25' : 'ff'
    ctx.fillStyle = alpha === 'ff' ? color : `${color}${alpha}`
    ctx.fill()

    // Ring
    if (isSelected) {
      ctx.strokeStyle = '#c5a44e'
      ctx.lineWidth = 2 / globalScale
      ctx.stroke()
    } else if (isHovered) {
      ctx.strokeStyle = `${color}90`
      ctx.lineWidth = 1.5 / globalScale
      ctx.stroke()
    }

    // Labels
    const showLabel = searchDimmed
      ? false
      : type === 'project' || isFocused || (highlightIds && isHighlighted) || isSearchMatch
    if (showLabel) {
      const truncated = label.length > 28 ? label.slice(0, 26) + '…' : label
      const fontSize = isFocused
        ? Math.max(14 / globalScale, 4)
        : type === 'project'
          ? Math.max(12 / globalScale, 3.5)
          : Math.max(10 / globalScale, 2.5)

      ctx.font = `${isFocused ? 600 : type === 'project' ? 500 : 400} ${fontSize}px Inter, -apple-system, sans-serif`
      ctx.textAlign = 'center'
      ctx.textBaseline = 'top'

      // Shadow
      ctx.fillStyle = '#0c1018'
      ctx.fillText(truncated, x + 0.5, y + size + 3.5)
      ctx.fillText(truncated, x - 0.5, y + size + 2.5)

      // Text
      ctx.fillStyle = isFocused ? '#e6edf3' : dimmed ? '#4a556830' : '#8b949ecc'
      ctx.fillText(truncated, x, y + size + 3)
    }
  }, [selectedNodeId, hoveredId, highlightIds, searchMatchIds])

  const paintNodeArea = useCallback((node: any, color: string, ctx: CanvasRenderingContext2D) => {
    const { x, y, _size: size } = node
    if (x == null || y == null) return
    ctx.beginPath()
    ctx.arc(x, y, size + 4, 0, 2 * Math.PI)
    ctx.fillStyle = color
    ctx.fill()
  }, [])

  // ─── Edge label on hover ─────────────────────────────────────────
  const paintLink = useCallback((link: any, ctx: CanvasRenderingContext2D, globalScale: number) => {
    if (!hoveredId) return
    const srcId = typeof link.source === 'string' ? link.source : link.source?.id
    const tgtId = typeof link.target === 'string' ? link.target : link.target?.id
    if (srcId !== hoveredId && tgtId !== hoveredId) return

    const src = link.source
    const tgt = link.target
    if (!src?.x || !tgt?.x) return

    const midX = (src.x + tgt.x) / 2
    const midY = (src.y + tgt.y) / 2
    const fontSize = Math.max(9 / globalScale, 2)

    ctx.font = `400 ${fontSize}px Inter, sans-serif`
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'

    // Background
    const text = link._relation
    const tw = ctx.measureText(text).width
    ctx.fillStyle = '#0c1018cc'
    ctx.fillRect(midX - tw / 2 - 2 / globalScale, midY - fontSize / 2 - 1 / globalScale, tw + 4 / globalScale, fontSize + 2 / globalScale)

    // Text
    ctx.fillStyle = '#c5a44ecc'
    ctx.fillText(text, midX, midY)
  }, [hoveredId])

  const getLinkColor = useCallback((link: any) => {
    if (!highlightIds) return '#2a3a4c30'
    const srcId = typeof link.source === 'string' ? link.source : link.source?.id
    const tgtId = typeof link.target === 'string' ? link.target : link.target?.id
    if (highlightIds.has(srcId) && highlightIds.has(tgtId)) return '#c5a44e70'
    return '#1e2a3812'
  }, [highlightIds])

  const getLinkWidth = useCallback((link: any) => {
    if (!highlightIds) return 0.6
    const srcId = typeof link.source === 'string' ? link.source : link.source?.id
    const tgtId = typeof link.target === 'string' ? link.target : link.target?.id
    if (highlightIds.has(srcId) && highlightIds.has(tgtId)) return 1.8
    return 0.3
  }, [highlightIds])

  // Debug: expose ref to window for minimap
  useEffect(() => {
    if (fgRef.current) (window as any).__aurelius_fg = fgRef.current
  })

  // Physics setup — runs once on mount. Force config lives in physics.ts.
  // applyForces is idempotent; safe to re-run.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => {
    const fg = fgRef.current
    if (!fg) return
    applyForces(fg)
  }, [])

  // Re-heat only when the structural set of nodes/edges changes —
  // never on filter, search, or selection prop churn (FR-005, FR-006).
  useEffect(() => {
    const fg = fgRef.current
    if (!fg) return
    applyForces(fg)
    fg.d3ReheatSimulation?.()
  }, [nodes.length, edges.length])

  // Zoom to fit on first load
  useEffect(() => {
    const timer = setTimeout(() => fgRef.current?.zoomToFit?.(600, 80), 800)
    return () => clearTimeout(timer)
  }, [nodes.length])

  // Zoom to project cluster
  useEffect(() => {
    if (!zoomToProject || !fgRef.current) return
    const timer = setTimeout(() => {
      fgRef.current?.zoomToFit?.(600, 60, (node: any) => {
        return node.label?.startsWith(`[${zoomToProject}]`) ||
          (node._type === 'project' && node.label === zoomToProject)
      })
    }, 100)
    return () => clearTimeout(timer)
  }, [zoomToProject])

  return (
    <div className={styles.canvas}>
      <ForceGraph2D
        ref={fgRef}
        graphData={graphData}
        nodeCanvasObject={paintNode}
        nodePointerAreaPaint={paintNodeArea}
        onNodeClick={handleNodeClick}
        onNodeHover={handleNodeHover}
        onNodeDragEnd={(node: any) => {
          node.fx = undefined
          node.fy = undefined
        }}
        onRenderFramePre={() => { (window as any).__aurelius_positions = [] }}
        onBackgroundClick={() => onSelectNode(null)}
        linkColor={getLinkColor}
        linkWidth={getLinkWidth}
        linkDirectionalArrowLength={4}
        linkDirectionalArrowColor={getLinkColor}
        linkDirectionalArrowRelPos={0.85}
        linkCanvasObjectMode={() => 'after'}
        linkCanvasObject={paintLink}
        linkCurvature={0.05}
        backgroundColor="#0c1018"
        d3AlphaDecay={PHYSICS.sim.alphaDecay}
        d3VelocityDecay={PHYSICS.sim.velocityDecay}
        cooldownTicks={PHYSICS.sim.cooldownTicks}
        warmupTicks={PHYSICS.sim.warmupTicks}
        d3AlphaMin={PHYSICS.sim.alphaMin}
        enableNodeDrag={true}
        enableZoomInteraction={true}
        enablePanInteraction={true}
      />
    </div>
  )
}
