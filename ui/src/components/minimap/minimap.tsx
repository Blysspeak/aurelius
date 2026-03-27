import { useRef, useEffect } from 'react'
import styles from './minimap.module.css'

/* eslint-disable @typescript-eslint/no-explicit-any */

interface MinimapProps {
  graphRef: React.RefObject<any>
  selectedNodeId: string | null
  hasDetailPanel: boolean
}

const WIDTH = 180
const HEIGHT = 120
const PADDING = 8

export function Minimap({ graphRef, selectedNodeId, hasDetailPanel }: MinimapProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return

    let animId: number
    let lastDraw = 0

    const draw = (now: number) => {
      animId = requestAnimationFrame(draw)
      if (now - lastDraw < 100) return // ~10fps
      lastDraw = now

      const ctx = canvas.getContext('2d')
      if (!ctx) return

      const dpr = window.devicePixelRatio || 1
      canvas.width = WIDTH * dpr
      canvas.height = HEIGHT * dpr
      ctx.scale(dpr, dpr)
      ctx.clearRect(0, 0, WIDTH, HEIGHT)

      // Read node positions shared by paintNode via window
      const gNodes = (window as any).__aurelius_positions as any[] | undefined
      if (!gNodes || gNodes.length === 0) return

      // Compute bounding box
      let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity
      for (const n of gNodes) {
        if (n.x < minX) minX = n.x
        if (n.x > maxX) maxX = n.x
        if (n.y < minY) minY = n.y
        if (n.y > maxY) maxY = n.y
      }
      if (!isFinite(minX)) return

      const rangeX = maxX - minX || 1
      const rangeY = maxY - minY || 1
      const scaleX = (WIDTH - PADDING * 2) / rangeX
      const scaleY = (HEIGHT - PADDING * 2) / rangeY
      const scale = Math.min(scaleX, scaleY)
      const offsetX = (WIDTH - rangeX * scale) / 2
      const offsetY = (HEIGHT - rangeY * scale) / 2

      // Draw nodes
      for (const n of gNodes) {
        const px = (n.x - minX) * scale + offsetX
        const py = (n.y - minY) * scale + offsetY
        const isProject = n._type === 'project'
        const isSelected = n.id === selectedNodeId

        ctx.beginPath()
        ctx.arc(px, py, isSelected ? 3 : isProject ? 2.5 : 1.5, 0, 2 * Math.PI)
        ctx.fillStyle = isSelected ? '#c5a44e' : (n._color || '#6b7b8e')
        ctx.fill()
      }

      // Draw viewport rectangle
      const fg = graphRef.current
      if (fg && fg.screen2GraphCoords) {
        try {
          const graphCanvas = document.querySelector('[class*="canvas"] canvas') as HTMLCanvasElement
          if (graphCanvas) {
            const cw = graphCanvas.clientWidth
            const ch = graphCanvas.clientHeight
            const tl = fg.screen2GraphCoords(0, 0)
            const br = fg.screen2GraphCoords(cw, ch)
            if (tl && br) {
              const vx = (tl.x - minX) * scale + offsetX
              const vy = (tl.y - minY) * scale + offsetY
              const vw = (br.x - tl.x) * scale
              const vh = (br.y - tl.y) * scale
              ctx.strokeStyle = '#c5a44e40'
              ctx.lineWidth = 1
              ctx.strokeRect(vx, vy, vw, vh)
            }
          }
        } catch { /* ignore */ }
      }
    }

    animId = requestAnimationFrame(draw)
    return () => cancelAnimationFrame(animId)
  }, [graphRef, selectedNodeId])

  return (
    <div
      className={styles.minimap}
      style={hasDetailPanel ? { right: 376 } : undefined}
    >
      <canvas
        ref={canvasRef}
        style={{ width: WIDTH, height: HEIGHT }}
      />
    </div>
  )
}
