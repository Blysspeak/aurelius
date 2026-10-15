import * as THREE from 'three/webgpu'
import { float, instancedBufferAttribute, max, mix, smoothstep, uniform, uv } from 'three/tsl'
import type { GraphModel } from '../lib/model'
import { hexToRgb, typeColor, typeRadius } from '../lib/palette'

// Draws the whole graph in two draw calls: every node is one instance of a
// sprite (position, radius and colour live in instanced buffers), every
// visible edge is one segment of a single LineSegments buffer. Buffers are
// rewritten only when positions, filters or emphasis change; a frame is just
// a camera uniform update. Rendering is on demand: no frame is drawn unless
// something changed.

const BG = '#11181d'
const MIN_NODE_PX = 2.2
const DIM = 0.16
/** Largest zoom a 'fit' may choose, in pixels per layout unit. */
const MAX_FIT_SCALE = 3.2

export type Backend = 'WebGPU' | 'WebGL2'

export interface View {
  cx: number
  cy: number
  /** Pixels per layout unit. */
  scale: number
}

export class GraphRenderer {
  readonly backend: Backend
  readonly canvas: HTMLCanvasElement
  private renderer: THREE.WebGPURenderer
  private scene = new THREE.Scene()
  private camera = new THREE.OrthographicCamera(-1, 1, 1, -1, -100, 100)
  private model: GraphModel | null = null
  private positions = new Float32Array(0)
  private visible: Uint8Array = new Uint8Array(0)
  private emphasis: Uint8Array | null = null
  private selected = -1
  private nodeRadius = new Float32Array(0)
  private nodeRgb = new Float32Array(0)
  private sprite: THREE.Sprite | null = null
  private posAttr: THREE.InstancedBufferAttribute | null = null
  private radAttr: THREE.InstancedBufferAttribute | null = null
  private colAttr: THREE.InstancedBufferAttribute | null = null
  private lines: THREE.LineSegments | null = null
  private minWorld = uniform(1)
  private width = 1
  private height = 1
  private dirty = true
  /** Frames actually drawn; the bench divides this by elapsed time. */
  private renders = 0
  private raf = 0
  private listeners = new Set<(v: View) => void>()
  private animation: { from: View; to: View; t0: number; ms: number } | null = null
  /** Spatial hash of visible nodes for picking and label culling. */
  private grid = new Map<number, number[]>()
  private gridCell = 24
  view: View = { cx: 0, cy: 0, scale: 1 }
  /** Edge relation indices that are not drawn (belongs_to: the islands show it). */
  hiddenRelations = new Set<number>()

  private constructor(canvas: HTMLCanvasElement, renderer: THREE.WebGPURenderer, backend: Backend) {
    this.canvas = canvas
    this.renderer = renderer
    this.backend = backend
    this.scene.background = new THREE.Color(BG)
    this.camera.position.z = 10
  }

  static async create(canvas: HTMLCanvasElement, forceWebGL = false): Promise<GraphRenderer> {
    // WebGPURenderer picks WebGPU when an adapter is available and falls back
    // to its WebGL2 backend otherwise; forceWebGL exists to test that path.
    const renderer = new THREE.WebGPURenderer({ canvas, antialias: true, forceWebGL })
    await renderer.init()
    const backend: Backend = (renderer.backend as { isWebGPUBackend?: boolean }).isWebGPUBackend ? 'WebGPU' : 'WebGL2'
    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2))
    const r = new GraphRenderer(canvas, renderer, backend)
    r.resize()
    r.loop()
    return r
  }

  setModel(model: GraphModel) {
    this.model = model
    const n = model.n
    this.positions = new Float32Array(n * 2)
    this.visible = new Uint8Array(n)
    this.nodeRadius = new Float32Array(n)
    this.nodeRgb = new Float32Array(n * 3)
    const rgbByType = model.typeNames.map((t) => hexToRgb(typeColor(t)))
    for (let i = 0; i < n; i++) {
      const t = model.typeNames[model.type[i]]
      // Degree grows a node gently: hubs of discussion stand out, leaves stay small.
      this.nodeRadius[i] = typeRadius(t) * (t === 'project' ? 1 : 1 + Math.min(0.6, Math.log2(1 + model.degree[i]) * 0.1))
      const c = rgbByType[model.type[i]]
      this.nodeRgb.set(c, i * 3)
    }
    this.hiddenRelations = new Set([model.relNames.indexOf('belongs_to')])

    if (this.sprite) this.scene.remove(this.sprite)
    if (this.lines) this.scene.remove(this.lines)

    this.posAttr = new THREE.InstancedBufferAttribute(new Float32Array(n * 3), 3)
    this.radAttr = new THREE.InstancedBufferAttribute(new Float32Array(n), 1)
    this.colAttr = new THREE.InstancedBufferAttribute(new Float32Array(n * 3), 3)
    for (const a of [this.posAttr, this.radAttr, this.colAttr]) a.setUsage(THREE.DynamicDrawUsage)

    const material = new THREE.SpriteNodeMaterial({ transparent: true, depthWrite: false, depthTest: false })
    material.positionNode = instancedBufferAttribute(this.posAttr, 'vec3')
    // Diameter in world units, never below MIN_NODE_PX on screen.
    material.scaleNode = max(instancedBufferAttribute(this.radAttr, 'float'), this.minWorld).mul(2)
    const dist = uv().sub(0.5).length()
    const fill = instancedBufferAttribute<'vec3'>(this.colAttr, 'vec3')
    // A darker rim separates touching beads without outlines on the canvas.
    material.colorNode = mix(fill.mul(0.55), fill, smoothstep(float(0.43), float(0.38), dist))
    material.opacityNode = smoothstep(float(0.5), float(0.46), dist)
    const sprite = new THREE.Sprite(material)
    sprite.count = 0
    sprite.frustumCulled = false
    sprite.renderOrder = 2
    this.sprite = sprite

    const e = model.edgeFrom.length
    const geo = new THREE.BufferGeometry()
    const lp = new THREE.BufferAttribute(new Float32Array(e * 6), 3)
    const lc = new THREE.BufferAttribute(new Float32Array(e * 6), 3)
    lp.setUsage(THREE.DynamicDrawUsage)
    lc.setUsage(THREE.DynamicDrawUsage)
    geo.setAttribute('position', lp)
    geo.setAttribute('color', lc)
    geo.setDrawRange(0, 0)
    const lineMat = new THREE.LineBasicNodeMaterial({ vertexColors: true, transparent: true, opacity: 0.32, depthWrite: false, depthTest: false })
    const lines = new THREE.LineSegments(geo, lineMat)
    lines.frustumCulled = false
    lines.renderOrder = 1
    this.lines = lines

    this.scene.add(lines)
    this.scene.add(sprite)
  }

  /** Write positions for a set of nodes (from a worker message). */
  updatePositions(members: Uint32Array, xy: Float32Array) {
    for (let k = 0; k < members.length; k++) {
      const i = members[k]
      this.positions[i * 2] = xy[k * 2]
      this.positions[i * 2 + 1] = xy[k * 2 + 1]
    }
  }

  getPosition(i: number): [number, number] {
    return [this.positions[i * 2], this.positions[i * 2 + 1]]
  }

  radius(i: number): number {
    return Math.max(this.nodeRadius[i], this.minWorld.value as number)
  }

  /**
   * Rebuild GPU buffers for the visible set. `emphasis` (when given) marks the
   * nodes to keep at full colour; the rest are dimmed toward the background.
   */
  rebuild(visible: Uint8Array, emphasis: Uint8Array | null, selected: number) {
    const m = this.model
    if (!m || !this.posAttr || !this.radAttr || !this.colAttr || !this.sprite || !this.lines) return
    this.visible = visible
    this.emphasis = emphasis
    this.selected = selected
    const pos = this.posAttr.array as Float32Array
    const rad = this.radAttr.array as Float32Array
    const col = this.colAttr.array as Float32Array
    const bg = hexToRgb(BG)
    const p = this.positions
    let k = 0
    this.grid.clear()
    // Dimmed nodes first so emphasised ones draw on top of them.
    for (let pass = 0; pass < 2; pass++) {
      for (let i = 0; i < m.n; i++) {
        if (!visible[i]) continue
        const strong = !emphasis || emphasis[i] === 1 || i === selected
        if ((pass === 0) === strong) continue
        pos[k * 3] = p[i * 2]
        pos[k * 3 + 1] = p[i * 2 + 1]
        pos[k * 3 + 2] = 0
        rad[k] = this.nodeRadius[i] * (i === selected ? 1.5 : 1)
        const f = strong ? 1 : DIM
        for (let c = 0; c < 3; c++) col[k * 3 + c] = this.nodeRgb[i * 3 + c] * f + bg[c] * (1 - f)
        k++
        const key = this.cellKey(p[i * 2], p[i * 2 + 1])
        let list = this.grid.get(key)
        if (!list) this.grid.set(key, (list = []))
        list.push(i)
      }
    }
    this.sprite.count = k
    // A zero-count draw is legal but WebGPU warns about it on every frame.
    this.sprite.visible = k > 0
    for (const a of [this.posAttr, this.radAttr, this.colAttr]) {
      a.needsUpdate = true
    }

    const lp = this.lines.geometry.getAttribute('position') as THREE.BufferAttribute
    const lc = this.lines.geometry.getAttribute('color') as THREE.BufferAttribute
    const la = lp.array as Float32Array
    const ca = lc.array as Float32Array
    let s = 0
    for (let e = 0; e < m.edgeFrom.length; e++) {
      const a = m.edgeFrom[e]
      const b = m.edgeTo[e]
      if (!visible[a] || !visible[b]) continue
      const touchesSel = a === selected || b === selected
      if (this.hiddenRelations.has(m.edgeRel[e]) && !touchesSel) continue
      const strong = touchesSel || !emphasis || (emphasis[a] === 1 && emphasis[b] === 1)
      la[s * 6] = p[a * 2]
      la[s * 6 + 1] = p[a * 2 + 1]
      la[s * 6 + 2] = 0
      la[s * 6 + 3] = p[b * 2]
      la[s * 6 + 4] = p[b * 2 + 1]
      la[s * 6 + 5] = 0
      // Edges take the colour of their source, faded toward the background.
      const f = touchesSel ? 1 : strong ? 0.55 : 0.08
      for (let c = 0; c < 3; c++) {
        const v = this.nodeRgb[a * 3 + c] * f + bg[c] * (1 - f)
        ca[s * 6 + c] = v
        ca[s * 6 + 3 + c] = this.nodeRgb[b * 3 + c] * f + bg[c] * (1 - f)
      }
      s++
    }
    this.lines.geometry.setDrawRange(0, s * 2)
    this.lines.visible = s > 0
    lp.needsUpdate = true
    lc.needsUpdate = true
    this.requestRender()
  }

  private cellKey(x: number, y: number) {
    return Math.floor(x / this.gridCell) * 100003 + Math.floor(y / this.gridCell)
  }

  /** Visible node under a screen point, or -1. */
  pick(sx: number, sy: number): number {
    const [wx, wy] = this.screenToWorld(sx, sy)
    const slop = 4 / this.view.scale
    let best = -1
    let bestD = Infinity
    const cx = Math.floor(wx / this.gridCell)
    const cy = Math.floor(wy / this.gridCell)
    const reach = Math.ceil((12 + slop) / this.gridCell)
    for (let ox = -reach; ox <= reach; ox++) {
      for (let oy = -reach; oy <= reach; oy++) {
        const list = this.grid.get((cx + ox) * 100003 + (cy + oy))
        if (!list) continue
        for (const i of list) {
          const dx = this.positions[i * 2] - wx
          const dy = this.positions[i * 2 + 1] - wy
          const d = Math.hypot(dx, dy)
          if (d <= this.radius(i) + slop && d < bestD) {
            bestD = d
            best = i
          }
        }
      }
    }
    return best
  }

  /** Visible nodes inside the viewport, up to `limit`, biggest first. */
  nodesInView(limit: number, filter?: (i: number) => boolean): number[] {
    const [x0, y1] = this.screenToWorld(0, 0)
    const [x1, y0] = this.screenToWorld(this.width, this.height)
    const out: number[] = []
    const c0 = Math.floor(x0 / this.gridCell)
    const c1 = Math.floor(x1 / this.gridCell)
    const r0 = Math.floor(y0 / this.gridCell)
    const r1 = Math.floor(y1 / this.gridCell)
    if ((c1 - c0 + 1) * (r1 - r0 + 1) > 40000) return out
    for (let cx = c0; cx <= c1; cx++) {
      for (let cy = r0; cy <= r1; cy++) {
        const list = this.grid.get(cx * 100003 + cy)
        if (!list) continue
        for (const i of list) if (!filter || filter(i)) out.push(i)
      }
    }
    const m = this.model!
    out.sort((a, b) => m.degree[b] - m.degree[a])
    return out.slice(0, limit)
  }

  screenToWorld(sx: number, sy: number): [number, number] {
    const v = this.view
    return [v.cx + (sx - this.width / 2) / v.scale, v.cy - (sy - this.height / 2) / v.scale]
  }

  worldToScreen(wx: number, wy: number): [number, number] {
    const v = this.view
    return [(wx - v.cx) * v.scale + this.width / 2, -(wy - v.cy) * v.scale + this.height / 2]
  }

  setView(v: View) {
    this.view = { ...v, scale: Math.min(60, Math.max(0.01, v.scale)) }
    this.applyCamera()
  }

  /** Fit a world-space circle into the viewport, with an eased move. */
  fitCircle(cx: number, cy: number, r: number, ms = 450) {
    // Capped so a 100-node project does not open with beads the size of coins.
    const scale = Math.min(MAX_FIT_SCALE, (Math.min(this.width, this.height) / (2 * Math.max(r, 10))) * 0.92)
    this.animateTo({ cx, cy, scale }, ms)
  }

  animateTo(to: View, ms = 450) {
    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    if (ms <= 0 || reduced) {
      this.animation = null
      this.setView(to)
      return
    }
    this.animation = { from: { ...this.view }, to, t0: performance.now(), ms }
    this.requestRender()
  }

  zoomAt(sx: number, sy: number, factor: number) {
    this.animation = null
    const [wx, wy] = this.screenToWorld(sx, sy)
    const scale = Math.min(60, Math.max(0.01, this.view.scale * factor))
    // Keep the world point under the cursor fixed.
    const cx = wx - (sx - this.width / 2) / scale
    const cy = wy + (sy - this.height / 2) / scale
    this.setView({ cx, cy, scale })
  }

  panBy(dxPx: number, dyPx: number) {
    this.animation = null
    this.setView({ cx: this.view.cx - dxPx / this.view.scale, cy: this.view.cy + dyPx / this.view.scale, scale: this.view.scale })
  }

  onViewChange(fn: (v: View) => void): () => void {
    this.listeners.add(fn)
    return () => this.listeners.delete(fn)
  }

  resize() {
    const rect = this.canvas.getBoundingClientRect()
    this.width = Math.max(1, rect.width)
    this.height = Math.max(1, rect.height)
    this.renderer.setSize(this.width, this.height, false)
    this.applyCamera()
  }

  private applyCamera() {
    const v = this.view
    const hw = this.width / (2 * v.scale)
    const hh = this.height / (2 * v.scale)
    this.camera.left = -hw
    this.camera.right = hw
    this.camera.top = hh
    this.camera.bottom = -hh
    this.camera.position.set(v.cx, v.cy, 10)
    this.camera.updateProjectionMatrix()
    this.minWorld.value = MIN_NODE_PX / v.scale
    this.requestRender()
    for (const fn of this.listeners) fn(this.view)
  }

  requestRender() {
    this.dirty = true
  }

  private loop = () => {
    this.raf = requestAnimationFrame(this.loop)
    if (this.animation) {
      const a = this.animation
      const t = Math.min(1, (performance.now() - a.t0) / a.ms)
      const e = 1 - Math.pow(1 - t, 3)
      // Interpolate zoom in log space so the move feels even.
      const scale = Math.exp(Math.log(a.from.scale) + (Math.log(a.to.scale) - Math.log(a.from.scale)) * e)
      this.view = { cx: a.from.cx + (a.to.cx - a.from.cx) * e, cy: a.from.cy + (a.to.cy - a.from.cy) * e, scale }
      if (t >= 1) this.animation = null
      this.applyCamera()
    }
    if (!this.dirty) return
    this.dirty = false
    this.renderer.render(this.scene, this.camera)
    this.renders++
  }

  /**
   * Frame-rate probe: pans and zooms the camera every frame for `ms`, forcing
   * a full redraw of every visible node and edge each frame, and reports the
   * frames actually presented per second.
   */
  bench(ms = 5000): Promise<{ fps: number; frames: number; ms: number; nodes: number; edges: number; backend: Backend }> {
    return new Promise((resolve) => {
      const start = { ...this.view }
      const t0 = performance.now()
      const r0 = this.renders
      const step = () => {
        const t = performance.now() - t0
        if (t >= ms) {
          const frames = this.renders - r0
          this.setView(start)
          resolve({
            fps: (frames * 1000) / t,
            frames,
            ms: t,
            ...this.counts(),
            backend: this.backend,
          })
          return
        }
        const ph = t / 1000
        this.setView({
          cx: start.cx + Math.sin(ph * 1.3) * 40 / start.scale,
          cy: start.cy + Math.cos(ph * 0.9) * 40 / start.scale,
          scale: start.scale * (1 + 0.25 * Math.sin(ph * 2)),
        })
        requestAnimationFrame(step)
      }
      requestAnimationFrame(step)
    })
  }

  dispose() {
    cancelAnimationFrame(this.raf)
    this.renderer.dispose()
  }

  /** Nodes and edges in the current draw. */
  counts() {
    return { nodes: this.sprite?.count ?? 0, edges: (this.lines?.geometry.drawRange.count ?? 0) / 2 }
  }

  get size() {
    return { width: this.width, height: this.height }
  }

  isVisible(i: number) {
    return this.visible[i] === 1
  }

  isEmphasised(i: number) {
    return !this.emphasis || this.emphasis[i] === 1 || i === this.selected
  }
}
