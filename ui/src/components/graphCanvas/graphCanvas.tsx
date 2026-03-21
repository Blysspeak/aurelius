import { useRef, useCallback, useEffect, useMemo } from 'react'
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

export function GraphCanvas({ nodes, edges, selectedNodeId, onSelectNode }: GraphCanvasProps) {
  const fgRef = useRef<any>(null)

  // Build a lookup map for original nodes
  const nodeMap = useMemo(() => new Map(nodes.map(n => [n.id, n])), [nodes])

  // Build graph data with extra rendering info
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
          _size: getNodeSize(type),
          _note: n.note,
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

  // Connected node IDs for highlighting
  const connectedIds = useMemo(() => {
    if (!selectedNodeId) return null
    const ids = new Set<string>([selectedNodeId])
    for (const e of edges) {
      if (e.from_id === selectedNodeId) ids.add(e.to_id)
      if (e.to_id === selectedNodeId) ids.add(e.from_id)
    }
    return ids
  }, [selectedNodeId, edges])

  const handleNodeClick = useCallback((node: any) => {
    const orig = nodeMap.get(node.id)
    onSelectNode(orig || null)
  }, [onSelectNode, nodeMap])

  const paintNode = useCallback((node: any, ctx: CanvasRenderingContext2D, globalScale: number) => {
    const { x, y, _color: color, _size: baseSize, _type: type, label, id } = node
    if (x == null || y == null) return

    const isSelected = id === selectedNodeId
    const isConnected = connectedIds ? connectedIds.has(id) : true
    const dimmed = connectedIds && !isConnected
    const size = isSelected ? baseSize * 1.6 : baseSize

    // Outer glow for selected node
    if (isSelected) {
      const gradient = ctx.createRadialGradient(x, y, size * 0.5, x, y, size * 3)
      gradient.addColorStop(0, `${color}40`)
      gradient.addColorStop(1, `${color}00`)
      ctx.beginPath()
      ctx.arc(x, y, size * 3, 0, 2 * Math.PI)
      ctx.fillStyle = gradient
      ctx.fill()
    }

    // Connected glow
    if (connectedIds && isConnected && !isSelected) {
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
    }

    // Label
    const showLabel = globalScale > 0.6 || type === 'project' || type === 'crate' || isSelected
    if (showLabel && !dimmed) {
      const truncated = label.length > 22 ? label.slice(0, 20) + '…' : label
      const fontSize = isSelected
        ? Math.max(14 / globalScale, 4)
        : type === 'project'
          ? Math.max(12 / globalScale, 3.5)
          : Math.max(10 / globalScale, 3)

      ctx.font = `${isSelected ? 600 : type === 'project' ? 500 : 400} ${fontSize}px Inter, -apple-system, sans-serif`
      ctx.textAlign = 'center'
      ctx.textBaseline = 'top'

      // Text shadow for readability
      ctx.fillStyle = '#0c1018'
      ctx.fillText(truncated, x + 0.5, y + size + 3.5)
      ctx.fillText(truncated, x - 0.5, y + size + 2.5)

      // Text
      ctx.fillStyle = isSelected ? '#e6edf3' : dimmed ? '#4a556830' : '#8b949ecc'
      ctx.fillText(truncated, x, y + size + 3)
    }
  }, [selectedNodeId, connectedIds])

  const paintNodeArea = useCallback((node: any, color: string, ctx: CanvasRenderingContext2D) => {
    const { x, y, _size: size } = node
    if (x == null || y == null) return
    ctx.beginPath()
    ctx.arc(x, y, size + 4, 0, 2 * Math.PI)
    ctx.fillStyle = color
    ctx.fill()
  }, [])

  const getLinkColor = useCallback((link: any) => {
    if (!connectedIds) return '#1e2a3860'
    const srcId = typeof link.source === 'string' ? link.source : link.source?.id
    const tgtId = typeof link.target === 'string' ? link.target : link.target?.id
    const srcConnected = connectedIds.has(srcId)
    const tgtConnected = connectedIds.has(tgtId)
    if (srcConnected && tgtConnected) return '#c5a44e50'
    return '#1e2a3815'
  }, [connectedIds])

  const getLinkWidth = useCallback((link: any) => {
    if (!connectedIds) return 0.5
    const srcId = typeof link.source === 'string' ? link.source : link.source?.id
    const tgtId = typeof link.target === 'string' ? link.target : link.target?.id
    if (connectedIds.has(srcId) && connectedIds.has(tgtId)) return 1.5
    return 0.2
  }, [connectedIds])

  // Obsidian-style physics: spring links, gentle repulsion, no fixed center
  useEffect(() => {
    const fg = fgRef.current
    if (!fg) return

    // Spring-like links: connected nodes attract
    fg.d3Force('link')
      ?.distance(40)
      .strength(0.7)

    // Gentle repulsion between all nodes
    fg.d3Force('charge')?.strength(-120).distanceMax(300)

    // Soft centering (keeps graph from drifting, not pinning)
    fg.d3Force('center')?.strength(0.03)

    // Collision to prevent overlap
    fg.d3Force('collision', null)

    fg.d3ReheatSimulation?.()
  }, [graphData])

  // Zoom to fit on load
  useEffect(() => {
    const timer = setTimeout(() => {
      fgRef.current?.zoomToFit?.(600, 80)
    }, 800)
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
        onNodeDragEnd={(node: any) => {
          // Release node — let it float naturally (Obsidian behavior)
          node.fx = undefined
          node.fy = undefined
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
