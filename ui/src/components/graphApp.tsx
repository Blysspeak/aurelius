'use client'

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { searchNodes, GRAPH_URL } from '@/lib/api'
import type { GraphModel } from '@/lib/model'
import type { FromWorker, NodeDetail, ToWorker } from '@/layout/protocol'
import { GraphRenderer, type Backend } from '@/render/graphRenderer'
import { Rail } from './rail'
import { DetailPanel } from './detailPanel'
import { Labels } from './labels'

/** 'all' or a group index. */
export type Scope = 'all' | number

interface Progress {
  laidOut: number
  total: number
}

function scopeFromHash(model: GraphModel): Scope | null {
  const m = window.location.hash.match(/p=([^&]+)/)
  if (!m) return null
  const name = decodeURIComponent(m[1])
  if (name === '*') return 'all'
  const g = model.groups.findIndex((x) => x.name.toLowerCase() === name.toLowerCase())
  return g >= 0 ? g : null
}

export function GraphApp() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const rendererRef = useRef<GraphRenderer | null>(null)
  const workerRef = useRef<Worker | null>(null)
  const readyGroups = useRef(new Set<number>())
  const detailReq = useRef(0)

  const [model, setModel] = useState<GraphModel | null>(null)
  const [backend, setBackend] = useState<Backend | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [progress, setProgress] = useState<Progress>({ laidOut: 0, total: 0 })
  const [layoutTick, setLayoutTick] = useState(0)
  const [scope, setScope] = useState<Scope>('all')
  const [hiddenTypes, setHiddenTypes] = useState<Set<number>>(new Set())
  const [query, setQuery] = useState('')
  const [matches, setMatches] = useState<number[] | null>(null)
  const [searching, setSearching] = useState(false)
  const [selected, setSelected] = useState(-1)
  const [hovered, setHovered] = useState<{ i: number; x: number; y: number } | null>(null)
  const [detail, setDetail] = useState<NodeDetail | null>(null)
  const [stats, setStats] = useState({ nodes: 0, edges: 0 })

  const idIndex = useMemo(() => {
    const map = new Map<string, number>()
    model?.ids.forEach((id, i) => map.set(id, i))
    return map
  }, [model])

  // Renderer and worker live for the page's lifetime.
  useEffect(() => {
    let disposed = false
    const canvas = canvasRef.current!
    const forceWebGL = new URLSearchParams(window.location.search).has('webgl')
    GraphRenderer.create(canvas, forceWebGL)
      .then((r) => {
        if (disposed) return r.dispose()
        rendererRef.current = r
        setBackend(r.backend)
        const worker = new Worker(new URL('../layout/graph.worker.ts', import.meta.url))
        workerRef.current = worker
        worker.onmessage = (ev: MessageEvent<FromWorker>) => {
          const msg = ev.data
          if (msg.type === 'model') {
            r.setModel(msg.model)
            setModel(msg.model)
          } else if (msg.type === 'positions') {
            r.updatePositions(msg.members, msg.xy)
            readyGroups.current.add(msg.g)
            setProgress({ laidOut: msg.laidOut, total: msg.total })
            setLayoutTick((t) => t + 1)
          } else if (msg.type === 'detail') {
            if (msg.reqId === detailReq.current) setDetail(msg.detail)
          } else if (msg.type === 'error') {
            setError(msg.message)
          }
        }
        worker.postMessage({ type: 'load', url: GRAPH_URL } satisfies ToWorker)
      })
      .catch((err: unknown) => setError(`This browser cannot draw the graph: ${String(err)}`))
    // The canvas also changes size when the detail panel opens, not only on
    // window resize; a stale size would offset picking and labels.
    const observer = new ResizeObserver(() => rendererRef.current?.resize())
    observer.observe(canvas)
    return () => {
      disposed = true
      observer.disconnect()
      workerRef.current?.terminate()
      rendererRef.current?.dispose()
      rendererRef.current = null
    }
  }, [])

  // First model: open on the project from the URL, else the most recently active one.
  useEffect(() => {
    if (!model) return
    const s = scopeFromHash(model) ?? model.defaultGroup
    setScope(s)
  }, [model])

  // Tell the worker which project is on screen, keep the URL shareable, frame it.
  useEffect(() => {
    if (!model) return
    if (scope !== 'all') workerRef.current?.postMessage({ type: 'prioritize', g: scope } satisfies ToWorker)
    const name = scope === 'all' ? '*' : model.groups[scope].name
    history.replaceState(null, '', `#p=${encodeURIComponent(name)}`)
    frameScope(rendererRef.current, model, scope, true)
  }, [model, scope])

  // Visible set: scope, type filters and layout readiness.
  const visible = useMemo(() => {
    if (!model) return null
    void layoutTick
    const v = new Uint8Array(model.n)
    for (let i = 0; i < model.n; i++) {
      const g = model.group[i]
      if (scope !== 'all' && g !== scope) continue
      if (!readyGroups.current.has(g)) continue
      if (hiddenTypes.has(model.type[i])) continue
      v[i] = 1
    }
    return v
  }, [model, scope, hiddenTypes, layoutTick])

  const emphasis = useMemo(() => {
    if (!model || !matches) return null
    const e = new Uint8Array(model.n)
    for (const i of matches) e[i] = 1
    return e
  }, [model, matches])

  useEffect(() => {
    const r = rendererRef.current
    if (!r || !visible) return
    r.rebuild(visible, emphasis, selected)
    setStats(r.counts())
  }, [visible, emphasis, selected])

  // Detail for the selected node comes from the worker, which holds notes and data.
  useEffect(() => {
    if (selected < 0) {
      setDetail(null)
      return
    }
    const reqId = ++detailReq.current
    workerRef.current?.postMessage({ type: 'detail', idx: selected, reqId } satisfies ToWorker)
  }, [selected])

  // Server-side search, debounced; results highlight, they do not filter.
  useEffect(() => {
    const q = query.trim()
    if (!q) {
      setMatches(null)
      setSearching(false)
      return
    }
    const ctl = new AbortController()
    setSearching(true)
    const t = setTimeout(() => {
      searchNodes(q, ctl.signal)
        .then((ids) => {
          setMatches(ids.map((id) => idIndex.get(id)).filter((i): i is number => i !== undefined))
          setSearching(false)
        })
        .catch((err: unknown) => {
          if ((err as Error).name !== 'AbortError') {
            setMatches([])
            setSearching(false)
          }
        })
    }, 220)
    return () => {
      clearTimeout(t)
      ctl.abort()
    }
  }, [query, idIndex])

  const focusNode = useCallback(
    (i: number) => {
      if (!model) return
      const g = model.group[i]
      if (scope !== 'all' && scope !== g) setScope(g)
      setHiddenTypes((h) => (h.has(model.type[i]) ? new Set([...h].filter((t) => t !== model.type[i])) : h))
      setSelected(i)
      const r = rendererRef.current
      if (!r) return
      const go = () => {
        const [x, y] = r.getPosition(i)
        r.animateTo({ cx: x, cy: y, scale: Math.max(r.view.scale, 3) })
      }
      if (readyGroups.current.has(g)) go()
      else setTimeout(go, 600)
    },
    [model, scope],
  )

  // Canvas interaction: drag to pan, wheel to zoom, click to select.
  useEffect(() => {
    const canvas = canvasRef.current!
    let down: { x: number; y: number; moved: boolean } | null = null
    let hoverFrame = 0
    const onDown = (e: PointerEvent) => {
      canvas.setPointerCapture(e.pointerId)
      down = { x: e.clientX, y: e.clientY, moved: false }
    }
    const onMove = (e: PointerEvent) => {
      const r = rendererRef.current
      if (!r) return
      if (down) {
        const dx = e.clientX - down.x
        const dy = e.clientY - down.y
        if (down.moved || Math.hypot(dx, dy) > 3) {
          down.moved = true
          r.panBy(dx, dy)
          down.x = e.clientX
          down.y = e.clientY
          setHovered(null)
        }
        return
      }
      const rect = canvas.getBoundingClientRect()
      const sx = e.clientX - rect.left
      const sy = e.clientY - rect.top
      cancelAnimationFrame(hoverFrame)
      hoverFrame = requestAnimationFrame(() => {
        const i = r.pick(sx, sy)
        setHovered(i >= 0 ? { i, x: sx, y: sy } : null)
        canvas.style.cursor = i >= 0 ? 'pointer' : 'grab'
      })
    }
    const onUp = (e: PointerEvent) => {
      const r = rendererRef.current
      if (down && !down.moved && r) {
        const rect = canvas.getBoundingClientRect()
        const i = r.pick(e.clientX - rect.left, e.clientY - rect.top)
        setSelected(i)
      }
      down = null
    }
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const r = rendererRef.current
      if (!r) return
      const rect = canvas.getBoundingClientRect()
      const delta = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY
      r.zoomAt(e.clientX - rect.left, e.clientY - rect.top, Math.exp(-delta * 0.0015))
    }
    const onLeave = () => setHovered(null)
    canvas.addEventListener('pointerdown', onDown)
    canvas.addEventListener('pointermove', onMove)
    canvas.addEventListener('pointerup', onUp)
    canvas.addEventListener('pointerleave', onLeave)
    canvas.addEventListener('wheel', onWheel, { passive: false })
    return () => {
      canvas.removeEventListener('pointerdown', onDown)
      canvas.removeEventListener('pointermove', onMove)
      canvas.removeEventListener('pointerup', onUp)
      canvas.removeEventListener('pointerleave', onLeave)
      canvas.removeEventListener('wheel', onWheel)
    }
  }, [])

  // Keyboard: "/" focuses search, Escape clears selection, then the query.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = (e.target as HTMLElement).tagName === 'INPUT'
      if (e.key === '/' && !typing) {
        e.preventDefault()
        document.getElementById('search')?.focus()
      } else if (e.key === 'Escape') {
        if (selected >= 0) setSelected(-1)
        else if (query) setQuery('')
        ;(document.activeElement as HTMLElement | null)?.blur()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [selected, query])

  // Test and measurement hooks (used by the Playwright check).
  useEffect(() => {
    const w = window as unknown as Record<string, unknown>
    w.__aurelius = {
      bench: (ms?: number) => rendererRef.current?.bench(ms),
      setScope: (name: string) => {
        if (!model) return false
        if (name === '*') setScope('all')
        else {
          const g = model.groups.findIndex((x) => x.name === name)
          if (g < 0) return false
          setScope(g)
        }
        return true
      },
      state: () => ({ backend, progress, scope, stats, selected, n: model?.n ?? 0 }),
      select: (i: number) => focusNode(i),
    }
  }, [model, backend, progress, scope, stats, selected, focusNode])

  const toggleType = useCallback((t: number) => {
    setHiddenTypes((h) => {
      const next = new Set(h)
      if (next.has(t)) next.delete(t)
      else next.add(t)
      return next
    })
  }, [])

  const scopeReady = model && (scope === 'all' ? progress.laidOut >= progress.total : readyGroups.current.has(scope))

  return (
    <div className="app">
      <Rail
        model={model}
        scope={scope}
        onScope={(s) => {
          setScope(s)
          setSelected(-1)
        }}
        hiddenTypes={hiddenTypes}
        onToggleType={toggleType}
        onShowAllTypes={() => setHiddenTypes(new Set())}
        query={query}
        onQuery={setQuery}
        matches={matches}
        searching={searching}
        onPick={focusNode}
        backend={backend}
      />
      <main className="stage">
        <canvas ref={canvasRef} className="canvas" aria-label="Memory graph" />
        {model && rendererRef.current && (
          <Labels
            renderer={rendererRef.current}
            model={model}
            scope={scope}
            hovered={hovered}
            selected={selected}
            version={stats}
            matches={matches}
          />
        )}
        <footer className="status">
          {error ? (
            <span className="status-error">{error}</span>
          ) : !model ? (
            <span>Reading the graph from au</span>
          ) : (
            <>
              <span>{stats.nodes.toLocaleString('en')} nodes</span>
              <span>{stats.edges.toLocaleString('en')} links</span>
              {!scopeReady && <span className="status-busy">Placing nodes</span>}
              {progress.laidOut < progress.total && (
                <span className="status-faint">
                  {progress.laidOut} of {progress.total} projects laid out
                </span>
              )}
            </>
          )}
        </footer>
        <div className="zoom">
          <button type="button" onClick={() => zoomBy(rendererRef.current, 1.4)} aria-label="Zoom in">
            +
          </button>
          <button type="button" onClick={() => zoomBy(rendererRef.current, 1 / 1.4)} aria-label="Zoom out">
            −
          </button>
          <button
            type="button"
            onClick={() => model && frameScope(rendererRef.current, model, scope, true)}
            aria-label="Fit to view"
          >
            Fit
          </button>
        </div>
      </main>
      {selected >= 0 && model && (
        <DetailPanel model={model} index={selected} detail={detail} onClose={() => setSelected(-1)} onPick={focusNode} />
      )}
    </div>
  )
}

function zoomBy(r: GraphRenderer | null, f: number) {
  if (r) r.animateTo({ ...r.view, scale: r.view.scale * f }, 200)
}

function frameScope(r: GraphRenderer | null, model: GraphModel, scope: Scope, animate: boolean) {
  if (!r) return
  if (scope === 'all') {
    let x0 = Infinity
    let y0 = Infinity
    let x1 = -Infinity
    let y1 = -Infinity
    for (const g of model.groups) {
      x0 = Math.min(x0, g.cx - g.r)
      y0 = Math.min(y0, g.cy - g.r)
      x1 = Math.max(x1, g.cx + g.r)
      y1 = Math.max(y1, g.cy + g.r)
    }
    r.fitCircle((x0 + x1) / 2, (y0 + y1) / 2, Math.max(x1 - x0, y1 - y0) / 2, animate ? 500 : 0)
  } else {
    const g = model.groups[scope]
    r.fitCircle(g.cx, g.cy, g.r, animate ? 500 : 0)
  }
}
