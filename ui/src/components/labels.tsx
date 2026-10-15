'use client'

import { useEffect, useState } from 'react'
import type { GraphModel } from '@/lib/model'
import { typeColor } from '@/lib/palette'
import { stripProject } from '@/lib/types'
import type { GraphRenderer } from '@/render/graphRenderer'
import type { Scope } from './graphApp'

interface Props {
  renderer: GraphRenderer
  model: GraphModel
  scope: Scope
  hovered: { i: number; x: number; y: number } | null
  selected: number
  /** Changes whenever the drawn set changes, so labels recompute. */
  version: unknown
  matches: number[] | null
}

interface Placed {
  key: string
  x: number
  y: number
  text: string
  kind: 'node' | 'group'
}

/** Node labels appear only once a typical node is drawn this many pixels wide. */
const LABEL_MIN_PX = 16
/** Session labels are timestamps: they say nothing at a glance, so skip them. */
const TIMESTAMP = /^\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}/
const MAX_LABELS = 28
const CHAR_PX = 6.4

function clip(s: string, n: number) {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s
}

export function Labels({ renderer, model, scope, hovered, selected, version, matches }: Props) {
  const [placed, setPlaced] = useState<Placed[]>([])
  const [, setViewTick] = useState(0)

  useEffect(() => {
    let frame = 0
    const compute = () => {
      const out: Placed[] = []
      const boxes: [number, number, number, number][] = []
      const fits = (x: number, y: number, w: number, h: number) => {
        for (const b of boxes) if (x < b[2] && x + w > b[0] && y < b[3] && y + h > b[1]) return false
        boxes.push([x, y, x + w, y + h])
        return true
      }
      const scale = renderer.view.scale
      const { width, height } = renderer.size

      // Overview: name each project island that is big enough to read.
      if (scope === 'all') {
        model.groups.forEach((g, gi) => {
          const pr = g.r * scale
          if (pr < 26 || g.size === 0) return
          const [sx, sy] = renderer.worldToScreen(g.cx, g.cy + g.r)
          if (sx < -100 || sx > width + 100 || sy < -20 || sy > height + 20) return
          const text = g.name
          const w = text.length * CHAR_PX + 8
          if (fits(sx - w / 2, sy - 18, w, 16)) out.push({ key: `g${gi}`, x: sx, y: sy - 10, text, kind: 'group' })
        })
      }

      // Zoomed in: label the best-connected nodes in view, without overlaps.
      const emphasised = matches ? new Set(matches) : null
      const pool = new Set<number>()
      if (emphasised) for (const i of emphasised) if (renderer.isVisible(i)) pool.add(i)
      if (2 * 2.8 * scale >= LABEL_MIN_PX) {
        for (const i of renderer.nodesInView(400, (i) => renderer.isEmphasised(i) && !TIMESTAMP.test(stripProject(model.labels[i]))))
          pool.add(i)
      }
      for (const i of pool) {
        if (out.length >= MAX_LABELS) break
        if (i === selected) continue
        const [x, y] = renderer.getPosition(i)
        const [sx, sy] = renderer.worldToScreen(x, y)
        if (sx < 0 || sy < 0 || sx > width || sy > height) continue
        const text = clip(stripProject(model.labels[i]), 42)
        const r = renderer.radius(i) * scale
        const w = text.length * CHAR_PX
        if (fits(sx + r - 2, sy - 11, w + 10, 22)) out.push({ key: `n${i}`, x: sx + r + 4, y: sy, text, kind: 'node' })
      }
      setPlaced(out)
      setViewTick((t) => t + 1)
    }
    const schedule = () => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(compute)
    }
    schedule()
    const off = renderer.onViewChange(schedule)
    return () => {
      off()
      cancelAnimationFrame(frame)
    }
  }, [renderer, model, scope, selected, version, matches])

  const ring = (i: number, cls: string) => {
    if (i < 0 || !renderer.isVisible(i)) return null
    const [x, y] = renderer.getPosition(i)
    const [sx, sy] = renderer.worldToScreen(x, y)
    const r = renderer.radius(i) * renderer.view.scale * (i === selected ? 1.5 : 1) + 4
    return <span className={cls} style={{ left: sx - r, top: sy - r, width: r * 2, height: r * 2 }} />
  }

  const selectedLabel = () => {
    if (selected < 0 || !renderer.isVisible(selected)) return null
    const [x, y] = renderer.getPosition(selected)
    const [sx, sy] = renderer.worldToScreen(x, y)
    const r = renderer.radius(selected) * renderer.view.scale * 1.5
    return (
      <span className="label label-selected" style={{ left: sx + r + 8, top: sy }}>
        {clip(stripProject(model.labels[selected]), 60)}
      </span>
    )
  }

  return (
    <div className="overlay" aria-hidden="true">
      {placed.map((p) => (
        <span key={p.key} className={p.kind === 'group' ? 'label label-group' : 'label'} style={{ left: p.x, top: p.y }}>
          {p.text}
        </span>
      ))}
      {ring(selected, 'ring ring-selected')}
      {selectedLabel()}
      {hovered && hovered.i !== selected && (
        <>
          {ring(hovered.i, 'ring')}
          <div className="tip" style={{ left: hovered.x + 14, top: hovered.y + 12 }}>
            <span className="tip-type">
              <i className="dot" style={{ background: typeColor(model.typeNames[model.type[hovered.i]]) }} />
              {model.typeNames[model.type[hovered.i]].replace('_', ' ')}
            </span>
            <span className="tip-label">{clip(stripProject(model.labels[hovered.i]), 140)}</span>
          </div>
        </>
      )}
    </div>
  )
}
