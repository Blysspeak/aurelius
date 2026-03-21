import { useRef, useCallback, useEffect, useMemo, useState } from 'react'
import ForceGraph2D from 'react-force-graph-2d'
import type { AureliusNode, AureliusEdge } from '../../types'
import { parseNodeType } from '../../types'
import { getNodeColor, getNodeSize } from '../../theme'
import styles from './graphCanvas.module.css'

/* eslint-disable @typescript-eslint/no-explicit-any */

interface GraphCanvasProps {
  nodes: AureliusNode[]
  edges: AureliusEdge[]
  selectedNodeId: string | null
  onSelectNode: (node: AureliusNode | null) => void
}

// ─── Position persistence (only manually dragged nodes) ─────────────
const POSITIONS_KEY = 'aurelius-node-positions'

function loadPositions(): Record<string, { x: number; y: number }> {
  try {
    return JSON.parse(localStorage.getItem(POSITIONS_KEY) || '{}')
  } catch { return {} }
}

function savePositionFor(id: string, x: number, y: number) {
  const positions = loadPositions()
  positions[id] = { x, y }
  localStorage.setItem(POSITIONS_KEY, JSON.stringify(positions))
}

// ─── Component ──────────────────────────────────────────────────────
export function GraphCanvas({ nodes, edges, selectedNodeId, onSelectNode }: GraphCanvasProps) {
  const fgRef = useRef<any>(null)
  const savedPositions = useRef(loadPositions())
  // Hover state kept local — no parent re-renders
  const [hoveredId, setHoveredId] = useState<string | null>(null)

  const nodeMap = useMemo(() => new Map(nodes.map(n => [n.id, n])), [nodes])

  // Build graph data with pinned positions for previously dragged nodes
  const graphData = useMemo(() => {
    const nodeIds = new Set(nodes.map(n => n.id))
    const positions = savedPositions.current

    return {
      nodes: nodes.map(n => {
        const type = parseNodeType(n.node_type)
        const pos = positions[n.id]
        return {
          id: n.id,
          label: n.label,
          _type: type,
          _color: getNodeColor(type),
          _size: getNodeSize(type),
          _note: n.note,
          ...(pos ? { x: pos.x, y: pos.y, fx: pos.x, fy: pos.y } : {}),
        }
      }),
      links: edges
        .filter(e => nodeIds.has(e.from_id) && nodeIds.has(e.to_id))
        .map(e => ({
          source: e.from_id,
          target: e.to_id,
          _weight: e.weight,
        })),
    }
  }, [nodes, edges])

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

    const isSelected = id === selectedNodeId
    const isHovered = id === hoveredId
    const isFocused = isSelected || isHovered
    const isHighlighted = highlightIds ? highlightIds.has(id) : true
    const dimmed = highlightIds && !isHighlighted
    const size = isFocused ? baseSize * 1.6 : baseSize

    // Outer glow for focused node
    if (isFocused) {
      const gradient = ctx.createRadialGradient(x, y, size * 0.5, x, y, size * 3)
      gradient.addColorStop(0, `${color}40`)
      gradient.addColorStop(1, `${color}00`)
      ctx.beginPath()
      ctx.arc(x, y, size * 3, 0, 2 * Math.PI)
      ctx.fillStyle = gradient
      ctx.fill()
    }

    // Neighbor glow
    if (highlightIds && isHighlighted && !isFocused) {
      ctx.beginPath()
      ctx.arc(x, y, size + 2, 0, 2 * Math.PI)
      ctx.fillStyle = `${color}20`
      ctx.fill()
    }

    // Main circle
    ctx.beginPath()
    ctx.arc(x, y, size, 0, 2 * Math.PI)
    ctx.fillStyle = dimmed ? `${color}25` : color
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

    // Labels: always for projects, on hover/select for focused + neighbors
    const showLabel = type === 'project' || isFocused || (highlightIds && isHighlighted)
    if (showLabel) {
      const truncated = label.length > 28 ? label.slice(0, 26) + '…' : label
      const fontSize = isFocused
        ? Math.max(14 / globalScale, 4)
        : type === 'project'
          ? Math.max(12 / globalScale, 3.5)
          : Math.max(10 / globalScale, 3)

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
  }, [selectedNodeId, hoveredId, highlightIds])

  const paintNodeArea = useCallback((node: any, color: string, ctx: CanvasRenderingContext2D) => {
    const { x, y, _size: size } = node
    if (x == null || y == null) return
    ctx.beginPath()
    ctx.arc(x, y, size + 4, 0, 2 * Math.PI)
    ctx.fillStyle = color
    ctx.fill()
  }, [])

  const getLinkColor = useCallback((link: any) => {
    if (!highlightIds) return '#1e2a3860'
    const srcId = typeof link.source === 'string' ? link.source : link.source?.id
    const tgtId = typeof link.target === 'string' ? link.target : link.target?.id
    if (highlightIds.has(srcId) && highlightIds.has(tgtId)) return '#c5a44e50'
    return '#1e2a3815'
  }, [highlightIds])

  const getLinkWidth = useCallback((link: any) => {
    if (!highlightIds) return 0.5
    const srcId = typeof link.source === 'string' ? link.source : link.source?.id
    const tgtId = typeof link.target === 'string' ? link.target : link.target?.id
    if (highlightIds.has(srcId) && highlightIds.has(tgtId)) return 1.5
    return 0.2
  }, [highlightIds])

  // Physics
  useEffect(() => {
    const fg = fgRef.current
    if (!fg) return
    fg.d3Force('link')?.distance(40).strength(0.7)
    fg.d3Force('charge')?.strength(-120).distanceMax(300)
    fg.d3Force('center')?.strength(0.03)
    fg.d3Force('collision', null)
    fg.d3ReheatSimulation?.()
  }, [graphData])

  // Zoom to fit on first load (skip if positions exist)
  useEffect(() => {
    if (Object.keys(savedPositions.current).length > 0) return
    const timer = setTimeout(() => fgRef.current?.zoomToFit?.(600, 80), 800)
    return () => clearTimeout(timer)
  }, [nodes.length])

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
          node.fx = node.x
          node.fy = node.y
          savePositionFor(node.id, node.x, node.y)
        }}
        onBackgroundClick={() => onSelectNode(null)}
        linkColor={getLinkColor}
        linkWidth={getLinkWidth}
        linkDirectionalArrowLength={0}
        linkCurvature={0.05}
        backgroundColor="#0c1018"
        d3AlphaDecay={0.01}
        d3VelocityDecay={0.3}
        cooldownTicks={Infinity}
        warmupTicks={100}
        d3AlphaMin={0}
        enableNodeDrag={true}
        enableZoomInteraction={true}
        enablePanInteraction={true}
      />
    </div>
  )
}
