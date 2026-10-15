/// <reference lib="webworker" />
// Owns the raw /api/graph payload (80+ MB of JSON on a large memory) so the
// main thread never parses it, builds the columnar model, lays every project
// out once, and answers detail lookups for the side panel.
import { buildModel, type GraphModel } from '../lib/model'
import type { AureliusNode } from '../lib/types'
import { packGroups, placeGroup, startGroupLayout, stepGroupLayout, type GroupLayout } from './layout'
import type { FromWorker, NodeDetail, ToWorker } from './protocol'

const ctx = self as unknown as DedicatedWorkerGlobalScope
const post = (msg: FromWorker, transfer: Transferable[] = []) => ctx.postMessage(msg, transfer)

/** Ticks between intermediate frames for the project on screen. */
const PREVIEW_EVERY = 40
/** Ticks per step inside a slice. */
const STEP = 10
/** Length of one uninterrupted run before the worker reads its messages. */
const SLICE_MS = 16

let raw: AureliusNode[] = []
let model: GraphModel | null = null
let adjStart: Uint32Array | null = null
let adjEdge: Uint32Array | null = null
/** The project on screen: laid out first, and previewed while it settles. */
let priority = -1
const pending: number[] = []
const parked = new Map<number, GroupLayout>()
const done = new Set<number>()
let running: { g: number; l: GroupLayout } | null = null

async function load(url: string) {
  const t0 = performance.now()
  const resp = await fetch(url)
  if (!resp.ok) throw new Error(`${url} answered ${resp.status}`)
  const payload = await resp.json()
  raw = payload.nodes
  const m = buildModel(payload)
  packGroups(m.groups)
  model = m
  // Structured clone copies the typed arrays (a few MB); the worker keeps its own.
  post({ type: 'model', model: m, ms: performance.now() - t0 })
  if (priority < 0) priority = m.defaultGroup
  // Small projects first: the overview fills in quickly, the giant comes last.
  pending.push(...m.groups.map((_, g) => g).sort((a, b) => m.groups[a].size - m.groups[b].size))
  setTimeout(work, 0)
}

function next(): number {
  if (priority >= 0 && !done.has(priority)) return priority
  while (pending.length) {
    const g = pending.shift()!
    if (!done.has(g)) return g
  }
  return -1
}

function work() {
  const m = model
  if (!m) return
  // Work in ~16 ms slices, then yield so 'prioritize' and 'detail' messages
  // get through. (One setTimeout per 10 ticks was clamped to 4 ms by the
  // browser and stretched the full layout from ~4 s to ~17 s.)
  const deadline = performance.now() + SLICE_MS
  while (performance.now() < deadline) {
    if (running && priority >= 0 && running.g !== priority && !done.has(priority)) {
      // The user switched projects: park this one with its state and move on.
      parked.set(running.g, running.l)
      pending.unshift(running.g)
      running = null
    }
    if (!running) {
      const g = next()
      if (g < 0) return
      running = { g, l: parked.get(g) ?? startGroupLayout(m, g) }
      parked.delete(g)
    }
    const { g, l } = running
    const final = stepGroupLayout(l, STEP)
    if (final) {
      done.add(g)
      running = null
    }
    if (final || (g === priority && l.tick % PREVIEW_EVERY === 0)) {
      const xy = placeGroup(l, m.groups[g])
      post(
        { type: 'positions', g, members: l.members, xy, final, laidOut: done.size, total: m.groups.length },
        [xy.buffer],
      )
    }
  }
  setTimeout(work, 0)
}

function buildAdjacency(m: GraphModel) {
  const start = new Uint32Array(m.n + 1)
  for (let e = 0; e < m.edgeFrom.length; e++) {
    start[m.edgeFrom[e] + 1]++
    start[m.edgeTo[e] + 1]++
  }
  for (let i = 0; i < m.n; i++) start[i + 1] += start[i]
  const fill = start.slice(0, m.n)
  const list = new Uint32Array(m.edgeFrom.length * 2)
  for (let e = 0; e < m.edgeFrom.length; e++) {
    list[fill[m.edgeFrom[e]]++] = e
    list[fill[m.edgeTo[e]]++] = e
  }
  adjStart = start
  adjEdge = list
}

function detail(m: GraphModel, idx: number): NodeDetail {
  if (!adjStart || !adjEdge) buildAdjacency(m)
  const links: NodeDetail['links'] = []
  for (let s = adjStart![idx]; s < adjStart![idx + 1]; s++) {
    const e = adjEdge![s]
    const out = m.edgeFrom[e] === idx
    links.push({ rel: m.relNames[m.edgeRel[e]], other: out ? m.edgeTo[e] : m.edgeFrom[e], out })
  }
  return { node: raw[idx], links }
}

ctx.onmessage = (ev: MessageEvent<ToWorker>) => {
  const msg = ev.data
  switch (msg.type) {
    case 'load':
      load(msg.url).catch((err: unknown) => post({ type: 'error', message: String(err) }))
      break
    case 'prioritize':
      priority = msg.g
      break
    case 'detail':
      if (model) post({ type: 'detail', reqId: msg.reqId, detail: detail(model, msg.idx) })
      break
  }
}
