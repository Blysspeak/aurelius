// aurelius — light and glass. Design prototype, phase 1. Plain JS, no build step.
// Data: window.AU from snapshot.js (real snapshot of the owner's database, kept out of git).
(() => {
  'use strict';
  const AU = window.AU;
  if (!AU) {
    document.body.innerHTML = '<p style="padding:40px;max-width:60ch;color:#e8e6e1;font:15px/1.6 sans-serif">Нет snapshot.js рядом с index.html. Снимок держится вне git: репозиторий публичный, а в нём личные финансы и метки памяти. Собрать: <code>ui/design/tools/make-snapshot.sh</code>.</p>';
    return;
  }
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const params = new URLSearchParams(location.search);
  const REDUCE = matchMedia('(prefers-reduced-motion: reduce)').matches || params.has('reduce');
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const lerp = (a, b, t) => a + (b - a) * t;
  const sstep = (a, b, v) => { const t = clamp((v - a) / (b - a), 0, 1); return t * t * (3 - 2 * t); };
  const outExpo = (t) => (t >= 1 ? 1 : 1 - Math.pow(2, -10 * t));
  const inOut = (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);
  const quintic = (t) => t * t * t * (t * (t * 6 - 15) + 10);
  const hash = (i) => { const x = Math.sin(i * 127.1 + 311.7) * 43758.5453; return x - Math.floor(x); };
  const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const nf0 = new Intl.NumberFormat('ru-RU');
  const nf2 = new Intl.NumberFormat('ru-RU', { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  const usd = (minor) => nf2.format(minor / 100);
  const dmy = (iso) => { if (!iso) return '—'; const d = new Date(iso); return d.toLocaleDateString('ru-RU', { day: '2-digit', month: '2-digit', year: 'numeric' }) + ' ' + d.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' }); };
  const stripProj = (s) => String(s || '').replace(/^\[[^\]]+\]\s*/, '');

  // ------------------------------------------------------------------ canvases
  const glCanvas = $('#gl');
  let glass = REDUCE && params.has('nogl') ? null : window.Glass.init(glCanvas);
  if (params.has('nogl')) glass = null;
  const scene = glass ? document.createElement('canvas') : $('#fallback');
  if (!glass) document.body.classList.add('nogl');
  const ctx = scene.getContext('2d');
  let VW = 0, VH = 0, DPR = 1;
  function resize() {
    DPR = Math.min(window.devicePixelRatio || 1, 2); vignette = null;
    VW = innerWidth; VH = innerHeight;
    for (const c of [scene, glCanvas]) { c.width = Math.round(VW * DPR); c.height = Math.round(VH * DPR); }
    fitCamera(true);
  }

  // ------------------------------------------------------------------ the mark (from ui/design/logo-mark.svg)
  const MK = {
    c: [512, 511.5], R: 362,
    nodes: [[591.1, 319.1], [528.2, 393.1], [658.2, 393.1], [513.8, 497.6], [626.8, 497.7], [695.1, 497.5]],
    edges: [[0, 1], [0, 2], [1, 3], [1, 4], [2, 3], [2, 4], [2, 5]],
    sil: 'M4995 7648 c-206 -18 -570 -114 -655 -173 -20 -14 -44 -25 -53 -25 -10 0 -41 -12 -69 -26 -73 -37 -129 -36 -209 3 -88 43 -112 48 -133 27 -10 -10 -14 -23 -10 -33 55 -127 18 -215 -107 -256 -200 -66 -320 -180 -365 -349 -33 -122 5 -294 83 -377 58 -62 55 -82 -44 -377 -83 -246 -88 -284 -45 -368 49 -99 47 -106 -163 -563 -181 -396 -179 -388 -120 -440 35 -30 37 -31 129 -28 103 2 126 -7 169 -65 27 -36 26 -118 -2 -181 -26 -60 -27 -50 10 -92 42 -47 51 -93 33 -162 -19 -73 -22 -65 51 -136 86 -83 90 -124 24 -244 -40 -72 -40 -72 -25 -135 35 -151 77 -194 227 -233 141 -37 303 -1 709 160 185 74 201 78 253 76 38 -1 96 -57 106 -101 15 -71 0 -98 -113 -207 l-106 -101 0 -299 c0 -164 4 -304 9 -311 15 -25 86 -34 321 -39 570 -13 1071 125 1559 428 161 100 153 87 130 210 -116 623 -148 811 -142 841 3 18 36 87 73 153 189 339 234 423 310 567 90 175 110 196 203 227 152 51 218 202 151 351 -34 76 -36 126 -10 300 50 344 -44 740 -251 1055 -439 665 -1125 994 -1928 923z',
  };
  // one continuous stroke of light: outer ring, inner ring, profile, network
  const strokes = [];
  (function buildStrokes() {
    const ring = (r) => { const p = []; for (let i = 0; i <= 220; i++) { const a = -Math.PI / 2 + (i / 220) * Math.PI * 2; p.push([MK.c[0] + Math.cos(a) * r, MK.c[1] + Math.sin(a) * r]); } return p; };
    strokes.push(ring(338), ring(308));
    const path = $('#sil-path'); path.setAttribute('d', MK.sil);
    const L = path.getTotalLength(); const sil = [];
    for (let s = 0; s <= L; s += 18) { const q = path.getPointAtLength(s); sil.push([q.x * 0.1, 1024 - q.y * 0.1]); }
    sil.push(sil[0]);
    strokes.push(sil);
    for (const [a, b] of MK.edges) strokes.push([MK.nodes[a], MK.nodes[b]].map((p) => p.slice()));
    for (const n of MK.nodes) { const p = []; for (let i = 0; i <= 24; i++) { const a = (i / 24) * Math.PI * 2; p.push([n[0] + Math.cos(a) * 16, n[1] + Math.sin(a) * 16]); } strokes.push(p); }
    let acc = 0;
    for (const s of strokes) { s.len = [acc]; for (let i = 1; i < s.length; i++) { acc += Math.hypot(s[i][0] - s[i - 1][0], s[i][1] - s[i - 1][1]); s.len.push(acc); } s.end = acc; }
    strokes.total = acc;
  })();
  function markPoint(u) { // point at fraction u of the whole stroke, mark coords
    const L = u * strokes.total;
    for (const s of strokes) { if (L <= s.end) { let i = 1; while (i < s.length - 1 && s.len[i] < L) i++; const t = clamp((L - s.len[i - 1]) / Math.max(s.len[i] - s.len[i - 1], 1e-6), 0, 1); return [lerp(s[i - 1][0], s[i][0], t), lerp(s[i - 1][1], s[i][1], t)]; } }
    const s = strokes[strokes.length - 1]; return s[s.length - 1];
  }
  const markXf = () => { const r = Math.min(VW, VH) * 0.2; return { cx: VW / 2, cy: VH / 2 - 20, s: r / 338 }; };

  // signed distance field of the extruded mark for the glass raymarch
  if (glass) (function markSDF() {
    const N = 1024, cv = document.createElement('canvas'); cv.width = cv.height = N;
    const c = cv.getContext('2d'); const k = N / (2 * MK.R);
    c.setTransform(k, 0, 0, k, N / 2 - MK.c[0] * k, N / 2 - MK.c[1] * k);
    c.strokeStyle = '#fff'; c.fillStyle = '#fff'; c.lineCap = 'round'; c.lineJoin = 'round';
    for (const [r, w] of [[338, 15], [308, 9]]) { c.lineWidth = w; c.beginPath(); c.arc(MK.c[0], MK.c[1], r, 0, Math.PI * 2); c.stroke(); }
    c.save(); c.translate(0, 1024); c.scale(0.1, -0.1); c.lineWidth = 130; c.stroke(new Path2D(MK.sil)); c.restore();
    c.lineWidth = 10; for (const [a, b] of MK.edges) { c.beginPath(); c.moveTo(...MK.nodes[a]); c.lineTo(...MK.nodes[b]); c.stroke(); }
    for (const n of MK.nodes) { c.beginPath(); c.arc(n[0], n[1], 21, 0, Math.PI * 2); c.fill(); }
    const px = c.getImageData(0, 0, N, N).data;
    const INF = 1e20, fin = new Float64Array(N * N), fout = new Float64Array(N * N);
    for (let i = 0; i < N * N; i++) { const inside = px[i * 4 + 3] > 127; fin[i] = inside ? INF : 0; fout[i] = inside ? 0 : INF; }
    const edt = (g) => { // Felzenszwalb squared EDT, rows then columns
      const f = new Float64Array(N), d = new Float64Array(N), v = new Int32Array(N), z = new Float64Array(N + 1);
      const pass = (get, set) => { for (let q = 0; q < N; q++) f[q] = get(q); let k2 = 0; v[0] = 0; z[0] = -INF; z[1] = INF; for (let q = 1; q < N; q++) { let s; do { const r = v[k2]; s = (f[q] + q * q - (f[r] + r * r)) / (2 * q - 2 * r); if (s <= z[k2]) k2--; else break; } while (k2 >= 0); k2++; v[k2] = q; z[k2] = s; z[k2 + 1] = INF; } k2 = 0; for (let q = 0; q < N; q++) { while (z[k2 + 1] < q) k2++; const r = v[k2]; d[q] = (q - r) * (q - r) + f[r]; } for (let q = 0; q < N; q++) set(q, d[q]); };
      for (let y = 0; y < N; y++) pass((x) => g[y * N + x], (x, val) => { g[y * N + x] = val; });
      for (let x = 0; x < N; x++) pass((y) => g[y * N + x], (y, val) => { g[y * N + x] = val; });
    };
    edt(fin); edt(fout);
    const out = new Float32Array(N * N);
    for (let i = 0; i < N * N; i++) out[i] = (fout[i] > 0 ? Math.sqrt(fout[i]) - 0.5 : -(Math.sqrt(fin[i]) - 0.5)) * (2 / N);
    // two separable box passes (radius 3 texels) remove the stair-steps of the thresholded mask
    const tmpB = new Float32Array(N * N);
    for (let pass = 0; pass < 2; pass++) {
      for (let y = 0; y < N; y++) for (let x = 0; x < N; x++) { let a = 0, c2 = 0; for (let k = -3; k <= 3; k++) { const xx = x + k; if (xx >= 0 && xx < N) { a += out[y * N + xx]; c2++; } } tmpB[y * N + x] = a / c2; }
      for (let y = 0; y < N; y++) for (let x = 0; x < N; x++) { let a = 0, c2 = 0; for (let k = -3; k <= 3; k++) { const yy = y + k; if (yy >= 0 && yy < N) { a += tmpB[yy * N + x]; c2++; } } out[y * N + x] = a / c2; }
    }
    glass.setMark(out, N);
  })();

  // ------------------------------------------------------------------ graph model
  const GA = AU.graph;
  const T = GA.types;
  const NN = GA.nodes.length;
  const nodes = GA.nodes.map((n, i) => ({ nd: 0, i, t: T[n[0]], c: n[1], label: n[2], age: n[3], st: n[4] || null, x: 0, y: 0, sx: VW / 2, sy: VH / 2, a: 0, bx: 0, by: 0, ba: 0, deg: 0, z: clamp(n[3] / 420, 0, 1) }));
  const links = [];
  const adj = nodes.map(() => []);
  for (let k = 0; k < GA.edges.length; k += 3) {
    const a = GA.edges[k], b = GA.edges[k + 1], r = GA.rels[GA.edges[k + 2]];
    links.push({ source: nodes[a], target: nodes[b], rel: r, li: links.length, bend: (hash(k * 0.37 + 11) - 0.5) * 0.42 });
    adj[a].push([b, r, 'out']); adj[b].push([a, r, 'in']);
    nodes[a].deg++; nodes[b].deg++;
    if (r !== 'belongs_to') { nodes[a].nd = (nodes[a].nd || 0) + 1; nodes[b].nd = (nodes[b].nd || 0) + 1; }
  }
  const projHub = new Map(GA.projects.map(([i, label, n, last, act, open, blk]) => [i, { label, n, last, act, open, blk }]));
  // bioluminescence by meaning: five groups, shape still names the exact type
  const GROUP = { decision: 'k', concept: 'k', solution: 'k', doc: 'k', skill: 'k', digest: 'k', task: 'w', work_log: 'w', session: 'w', reminder: 'w', problem: 'p', user_fact: 'o', person: 'o', file: 'i', dependency: 'i', server: 'i', crate: 'i', project: 'i', module: 'i', config: 'i' };
  const GHEX = { k: '#5CE1E6', w: '#FFB547', p: '#FF5C7A', o: '#7CF0A9', i: '#8FA3B8' };
  const GNAME = { k: 'знания', w: 'работа', p: 'проблемы', o: 'факты владельца', i: 'инфраструктура' };
  const rgb = (h) => [parseInt(h.slice(1, 3), 16) / 255, parseInt(h.slice(3, 5), 16) / 255, parseInt(h.slice(5, 7), 16) / 255];
  const GC = Object.fromEntries(Object.entries(GHEX).map(([k, v]) => [k, rgb(v)]));
  const SHAPE_ID = { concept: 0, decision: 1, problem: 2, task: 3, solution: 4, work_log: 5, session: 6, user_fact: 7, person: 7, project: 8 };
  nodes.forEach((n) => { n.g = GROUP[n.t] || 'i'; n.shape = SHAPE_ID[n.t] ?? 9; n.open = GA.nodes[n.i][5] === 1; n.attn = (n.t === 'task' && n.st === 'active') || n.open || (n.t === 'task' && n.st === 'blocked'); });
  const typeOn = new Set(T);

  // sprites: type = glyph shape, confidence = light quality, never hue
  const BASE_R = { project: 5.2, task: 3.3, decision: 2.8, problem: 2.9, solution: 2.6, concept: 2.6, session: 3.1, user_fact: 3, work_log: 1.7 };
  const glyphKind = (t) => ({ decision: 'diamond', problem: 'triangle', concept: 'circle', task: 'square', solution: 'plus', work_log: 'dot', session: 'ring', user_fact: 'star', project: 'double' }[t] || 'bar');
  function drawGlyph(c, kind, x, y, r, alpha, hollow) {
    c.fillStyle = c.strokeStyle = `rgba(232,230,225,${alpha})`;
    c.lineWidth = Math.max(1, r * 0.42);
    c.beginPath();
    switch (kind) {
      case 'diamond': c.moveTo(x, y - r * 1.25); c.lineTo(x + r, y); c.lineTo(x, y + r * 1.25); c.lineTo(x - r, y); c.closePath(); break;
      case 'triangle': c.moveTo(x, y - r * 1.15); c.lineTo(x + r * 1.1, y + r * 0.8); c.lineTo(x - r * 1.1, y + r * 0.8); c.closePath(); break;
      case 'square': c.rect(x - r * 0.9, y - r * 0.9, r * 1.8, r * 1.8); break;
      case 'plus': c.rect(x - r * 1.1, y - r * 0.28, r * 2.2, r * 0.56); c.rect(x - r * 0.28, y - r * 1.1, r * 0.56, r * 2.2); break;
      case 'dot': c.arc(x, y, r * 0.75, 0, 7); break;
      case 'ring': c.arc(x, y, r * 0.95, 0, 7); c.stroke(); c.beginPath(); c.arc(x, y, r * 0.3, 0, 7); break;
      case 'star': for (let i = 0; i < 8; i++) { const a = (i / 8) * Math.PI * 2 - Math.PI / 2, rr = i % 2 ? r * 0.42 : r * 1.2; c.lineTo(x + Math.cos(a) * rr, y + Math.sin(a) * rr); } c.closePath(); break;
      case 'double': c.arc(x, y, r, 0, 7); c.stroke(); c.beginPath(); c.arc(x, y, r * 0.55, 0, 7); c.stroke(); c.beginPath(); c.arc(x, y, r * 0.2, 0, 7); break;
      case 'bar': c.rect(x - r * 1.1, y - r * 0.3, r * 2.2, r * 0.6); break;
      default: c.arc(x, y, r, 0, 7);
    }
    if (hollow) c.stroke(); else c.fill();
  }
  const CONF_LOOK = [ // [glyph alpha, halo radius x, halo alpha, hollow]
    [0.36, 0, 0, false],   // none
    [1.0, 2.6, 0.42, false], // measured: sharp bright point
    [0.62, 5.0, 0.20, false], // inferred: soft halo
    [0.42, 0, 0, false],   // reported: dim point
    [0.5, 0, 0, true],     // unverified / stale: hollow ash ring
  ];
  const spriteCache = new Map();
  function sprite(t, conf) {
    const key = t + conf;
    if (spriteCache.has(key)) return spriteCache.get(key);
    const r = (BASE_R[t] || 2.3) * 2 * 2; // drawn at 4x for crisp scaling
    const [ga, hr, ha, hol] = CONF_LOOK[conf] || CONF_LOOK[0];
    const S = Math.ceil(r * (hr ? hr * 2.2 : 3.2));
    const cv = document.createElement('canvas'); cv.width = cv.height = S;
    const c = cv.getContext('2d');
    if (hr) { const g = c.createRadialGradient(S / 2, S / 2, 0, S / 2, S / 2, r * hr); g.addColorStop(0, `rgba(232,230,225,${ha})`); g.addColorStop(1, 'rgba(232,230,225,0)'); c.fillStyle = g; c.fillRect(0, 0, S, S); }
    const kind = glyphKind(t);
    drawGlyph(c, hol ? 'ring' : kind, S / 2, S / 2, r, hol ? 0.55 : ga, hol || kind === 'ring' || kind === 'double');
    const o = { cv, S, r };
    spriteCache.set(key, o);
    return o;
  }
  function glyphCanvas(t, conf, css = 14, hex) {
    const cv = document.createElement('canvas'); cv.width = cv.height = css * 2;
    const c = cv.getContext('2d');
    const kind = glyphKind(t), r = css * 0.28;
    drawGlyph(c, conf === 4 ? 'ring' : kind, css, css, r * 2, CONF_LOOK[conf][0], conf === 4 || kind === 'ring' || kind === 'double');
    if (hex) { c.globalCompositeOperation = 'source-in'; c.fillStyle = hex; c.fillRect(0, 0, cv.width, cv.height); }
    return cv;
  }

  // ------------------------------------------------------------------ layout: ganglia (real d3 ticks drive the loader)
  // Each project is an organic cluster; inside it radius follows age (recent at the core).
  // belongs_to only decides membership and is never drawn.
  const hubOf = new Int32Array(NN).fill(-1);
  for (const l of links) {
    if (l.rel !== 'belongs_to') continue;
    if (projHub.has(l.target.i)) hubOf[l.source.i] = l.target.i; else if (projHub.has(l.source.i)) hubOf[l.target.i] = l.source.i;
  }
  for (const i of projHub.keys()) hubOf[i] = i;
  for (let pass = 0; pass < 3; pass++) for (const n of nodes) if (hubOf[n.i] < 0) { const nb = adj[n.i].find(([j, r]) => r !== 'belongs_to' && hubOf[j] >= 0); if (nb) hubOf[n.i] = hubOf[nb[0]]; }
  const OTHER = -2;
  for (const n of nodes) { if (hubOf[n.i] < 0) hubOf[n.i] = OTHER; n.hub = hubOf[n.i]; }
  const ganglia = new Map();
  for (const n of nodes) { if (!ganglia.has(n.hub)) ganglia.set(n.hub, { key: n.hub, members: [], x: 0, y: 0 }); ganglia.get(n.hub).members.push(n); }
  for (const G of ganglia.values()) {
    G.members.sort((a, b) => a.age - b.age);
    G.R = 6.4 * Math.sqrt(G.members.length) + 14; G.rr = G.R; G.f = 1;
    G.ph1 = hash(G.key * 1.3 + 7) * 6.28; G.ph2 = hash(G.key * 2.1 + 3) * 6.28;
    const cnt = { k: 0, w: 0, p: 0, o: 0, i: 0 }; G.members.forEach((m) => cnt[m.g]++); G.dom = Object.entries(cnt).sort((x, y) => y[1] - x[1])[0][0];
    G.label = G.key === OTHER ? 'вне проектов' : projHub.get(G.key).label;
    G.info = G.key === OTHER ? null : projHub.get(G.key);
  }
  // ---- chains of meaning: a directed tree that converges on one root (the owner).
  // relation -> orientation: 'fwd' = source hangs on target, 'rev' = target hangs on source (aurelius_core::Relation)
  const CHAIN = { contains: 'rev', subtask_of: 'fwd', implements: 'fwd', solves: 'fwd', blocks: 'fwd', caused_by: 'rev', refines: 'fwd', supersedes: 'rev', depends_on: 'rev', prerequisite: 'rev', next_step: 'fwd', verified_by: 'rev' };
  const CHAIN_ORDER = ['contains', 'subtask_of', 'implements', 'solves', 'blocks', 'caused_by', 'refines', 'supersedes', 'depends_on', 'prerequisite', 'next_step', 'verified_by'];
  const ROOT = -10;
  const parent = new Int32Array(NN).fill(-1), prel = new Array(NN).fill(null);
  const wouldCycle = (c, p) => { for (let q = p, k = 0; q >= 0 && k < 80; q = parent[q], k++) if (q === c) return true; return false; };
  for (const rel of CHAIN_ORDER) for (const l of links) {
    if (l.rel !== rel) continue;
    const c = CHAIN[rel] === 'fwd' ? l.source.i : l.target.i, pp = CHAIN[rel] === 'fwd' ? l.target.i : l.source.i;
    if (parent[c] !== -1 || nodes[c].t === 'project' || nodes[c].hub !== nodes[pp].hub || wouldCycle(c, pp)) continue;
    parent[c] = pp; prel[c] = rel;
  }
  const NEEDS_CHAIN = new Set(['problem', 'solution', 'work_log']); // these only make sense attached to something
  for (const n of nodes) {
    if (n.t === 'project') { parent[n.i] = ROOT; prel[n.i] = 'belongs_to'; continue; }
    if (parent[n.i] === -1 && n.hub >= 0 && !NEEDS_CHAIN.has(n.t)) { parent[n.i] = n.hub; prel[n.i] = 'belongs_to'; }
  }
  const kids = nodes.map(() => []);
  for (const n of nodes) if (parent[n.i] >= 0) kids[parent[n.i]].push(n.i);
  const depthOf = (i) => { let d = 0; for (let q = i; q !== ROOT && q >= 0 && d < 90; q = parent[q]) d++; return d; };
  const leaves = new Int32Array(NN);
  const countLeaves = (i) => { let s = kids[i].length ? 0 : 1; for (const k of kids[i]) s += countLeaves(k); leaves[i] = s; return s; };
  const reachesRoot = (i) => { let q = i; for (let k = 0; k < 90 && q >= 0; k++) q = parent[q]; return q === ROOT; };
  for (const n of nodes) { n.orphan = !reachesRoot(n.i); n.depth = n.orphan ? 99 : depthOf(n.i); }
  for (const i of projHub.keys()) countLeaves(i);
  for (const G of ganglia.values()) G.orphans = G.members.filter((m) => m.orphan).length;
  const isWork = (c) => c.t === 'work_log' || c.t === 'session' || prel[c.i] === 'contains' && c.t === 'work_log';
  // ---- layout: root in the middle, projects around it, each project a radial tree growing away from the root
  (function pack() {
    const D = [...ganglia.values()].sort((a, b) => b.R - a.R);
    D.forEach((G, k) => { const a = k * 2.39996, r = 160 + 50 * Math.sqrt(k); G.x = Math.cos(a) * r; G.y = Math.sin(a) * r; });
    const ps = d3.forceSimulation(D).force('c', d3.forceCollide((G) => G.R * 1.15 + 26).iterations(4))
      .force('r', d3.forceRadial((G) => 90 + G.R * 1.2, 0, 0).strength(0.08)).stop();
    for (let k = 0; k < 400; k++) ps.tick();
  })();
  function placeTree(i, a0, a1, rStep, G, d) {
    const n = nodes[i];
    const am = (a0 + a1) / 2;
    if (i !== G.key) { const r = d * rStep * (0.94 + 0.12 * hash(i * 2.7)); n.ax = G.x + Math.cos(am) * r; n.ay = G.y + Math.sin(am) * r; }
    const ch = kids[i].slice().sort((x, y) => nodes[x].age - nodes[y].age);
    const tot = ch.reduce((s, k) => s + leaves[k], 0) || 1;
    let a = a0;
    for (const k of ch) { const w = (a1 - a0) * leaves[k] / tot; placeTree(k, a, a + w, rStep, G, d + 1); a += w; }
  }
  for (const G of ganglia.values()) {
    const hub = G.key >= 0 ? nodes[G.key] : null;
    if (hub) { hub.ax = G.x; hub.ay = G.y; }
    const out = Math.atan2(G.y, G.x);                 // grow away from the root, converge toward it
    if (hub) {
      const maxD = Math.max(2, ...G.members.filter((m) => !m.orphan).map((m) => m.depth - 1));
      placeTree(G.key, out - Math.PI * 0.82, out + Math.PI * 0.82, G.R / maxD, G, 0);
    }
    const orph = G.members.filter((m) => m.orphan || !hub);
    orph.forEach((n, k) => { const th = out + Math.PI + (k / Math.max(orph.length, 1) - 0.5) * Math.PI * 1.9; const r = G.R * (hub ? 1.12 : 0.6 * Math.sqrt((k + 0.5) / orph.length)) + (hash(n.i) - 0.5) * 6; n.ax = G.x + Math.cos(th) * r; n.ay = G.y + Math.sin(th) * r; });
  }
  // time: birth in days from now (<= 0); a superseded fact dims when its successor is born
  for (const n of nodes) { n.birth = -n.age; n.supAt = 1e9; }
  for (const l of links) if (l.rel === 'supersedes') l.target.supAt = Math.min(l.target.supAt, l.source.birth);
  // root as a parameter: 'now' (radius = log time from now, project = angular sector) or 'owner' (radial trees)
  const ROOT_MODE = params.get('root') === 'owner' ? 'owner' : 'now';
  const AGE_MAX = Math.max(...nodes.map((n) => n.age)) + 0.5, LOGA = Math.log1p(AGE_MAX);
  if (ROOT_MODE === 'now') {
    const angTree = (i, a0, a1) => { nodes[i].ang = (a0 + a1) / 2; const ch = kids[i].slice().sort((x, y) => nodes[x].age - nodes[y].age); const tot = ch.reduce((q, k) => q + leaves[k], 0) || 1; let a = a0; for (const k of ch) { const w = (a1 - a0) * leaves[k] / tot; angTree(k, a, a + w); a += w; } };
    const order = [...ganglia.values()].sort((a, b) => b.members.length - a.members.length);
    const tot = order.reduce((q, G) => q + Math.pow(G.members.length, 0.8), 0);
    let a = -Math.PI / 2;
    const RMIN = 60, RMAX = 560;
    const ageRank = new Map(nodes.slice().sort((x, y) => x.age - y.age).map((n, k) => [n.i, k / (NN - 1)])); // monotone in time from now, evens out density
    for (const G of order) {
      const span = (Math.PI * 2 * Math.pow(G.members.length, 0.8)) / tot, gap = Math.min(0.05, span * 0.12);
      G.a0 = a + gap; G.a1 = a + span - gap; a += span;
      if (G.key >= 0) angTree(G.key, G.a0, G.a1);
      for (const n of G.members) {
        if (n.ang == null || n.orphan) n.ang = G.a0 + hash(n.i * 1.9) * (G.a1 - G.a0);
        const r = n.t === 'project' ? RMIN * 0.7 : RMIN + (RMAX - RMIN) * Math.sqrt(ageRank.get(n.i)) * (0.93 + 0.14 * hash(n.i * 4.1));
        n.ax = Math.cos(n.ang) * r; n.ay = Math.sin(n.ang) * r;
      }
      const am = (G.a0 + G.a1) / 2; G.x = Math.cos(am) * RMAX * 0.4; G.y = Math.sin(am) * RMAX * 0.4;
      G.lx = Math.cos(am) * (RMAX + 34); G.ly = Math.sin(am) * (RMAX + 34); // label outside the rim of its sector
    }
  }
  const treeEdges = [];
  for (const n of nodes) if (parent[n.i] >= 0) treeEdges.push({ c: n, p: nodes[parent[n.i]], rel: prel[n.i], work: isWork(n), li: treeEdges.length, imp: Math.log2(1 + leaves[n.i]) + (prel[n.i] === 'solves' || prel[n.i] === 'implements' ? 1.5 : 0) + (n.age < 30 ? 0.6 : 0) });
  const rootN = { i: ROOT, sx: 0, sy: 0, a: 1, g: 'i', hub: ROOT, nd: 99, t: 'root', x: 0, y: 0 };
  function chainOf(i) { // upstream main line (deepest branch), the node, downstream to the root
    const down = []; for (let q = parent[i], k = 0; q >= 0 && k < 12; q = parent[q], k++) down.push(q);
    const up = []; let cur = i;
    for (let k = 0; k < 6 && kids[cur].length; k++) { cur = kids[cur].slice().sort((x, y) => leaves[y] - leaves[x] || nodes[x].age - nodes[y].age)[0]; up.push(cur); }
    const stubs = [];
    for (const m of [i, ...up]) for (const k of kids[m].filter((x) => !up.includes(x)).slice(0, 2)) stubs.push([k, m]);
    return { up: up.reverse(), node: i, down, stubs, toRoot: parent[down.length ? down[down.length - 1] : i] === ROOT || nodes[i].t === 'project' };
  }

  const nbLinks = [];
  for (const l of links) if (l.rel !== 'belongs_to') nbLinks.push({ source: l.source.i, target: l.target.i });
  let sim, tick = 0;
  const TICKS = 300;
  function anchorForce(alpha) { for (const n of nodes) { n.vx += (n.ax - n.x) * 0.3 * alpha; n.vy += (n.ay - n.y) * 0.3 * alpha; } }
  function newSim() {
    nodes.forEach((n) => { n.x = n.ax + (hash(n.i) - 0.5) * 6; n.y = n.ay + (hash(n.i + 5) - 0.5) * 6; n.vx = n.vy = 0; });
    tick = 0;
    sim = d3.forceSimulation(nodes)
      .force('charge', d3.forceManyBody().strength(-1.2).theta(0.9).distanceMax(14))
      .force('anchor', anchorForce)
      .stop();
  }
  function stepSim(n) { for (let k = 0; k < n && tick < TICKS; k++) { sim.tick(); tick++; } }

  // camera
  const cam = { x: 0, y: 0, k: 1, ox: 0, oy: 0 };
  let camFly = null;
  function bounds() {
    const xs = nodes.map((n) => n.x).sort((a, b) => a - b), ys = nodes.map((n) => n.y).sort((a, b) => a - b);
    const q = (arr, p) => arr[Math.floor(p * (arr.length - 1))];
    return { x0: q(xs, 0.015), x1: q(xs, 0.985), y0: q(ys, 0.015), y1: q(ys, 0.985) };
  }
  function fitTarget() {
    const b = bounds(); const railW = 132;
    const detailW = state.sel != null && state.screen === 'memory' ? 440 : 0;
    const aw = VW - railW - detailW - 80, ah = VH - 150;
    const k = Math.min(aw / Math.max(b.x1 - b.x0, 1e-3), ah / Math.max(b.y1 - b.y0, 1e-3)) * 0.95;
    return { x: (b.x0 + b.x1) / 2, y: (b.y0 + b.y1) / 2, k: isFinite(k) && k > 0 ? Math.min(k, 40) : 1 };
  }
  function fitCamera(instant) { if (!nodes.length) return; const t = fitTarget(); if (state.sel == null) fitK = t.k; if (instant) Object.assign(cam, t); else fly(t); }
  function fly(t, dur = 640) { camFly = { from: { x: cam.x, y: cam.y, k: cam.k }, to: t, t0: performance.now(), dur: REDUCE ? 1 : dur }; }
  function originX() { const railW = 132; const detailW = state.sel != null && state.screen === 'memory' ? 440 : 0; return railW + (VW - railW - detailW) / 2; }
  function project(n) {
    return [cam.ox + (n.x - cam.x) * cam.k, cam.oy + (n.y - cam.y) * cam.k];
  }

  // ------------------------------------------------------------------ state
  const state = { hoverProj: null, screen: 'memory', prev: null, morph0: 0, sel: null, hover: null, selT: 0, chain: null, loader: null, replay: { s: 1, playing: false }, filterType: null, fin: { hover: -1, a: 0, b: AU.fin.months.length - 1, kind: -1, cat: -1, q: '', voided: false }, task: 'b7a7d9af' };

  // ------------------------------------------------------------------ finance model
  const F = AU.fin;
  const months = F.months.map((m) => m.key);
  const ENT = F.entries.map((e) => ({ id: e[0], date: e[1], m: months.indexOf(e[1].slice(0, 7)), kind: e[2], amount: e[3], cur: e[4], camt: e[5], ccur: e[6], cat: e[7], note: e[8], usd: e[9], src: e[10], units: e[11], void: e[12], reason: e[13], sx: 0, sy: 0, a: 0, fx: 0, fy: 0, fa: 0, h: 2, fh: 2, tx: 0, ty: 0, th: 2, ta: 0 }));
  // stacked segments: within a month, income stacks up from the axis and expense down, oldest first
  (function stack() {
    const acc = months.map(() => [0, 0]);
    const order = ENT.slice().sort((a, b) => (a.date < b.date ? -1 : 1));
    for (const e of order) { if (e.void || e.kind === 2 || e.m < 0) { e.base = 0; continue; } e.base = acc[e.m][e.kind]; acc[e.m][e.kind] += e.usd; }
    F.maxMonth = Math.max(...acc.map((x) => Math.max(x[0], x[1])));
  })();
  function finTotals() {
    const { a, b } = state.fin; let exp = 0, inc = 0, n = 0;
    for (const e of ENT) if (!e.void && e.m >= a && e.m <= b && e.kind < 2) { if (e.kind === 0) exp += e.usd; else inc += e.usd; n++; }
    return { exp, inc, n };
  }

  // ------------------------------------------------------------------ DOM: memory
  function buildLegend() {
    const L = $('#m-legend');
    const counts = Object.fromEntries(GA.full.types);
    const order = ['decision', 'problem', 'solution', 'concept', 'task', 'session', 'work_log', 'user_fact', 'project', 'doc', 'skill', 'file'];
    L.innerHTML = '<span class="cap h">тип — форма</span>';
    for (const t of order) {
      const b = document.createElement('button');
      b.dataset.t = t;
      b.append(glyphCanvas(t, 1));
      const cnt = t === 'doc' ? (counts.doc || 0) + (counts['{"custom": "document"}'] || 0) : counts[t] || 0;
      b.insertAdjacentHTML('beforeend', `<span>${t.replace('_', ' ')} <span class="mono">${nf0.format(cnt)}</span></span>`);
      b.title = 'Показать только этот тип';
      b.onclick = () => { state.filterType = state.filterType === t ? null : t; $$('#m-legend button').forEach((x) => x.classList.toggle('off', state.filterType && x.dataset.t !== state.filterType)); };
      L.append(b);
    }
    L.insertAdjacentHTML('beforeend', '<span class="cap h">уверенность — свет</span><div id="m-conf"></div>');
    const C = $('#m-conf');
    const cc = Object.fromEntries(GA.full.conf);
    for (const [ci, name, key] of [[1, 'measured', 'measured'], [2, 'inferred', 'inferred'], [3, 'reported', 'reported'], [4, 'stale / unverified', 'unverified']]) {
      const s = document.createElement('span');
      const cv = document.createElement('canvas'); cv.width = cv.height = 28; cv.style.width = cv.style.height = '14px';
      const sp = sprite('concept', ci); cv.getContext('2d').drawImage(sp.cv, 0, 0, 28, 28);
      s.append(cv); s.insertAdjacentHTML('beforeend', `${name} <span class="mono">${nf0.format(cc[key] || 0)}</span>`);
      C.append(s);
    }
  }
  function statsLine() {
    $('#m-stats').innerHTML = `<b>${nf0.format(GA.full.nodes)}</b> узлов · <b>${nf0.format(GA.full.edges)}</b> связей<br>в прототипе ${nf0.format(NN)} узлов, ${nf0.format(links.length)} связей<br>облако = проект · ядро = свежее · цвет = смысл` + (params.has('fps') ? '<br><span id="fps"></span>' : '');
  }
  function showNode(i) {
    const D = $('#m-detail');
    if (i == null) { D.classList.remove('on'); return; }
    const n = nodes[i], d = GA.details[i];
    const sp = glyphCanvas(n.t, n.c, 16);
    const confName = GA.conf[n.c] || 'без оценки';
    const nb = adj[i].slice().sort((a, b) => nodes[b[0]].deg - nodes[a[0]].deg);
    const groups = {};
    for (const [j, r, dir] of (d ? d.links.map(([r, j, dir]) => [j, r, dir]) : nb)) (groups[r] ||= []).push([j, dir]);
    let links = '';
    for (const [r, arr] of Object.entries(groups)) for (const [j, dir] of arr.slice(0, 8)) links += `<button data-n="${j}"><span class="r">${dir === 'in' ? '← ' : '→ '}${esc(r)}</span><span class="l">${esc(stripProj(nodes[j].label))}</span></button>`;
    D.innerHTML = `<button class="close" data-close>Esc</button>
      <div class="kind cap"></div>
      <h2>${esc(stripProj(d ? d.label : n.label))}</h2>
      ${d && d.claim ? `<p class="claim">${esc(d.claim)}</p>` : ''}
      ${d && d.note ? `<p class="${d.claim ? 'note' : 'claim'}">${esc(d.note)}</p>` : ''}
      <div class="sec"><span class="cap">происхождение</span><dl class="prov">
        <dt>уверенность</dt><dd>${esc(confName)}</dd>
        ${d && d.subject ? `<dt>тема</dt><dd class="mono">${esc(d.subject)}</dd>` : ''}
        ${d && d.volatility ? `<dt>изменчивость</dt><dd>${esc(d.volatility)}</dd>` : ''}
        <dt>создан</dt><dd class="mono">${d ? dmy(d.created_at) : n.age + ' дн. назад'}</dd>
        ${d && d.measured_at ? `<dt>замерен</dt><dd class="mono">${dmy(d.measured_at)}</dd>` : ''}
        ${d && d.source ? `<dt>источник</dt><dd class="mono">${esc(d.source)}</dd>` : ''}
      </dl></div>
      ${d && d.evidence ? `<div class="sec"><span class="cap">улики</span><div class="evidence">${esc(d.evidence)}</div></div>` : ''}
      ${!d ? '<div class="sec"><p class="note">Заметка, claim и улики этого узла в снимок не попали; в фазе 2 их отдаёт /api/node/:id.</p></div>' : ''}
      ${chainSection(i)}
      <div class="sec"><span class="cap">связи · ${nf0.format(adj[i].length)}</span><div class="links">${links || '<p class="note">нет связей в выборке</p>'}</div></div>`;
    D.querySelector('.kind').insertAdjacentHTML('beforeend', (ico(TYPE_ICON[n.t], 'kic', 'color:' + GHEX[n.g]) || '') + esc(n.t.replace('_', ' ')) + `<span class="cfm">${ico(['', 'cf-measured', 'cf-inferred', 'cf-reported', 'cf-stale'][n.c] || 'cf-stale')}</span>`);
    D.dataset.tint = GHEX[n.g];
    D.classList.add('on');
    D.scrollTop = 0;
  }
  function chainSection(i) {
    const c = chainOf(i);
    const seq = [...c.up, i, ...c.down];
    if (seq.length < 2) return `<div class="sec"><span class="cap">цепочка</span><p class="note">${nodes[i].orphan ? 'Узел не связан с корнем цепочками смысла: висит без задачи или проекта.' : 'Цепочка из одного шага.'}</p></div>`;
    return `<div class="sec"><span class="cap">цепочка · ${seq.length}${c.toRoot ? ' → корень' : ''}</span><ol class="chainlist">${seq.map((k) => { const n = nodes[k]; return `<li class="${k === i ? 'me' : ''}"><button data-n="${k}">${ico(TYPE_ICON[n.t], '', 'color:' + GHEX[n.g])}<span class="ct">${esc(n.t.replace('_', ' '))}</span><span class="cl">${esc(stripProj(n.label))}</span><span class="cd num">${n.age < 1 ? 'сегодня' : Math.round(n.age) + ' дн.'}</span></button></li>`; }).join('')}</ol></div>`;
  }
  $('#m-detail').addEventListener('click', (e) => {
    const b = e.target.closest('[data-n]'); if (b) select(+b.dataset.n, true);
    if (e.target.closest('[data-close]')) select(null);
  });
  function select(i) {
    state.sel = i; state.selT = performance.now();
    showNode(i);
    if (i != null) enterChain(i); else { exitChain(); fitCamera(false); }
  }
  // ---- chain focus: upstream to the left, the node in the centre, downstream on to the root at the right
  function enterChain(i) {
    if (i == null || i < 0) return;
    const c = chainOf(i);
    const seq = [...c.up.map((k) => nodes[k]), nodes[i], ...c.down.map((k) => nodes[k])];
    if (c.toRoot) seq.push(rootN);
    for (const n of nodes) { n.bx = n.sx; n.by = n.sy; n.ba = n.a; }
    rootN.bx = rootN.sx; rootN.by = rootN.sy;
    state.morph0 = performance.now();
    state.chain = { i, seq, stubs: c.stubs, pos: new Map(), lastRun: 0 };
    layoutChain(); buildChainLabels();
    if (state.sel !== i) { state.sel = i; showNode(i); }
  }
  function layoutChain() {
    const ch = state.chain; if (!ch) return;
    const seq = ch.seq, k0 = seq.findIndex((n) => n.i === ch.i);
    const left = 132 + 70, right = VW - (state.sel != null ? 460 : 40) - 70;
    const ox = Math.min(Math.max((left + right) / 2, left + k0 * 60), right - (seq.length - 1 - k0) * 60);
    const gap = Math.min(150, (ox - left) / Math.max(k0, 1), (right - ox) / Math.max(seq.length - 1 - k0, 1));
    ch.pos = new Map();
    seq.forEach((n, k) => ch.pos.set(n.i, [ox + (k - k0) * gap, VH * 0.46]));
    ch.stubs.forEach(([c, m], q) => { const pm = ch.pos.get(m); if (pm && !ch.pos.has(c)) ch.pos.set(c, [pm[0] - gap * 0.28, pm[1] + (q % 2 ? -1 : 1) * (78 + 22 * (q % 3))]); });
  }
  function exitChain(instant) {
    if (!state.chain) return;
    for (const n of nodes) { n.bx = n.sx; n.by = n.sy; n.ba = n.a; }
    state.morph0 = instant ? 0 : performance.now();
    state.chain = null; $('#m-chain').innerHTML = '';
  }
  function buildChainLabels() {
    const ch = state.chain, box = $('#m-chain'); box.innerHTML = '';
    ch.labels = ch.seq.map((n, k) => {
      const el = document.createElement('button'); el.className = 'cstep' + (n.i === ch.i ? ' me' : '');
      el.innerHTML = n === rootN ? `<img class="inv" src="logo-mark.svg" alt=""><span class="cl">${ROOT_MODE === 'now' ? 'сейчас' : 'владелец'}</span>` : `${ico(TYPE_ICON[n.t], '', 'color:' + GHEX[n.g])}<span class="ct">${esc(n.t.replace('_', ' '))}</span><span class="cl">${esc(stripProj(n.label))}</span>`;
      if (n !== rootN) el.onclick = () => select(n.i);
      el.style.setProperty('--d', (k * 60) + 'ms');
      box.append(el); return [el, n, k];
    });
  }
  function updateChainLabels() {
    const ch = state.chain; if (!ch || !ch.labels) return;
    const off = $('#s-memory').getBoundingClientRect().left;
    for (const [el, n, k] of ch.labels) { const x = n.sx - off, y = n.sy; el.style.transform = `translate(${(x - 70).toFixed(1)}px, ${(y + (k % 2 ? -78 : 18)).toFixed(1)}px)`; el.classList.toggle('up', k % 2 === 1); }
  }

  // ------------------------------------------------------------------ DOM: tasks
  const TK = AU.tasks;
  const prByBranch = Object.fromEntries((TK.prs || []).map((p) => [p.headRefName, p]));
  function taskRows() {
    const list = TK.list.slice();
    const have = new Set(list.map((t) => t.id));
    for (const d of Object.values(TK.detail)) if (!have.has(d.id)) list.push({ id: d.id, label: d.label, status: d.data.status, priority: d.data.priority, branch: d.data.git && d.data.git.branch, worktree: d.data.git && d.data.git.worktree, criteria_total: (d.data.acceptance_criteria || []).length, criteria_met: 0, branch_state: null });
    return list;
  }
  const LANES = [['backlog', 'бэклог'], ['active', 'в работе'], ['blocked', 'блок'], ['done', 'готово']];
  const PRIO = { critical: 4, high: 3, medium: 2, low: 1 };
  function buildLanes() {
    const rows = taskRows();
    const el = $('#lanes'); el.innerHTML = '';
    LANES.forEach(([st, name], li) => {
      const items = rows.filter((t) => (st === 'done' ? t.status === 'done' || t.status === 'cancelled' : t.status === st)).sort((a, b) => (PRIO[b.priority] || 0) - (PRIO[a.priority] || 0));
      const lane = document.createElement('div');
      lane.className = 'lane stg ' + st; lane.style.setProperty('--i', li);
      lane.innerHTML = `<h3><span class="cap lh">${ico('st-' + st)}${name}</span><span class="n">${items.length}</span></h3>` + (items.length ? '' : '<div class="empty">пусто — ничего не заблокировано</div>');
      for (const t of items) {
        const g = TK.gidx[t.id];
        const p = PRIO[t.priority] || 0;
        const b = document.createElement('button');
        b.className = 'trow'; b.dataset.id = t.id;
        b.innerHTML = `<span class="pt" ${g != null ? `data-g="${g}"` : ''}></span><span class="tt">${esc(stripProj(t.label))}</span>
          <span class="tm"><span class="prio" title="${t.priority}">${[1, 2, 3, 4].map((k) => `<i class="${k <= p ? 'on' : ''}" style="height:${2 + k * 2}px"></i>`).join('')}</span>${t.criteria_total ? `<span>${t.criteria_met}/${t.criteria_total}</span>` : ''}${t.branch ? `<span>${esc(t.branch)}</span>` : ''}</span>`;
        if (g == null) b.querySelector('.pt').append(glyphCanvas('task', st === 'active' ? 1 : 3, 10, { active: '#FFB547', blocked: '#FF5C7A', done: '#7CF0A9' }[t.status] || '#8FA3B8'));
        lane.append(b);
      }
      el.append(lane);
    });
    el.onclick = (e) => { const b = e.target.closest('.trow'); if (b) { state.task = b.dataset.id; showTask(); } };
  }
  function gitRail(t, d) {
    const branch = (d && d.data.git && d.data.git.branch) || t.branch;
    const pr = branch ? prByBranch[branch] : null;
    const res = d && d.data.resolution;
    const text = d ? [...d.logs.map((l) => l.text), (res && res.commit) || ''].join(' ') : '';
    const commits = [...new Set((text.match(/\b(?=[0-9a-f]*[a-f])(?=[0-9a-f]*\d)[0-9a-f]{7,10}\b/g) || []))].slice(0, 7);
    if (res && res.commit && !commits.includes(res.commit)) commits.push(res.commit);
    const onMain = branch === 'main';
    const n = Math.max(commits.length, 1);
    const ticks = commits.map((c, k) => { const y = 120 - ((k + 1) / (n + 1)) * 86; return `<line x1="${onMain ? 8 : 44}" x2="${onMain ? 20 : 56}" y1="${y}" y2="${y}" stroke="#e8e6e1" stroke-width="1.5"/>`; }).join('');
    let br = '';
    if (!onMain && branch) {
      br = `<path d="M14 144 C14 128 50 132 50 118 L50 40" fill="none" stroke="#e8e6e1" stroke-opacity=".8" stroke-width="1.5"/>`;
      if (pr && pr.state === 'MERGED') br += `<path d="M50 40 C50 26 14 30 14 14" fill="none" stroke="#e8e6e1" stroke-width="1.5"/><circle cx="14" cy="14" r="4" fill="#e8e6e1"/>`;
      else if (pr && pr.state === 'OPEN') br += `<path d="M50 40 C50 30 34 28 26 22" fill="none" stroke="#e8e6e1" stroke-width="1.5" stroke-dasharray="3 3"/><circle cx="26" cy="22" r="2.5" fill="none" stroke="#e8e6e1"/>`;
      else br += `<line x1="50" x2="50" y1="40" y2="24" stroke="#e8e6e1" stroke-opacity=".5" stroke-dasharray="2 4"/>`;
    }
    const svg = `<svg viewBox="0 0 74 150" aria-hidden="true"><line x1="14" x2="14" y1="0" y2="150" stroke="#e8e6e1" stroke-opacity=".35" stroke-width="1.5"/>${br}${ticks}<text x="0" y="160" fill="#80858c" font-size="9" font-family="Geist Mono, monospace">main</text></svg>`;
    const prTxt = pr ? `#${pr.number} ${pr.state === 'OPEN' ? 'открыт' : pr.state === 'MERGED' ? 'влит' : 'закрыт'}` : res && res.pull_request ? res.pull_request.replace(/.*\/pull\//, '#') : '—';
    return `<div class="git">${svg}<dl>
      <dt>ветка</dt><dd>${esc(branch || 'не привязана')}</dd>
      <dt>состояние</dt><dd>${esc(t.branch_state || (onMain ? 'работа прямо в main' : pr ? (pr.state === 'MERGED' ? 'влита в main' : 'PR открыт, ждёт ревью') : branch ? 'ветка без PR' : '—'))}</dd>
      ${(d && d.data.git && d.data.git.worktree) || t.worktree ? `<dt>дерево</dt><dd>${esc(((d && d.data.git.worktree) || t.worktree).replace('/home/blyss/workSpace/project/', '~/…/'))}</dd>` : ''}
      <dt>PR</dt><dd>${esc(prTxt)}</dd>
      <dt>коммиты</dt><dd>${commits.length ? commits.map(esc).join(' ') : '—'}</dd>
    </dl></div>`;
  }
  function showTask() {
    $$('.trow').forEach((r) => r.classList.toggle('sel', r.dataset.id.startsWith(state.task)));
    const t = taskRows().find((x) => x.id.startsWith(state.task));
    if (!t) return;
    const d = Object.values(TK.detail).find((x) => x.id === t.id);
    const crit = (d && d.data.acceptance_criteria) || [];
    const met = t.criteria_met || 0;
    const st = { backlog: 'бэклог', active: 'в работе', blocked: 'блок', done: 'готово', cancelled: 'отменена' }[t.status] || t.status;
    $('#t-detail').innerHTML = `<div class="tline"><span class="chip st-${t.status}">${ico('st-' + t.status)}${st}</span><span>${esc(t.priority)}</span><span class="mono" style="letter-spacing:0">${t.id.slice(0, 8)}</span></div>
      <h2>${esc(stripProj(t.label))}</h2>
      ${d ? `<div class="dates"><div class="y"><span class="cap">заведена</span><span class="num">${dmy(d.created_at)}</span></div><div class="${d.data.activated_at ? 'y' : ''}"><span class="cap">взята</span><span class="num">${dmy(d.data.activated_at || d.data.started_at)}</span></div><div class="${d.data.closed_at ? 'y' : ''}"><span class="cap">закрыта</span><span class="num">${dmy(d.data.closed_at)}</span></div></div>` : ''}
      ${d && d.note ? `<div class="sec"><p class="note" style="color:var(--bone-2);line-height:1.6;margin-top:18px">${esc(d.note.slice(0, 420))}${d.note.length > 420 ? '…' : ''}</p></div>` : ''}
      <div class="sec" style="margin-top:22px"><span class="cap" style="display:block;margin-bottom:8px">git</span>${gitRail(t, d)}</div>
      ${crit.length ? `<div class="sec" style="margin-top:22px"><span class="cap" style="display:block;margin-bottom:6px">критерии · ${met}/${crit.length}</span><ul class="crit">${crit.map((c, k) => `<li class="${k < met ? 'met' : ''}">${esc(c)}</li>`).join('')}</ul></div>` : `<div class="sec" style="margin-top:22px"><p class="note" style="color:var(--bone-3)">Критериев ${t.criteria_total || 0}; полная карточка в фазе 2 из /api/tasks/:id.</p></div>`}
      ${d && d.subtasks.length ? `<div class="sec" style="margin-top:22px"><span class="cap" style="display:block;margin-bottom:6px">подзадачи</span><ul class="log">${d.subtasks.map((s) => `<li><span class="num">${esc(s.status || '')}</span><span>${esc(stripProj(s.label))}</span></li>`).join('')}</ul></div>` : ''}
      ${d && d.logs.length ? `<div class="sec" style="margin-top:22px"><span class="cap" style="display:block;margin-bottom:6px">журнал · ${d.logs.length}</span><ul class="log">${d.logs.slice(-6).reverse().map((l) => `<li><span class="num">${dmy(l.t).slice(0, 5)} ${dmy(l.t).slice(-5)}</span><span>${esc(l.text)}</span></li>`).join('')}</ul></div>` : ''}
      ${d && d.decisions.length ? `<div class="sec" style="margin-top:22px"><span class="cap" style="display:block;margin-bottom:6px">решения · ${d.decisions.length}</span><ul class="log">${d.decisions.slice(0, 5).map((l) => `<li><span class="num">◇</span><span>${esc(l.text)}</span></li>`).join('')}</ul></div>` : ''}`;
    $('#t-detail').scrollTop = 0;
  }

  // ------------------------------------------------------------------ DOM: finance
  const KIND = ['расход', 'доход', 'обмен'];
  function buildFinance() {
    const m = $('#f-scrub .months');
    m.style.gridTemplateColumns = `repeat(${months.length}, 1fr)`;
    const MN = ['янв', 'фев', 'мар', 'апр', 'май', 'июн', 'июл', 'авг', 'сен', 'окт', 'ноя', 'дек'];
    m.innerHTML = months.map((k) => `<span>${MN[+k.slice(5) - 1]}${k.slice(5) === '01' || k === months[0] ? ' ' + k.slice(2, 4) : ''}</span>`).join('');
    // scrubber drag
    const S = $('#f-scrub');
    let drag = null;
    const toIdx = (x) => { const r = S.getBoundingClientRect(); return clamp(Math.round(((x - r.left) / r.width) * months.length - 0.5), 0, months.length - 1); };
    S.addEventListener('pointerdown', (e) => {
      const h = e.target.closest('.h');
      const i = toIdx(e.clientX);
      drag = h ? h.dataset.h : Math.abs(i - state.fin.a) < Math.abs(i - state.fin.b) ? 'a' : 'b';
      S.setPointerCapture(e.pointerId); move(e);
    });
    const move = (e) => { if (!drag) return; const i = toIdx(e.clientX); if (drag === 'a') state.fin.a = Math.min(i, state.fin.b); else state.fin.b = Math.max(i, state.fin.a); renderFin(); $$('.h', S).forEach((h) => h.classList.toggle('drag', h.dataset.h === drag)); };
    S.addEventListener('pointermove', move);
    S.addEventListener('pointerup', () => { drag = null; $$('.h', S).forEach((h) => h.classList.remove('drag')); });
    $$('.h', S).forEach((h) => h.addEventListener('keydown', (e) => { const k = h.dataset.h; const d = e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowRight' ? 1 : 0; if (!d) return; if (k === 'a') state.fin.a = clamp(state.fin.a + d, 0, state.fin.b); else state.fin.b = clamp(state.fin.b + d, state.fin.a, months.length - 1); renderFin(); e.preventDefault(); }));
    // filters
    const cats = F.cats;
    $('#f-filters').innerHTML = `<div class="seg" role="group" aria-label="Вид">${['все', 'расход', 'доход', 'обмен'].map((n, k) => `<button data-k="${k - 1}" aria-pressed="${k === 0}">${n}</button>`).join('')}</div>
      <select id="f-cat" aria-label="Категория"><option value="-1">все категории</option>${cats.map((c, k) => `<option value="${k}">${esc(c)}</option>`).join('')}</select>
      <input id="f-q" placeholder="заметка, сумма, #id" aria-label="Поиск по записям">
      <label><input type="checkbox" id="f-void"> отменённые</label><span class="cnt" id="f-count"></span>`;
    $('#f-filters .seg').onclick = (e) => { const b = e.target.closest('button'); if (!b) return; state.fin.kind = +b.dataset.k; $$('#f-filters .seg button').forEach((x) => x.setAttribute('aria-pressed', x === b)); renderRows(); };
    $('#f-cat').onchange = (e) => { state.fin.cat = +e.target.value; renderRows(); };
    $('#f-q').oninput = (e) => { state.fin.q = e.target.value.trim().toLowerCase(); renderRows(); };
    $('#f-void').onchange = (e) => { state.fin.voided = e.target.checked; renderRows(); };
    renderFin();
  }
  function renderFin() {
    const { a, b } = state.fin;
    const t = finTotals();
    const all = a === 0 && b === months.length - 1;
    // all-time totals come straight from au fin stats; a sub-range is the sum of per-entry USD
    const exp = all ? Math.round(+F.usd.expense * 100) : t.exp, inc = all ? Math.round(+F.usd.income * 100) : t.inc;
    $('#f-exp').innerHTML = `−${usd(exp)}<span class="u">USD</span>`;
    $('#f-inc').innerHTML = `+${usd(inc)}`;
    const net = inc - exp;
    $('#f-net').textContent = (net >= 0 ? '+' : '−') + usd(Math.abs(net));
    $('#f-net').className = 'sm ' + (net >= 0 ? 'pos' : 'neg');
    const ML = (k) => new Date(k + '-01T00:00').toLocaleDateString('ru-RU', { month: 'short', year: 'numeric' });
    $('#f-per').textContent = all ? 'всё время' : `${ML(months[a])} — ${ML(months[b])}`;
    $('#f-cnt').textContent = `${nf0.format(t.n)} записей · ${F.voided_excluded} отменённых вне итога`;
    const S = $('#f-scrub'), w = 100 / months.length;
    $('.range', S).style.left = `${(a + 0.5) * w}%`; $('.range', S).style.width = `${(b - a) * w}%`;
    $('.h[data-h=a]', S).style.left = `${(a + 0.5) * w}%`; $('.h[data-h=b]', S).style.left = `${(b + 0.5) * w}%`;
    $$('.months span', S).forEach((s, k) => s.classList.toggle('in', k >= a && k <= b));
    // categories
    const ce = new Map(), ci = new Map();
    for (const e of ENT) if (!e.void && e.m >= a && e.m <= b && e.kind < 2) { const M = e.kind ? ci : ce; M.set(e.cat, (M.get(e.cat) || 0) + e.usd); }
    const row = (cat, v, tot, inc) => `<div class="cat ${inc ? 'inc' : ''}"><span class="n">${ico(IC.category[F.cats[cat]] || 'cat-other', '', 'color:' + (inc ? 'var(--inc)' : 'var(--exp)'))}${esc(F.cats[cat] || 'без категории')}</span><span class="bar" data-share="${v / tot}" data-inc="${inc ? 1 : 0}"></span><span class="v">${usd(v)}</span><span class="p">${((v / tot) * 100).toFixed(1)}%</span></div>`;
    const se = [...ce].sort((x, y) => y[1] - x[1]), si = [...ci].sort((x, y) => y[1] - x[1]);
    $('#f-cats').innerHTML = `<h2>Категории <span class="num">USD по курсу на дату записи</span></h2><span class="cap subh" style="margin-top:0">расход · ${se.length}</span>` + se.map(([c, v]) => row(c, v, t.exp || 1, false)).join('') + `<span class="cap subh">доход · ${si.length}</span>` + si.map(([c, v]) => row(c, v, t.inc || 1, true)).join('');
    // side: exchanges, currencies, rate sources
    const ex = ENT.filter((e) => e.kind === 2 && !e.void && e.m >= a && e.m <= b);
    const curs = ['BYN', 'RUB', 'USD', 'TON'];
    const srcN = F.usd.sources, srcT = Object.values(srcN).reduce((x, y) => x + y, 0);
    const SRCL = { nbrb: 'НБРБ', cbr: 'ЦБ РФ', usd: 'USD без пересчёта', kraken: 'Kraken' };
    const srcs = Object.entries(srcN).sort((x, y) => y[1] - x[1]);
    $('#f-side').innerHTML = `<h2>Обмены <span class="num">${ex.length} · отдельно от расхода и дохода</span></h2>
      <div id="f-ex">${curs.map((c, k) => `<span class="cl" style="top:${14 + k * 52}px">${c}</span>`).join('')}</div>
      <span class="cap subh">валюты · без пересчёта, всё время</span>
      <table class="nat"><thead><tr><th>валюта</th><th>расход</th><th>доход</th><th>отдано</th><th>получено</th></tr></thead><tbody>${F.native.map((r) => `<tr><td>${r.currency}</td><td>${nf2.format(+r.expense)}</td><td>${nf2.format(+r.income)}</td><td>${nf2.format(+r.exchanged_out)}</td><td>${nf2.format(+r.exchanged_in)}</td></tr>`).join('')}</tbody></table>
      <span class="cap subh">источник курса · ${nf0.format(F.usd.converted_entries)} записей пересчитано</span>
      <div class="srcs">${srcs.map(([, v]) => `<i style="flex:${v}"></i>`).join('')}</div>
      <div class="srcl">${srcs.map(([k, v]) => `<span>${SRCL[k] || k} ${nf0.format(v)}</span>`).join('')}<span>без курса ${F.usd.unconverted.entries}</span><span>устаревший ${F.usd.stale_entries}</span></div>`;
    F.exView = ex; F.exCurs = curs;
    renderRows();
  }
  function renderRows() {
    const { a, b, kind, cat, q, voided } = state.fin;
    const rows = ENT.filter((e) => e.m >= a && e.m <= b && (voided || !e.void) && (kind < 0 || e.kind === kind) && (cat < 0 || e.cat === cat) && (!q || (e.note || '').toLowerCase().includes(q) || String(e.amount).includes(q) || ('#' + e.id) === q));
    const shown = rows.slice(0, 220);
    $('#f-count').textContent = `${nf0.format(rows.length)} записей${rows.length > shown.length ? ` · показано ${shown.length}` : ''}`;
    $('#f-rows').innerHTML = shown.map((e) => {
      const amt = e.kind === 2 ? `${nf2.format(+e.amount)} ${e.cur} → ${nf2.format(+e.camt)} ${e.ccur}` : `${e.kind === 0 ? '−' : '+'}${nf2.format(+e.amount)} ${e.cur}`;
      return `<tr class="${e.void ? 'void' : ''}" ${e.void ? `data-why="${esc(e.reason || 'причина не указана')}"` : ''}><td class="k" title="${KIND[e.kind]}">${ico(['fin-expense', 'fin-income', 'fin-exchange'][e.kind], 'kg2 ' + ['exp', 'inc', 'exc'][e.kind])}</td><td class="num">${e.date.split('-').reverse().join('.')}</td><td>${esc(F.cats[e.cat] || (e.kind === 2 ? 'обмен' : '—'))}</td><td class="note">${esc(e.note)}</td><td class="num">${amt}</td><td class="num">${e.usd ? '≈ ' + usd(e.usd) : '<span class="src">нет курса</span>'}</td><td class="src">${esc(e.src || '')} ${e.units ? '<span class="mono">' + esc(e.units) + '</span>' : ''}</td></tr>`;
    }).join('');
  }
  $('#f-rows').addEventListener('pointerover', (e) => { const tr = e.target.closest('tr[data-why]'); if (tr) tip(e, 'отменена', tr.dataset.why); });
  $('#f-rows').addEventListener('pointerout', () => tip(null));
  $('#f-field').addEventListener('pointermove', (e) => {
    if (!finGeom || e.target.closest('#f-bar, #f-scrub')) { tip(null); return; }
    const m = Math.floor((e.clientX - finGeom.r.left) / finGeom.colW);
    if (m < 0 || m >= months.length || e.clientY < finGeom.r.top || e.clientY > finGeom.r.bottom) { state.fin.hover = -1; tip(null); return; }
    state.fin.hover = m;
    const M = F.months[m];
    tip(e, new Date(M.key + '-01T00:00').toLocaleDateString('ru-RU', { month: 'long', year: 'numeric' }), `доход +${nf2.format(+M.income)} · расход −${nf2.format(+M.expense)} USD · ${M.entries} записей`);
  });
  $('#f-field').addEventListener('pointerleave', () => { state.fin.hover = -1; tip(null); });

  // ------------------------------------------------------------------ tooltip
  function tip(e, t, text) {
    const el = $('#tip');
    if (!e) { el.classList.remove('on'); return; }
    el.innerHTML = `<span class="t">${esc(t)}</span>${esc(text)}`;
    el.style.left = Math.min(e.clientX + 14, VW - 400) + 'px'; el.style.top = e.clientY + 16 + 'px';
    el.classList.add('on');
  }

  // ------------------------------------------------------------------ screens + morphs
  const SCREENS = { memory: '#s-memory', tasks: '#s-tasks', finance: '#s-fin', spec: '#s-spec' };
  function setScreen(name, instant) {
    if (!SCREENS[name]) name = 'memory';
    const prev = state.screen;
    if (prev === name && !instant) return;
    state.prev = prev; state.screen = name; state.morph0 = performance.now();
    document.body.dataset.screen = name;
    for (const n of nodes) { n.bx = n.sx; n.by = n.sy; n.ba = n.a; }
    for (const e of ENT) { e.fx = e.sx; e.fy = e.sy; e.fa = e.a; e.fh = e.h; }
    if (name === 'finance' && prev !== 'finance') { // entries are born from the points of the field
      ENT.forEach((e, k) => { const n = nodes[(k * 7) % NN]; e.fx = e.sx = n.sx; e.fy = e.sy = n.sy; e.fa = e.a = Math.min(n.a, 0.8); e.fh = e.h = 2; });
    }
    for (const [k, sel] of Object.entries(SCREENS)) {
      const el = $(sel);
      el.classList.remove('enter', 'leave');
      if (k === name) { void el.offsetWidth; el.classList.add('on', 'enter'); }
      else if (k === prev && !instant) { el.classList.add('leave'); el.classList.remove('on'); setTimeout(() => el.classList.remove('leave'), 320); }
      else el.classList.remove('on');
    }
    $$('#rail nav button').forEach((b) => (b.dataset.go === name ? b.setAttribute('aria-current', 'page') : b.removeAttribute('aria-current')));
    if (location.hash !== '#' + name) history.replaceState(null, '', location.pathname + location.search + '#' + name);
    if (name === 'memory') fitCamera(false);
    $('#cmd').classList.toggle('away', document.activeElement !== cmdIn); // the command line comes up on "/"; in Memory the timeline holds the slot
    if (name !== 'memory' && state.chain) exitChain(true);
    tip(null);
  }

  const MORPH = REDUCE ? 1 : 900;
  function morphT(i, now) { // per-point progress with a stagger of up to 280 ms
    if (REDUCE) return 1;
    return outExpo(clamp((now - state.morph0 - hash(i) * 280) / MORPH, 0, 1));
  }

  // targets per screen
  function memoryTarget(n, now, out) {
    const ch = state.chain;
    if (ch && state.screen === 'memory') { // chain focus: the path straightens, everything else falls to ~8%
      const p = ch.pos.get(n.i);
      if (p) { out[0] = p[0]; out[1] = p[1]; out[2] = 1.25; return; }
      const [x0, y0] = project(n); out[0] = x0; out[1] = y0; out[2] = 0.08 * (CONF_LOOK[n.c][0] > 0.9 ? 1 : 0.85); return;
    }
    let [x, y] = project(n);
    if (state.replay.s < 0.999 && n.birth > state.replay.t + state.replay.fade) { out[0] = x; out[1] = y; out[2] = 0; return; }
    let a = CONF_LOOK[n.c][0] > 0.9 ? 1 : 0.85;
    a *= 1 - 0.4 * n.z; // older = dimmer (age also sets the radius inside the ganglion)
    if (state.hoverProj != null && n.hub !== state.hoverProj) a *= 0.22;
    if (state.filterType && n.t !== state.filterType && !(state.filterType === 'doc' && n.t === 'doc')) a *= 0.08;
    const focus = state.hover != null ? state.hover : state.sel;
    if (focus != null) {
      const near = n.i === focus || focusSet.has(n.i);
      a *= near ? 1.25 : 0.15; // silence around the thought
      if (state.sel != null && focusSet.has(n.i) && n.i !== state.sel && state.hover == null) {
        const dt = now - state.selT;
        const w = REDUCE ? 0 : dt < 560 ? outExpo(dt / 420) : 1 - inOut(clamp((dt - 760) / 760, 0, 1));
        if (w > 0) {
          const s = nodes[state.sel]; const [cx, cy] = project(s);
          const ring = focusRing.get(n.i) || 0;
          const R = 54 + Math.min(focusSet.size, 40) * 2.2;
          x = lerp(x, cx + Math.cos(ring) * R, w); y = lerp(y, cy + Math.sin(ring) * R, w);
        }
      }
    }
    if (state.screen === 'spec') a *= 0.07;
    out[0] = x; out[1] = y; out[2] = clamp(a, 0, 1.3);
  }
  let focusSet = new Set(), focusRing = new Map(), focusOf = null;
  function updateFocus() {
    const f = state.hover != null ? state.hover : state.sel;
    if (f === focusOf) return;
    focusOf = f; focusSet = new Set(); focusRing = new Map();
    if (f == null) return;
    state.focusRoot = f;
    fireFrom(f, null, 1, performance.now());
    adj[f].forEach(([j], k) => { focusSet.add(j); focusRing.set(j, (k / adj[f].length) * Math.PI * 2 - Math.PI / 2); });
  }
  const taskTargets = new Map();
  function readTaskTargets() {
    taskTargets.clear();
    for (const el of $$('#lanes .pt[data-g]')) { const r = el.getBoundingClientRect(); taskTargets.set(+el.dataset.g, [r.left + r.width / 2, r.top + r.height / 2, el.closest('.trow').classList.contains('sel'), el.closest('.lane').classList.contains('active'), el.closest('.lane').classList.contains('done')]); }
  }
  let finGeom = null;
  function readFinGeom() {
    const r = $('#f-cols').getBoundingClientRect();
    const pad = 0;
    const colW = (r.width - pad * 2) / months.length;
    const axis = r.top + r.height / 2;
    const scale = (r.height / 2 - 14) / F.maxMonth;
    const bars = $$('#f-cats .bar').map((b) => { const q = b.getBoundingClientRect(); return [q.left, q.top, q.width, q.height, +b.dataset.share, +b.dataset.inc]; });
    const exR = $('#f-ex') && $('#f-ex').getBoundingClientRect();
    finGeom = { r, colW, axis, scale, bars, exR };
  }

  // ------------------------------------------------------------------ loader (honest stages)
  const FULL = GA.full;
  function startLoader() {
    state.loader = { t0: performance.now(), phase: 'fetch', fetchP: 0, parseP: 0, layoutP: 0, done: 0 };
    newSim();
    $('#rail').style.opacity = 0; $('#cmd').style.opacity = 0;
    $('#rail').style.transition = $('#cmd').style.transition = 'none';
    for (const s of Object.values(SCREENS)) $(s).classList.remove('on', 'enter');
    $('#loader').style.display = ''; $('#stages').style.opacity = 1;
    $$('.stage').forEach((s) => s.classList.remove('on', 'ok'));
    $('#ld-note').textContent = 'прототип: байты и разбор идут по замеренному темпу; раскладка — настоящие тики d3-force';
    nodes.forEach((n, i) => { const u = hash(i * 3.1) * 0.999; n.mu = u; n.a = 0; });
    state.sel = null; showNode(null);
  }
  function loaderStep(now) {
    const L = state.loader; if (!L) return;
    const t = now - L.t0;
    const FETCH = REDUCE ? 400 : 2300, PARSE = REDUCE ? 300 : 1400;
    // fetch: bytes arrive with a slow start (TCP ramp) — in phase 2 this is the ReadableStream byte count
    L.fetchP = clamp(t / FETCH, 0, 1); L.fetchP = 1 - Math.pow(1 - L.fetchP, 1.6) * (1 - 0.25 * L.fetchP);
    L.fetchP = clamp(L.fetchP, 0, 1);
    if (t >= FETCH) { L.parseP = clamp((t - FETCH) / PARSE, 0, 1); }
    if (t >= FETCH + PARSE) { stepSim(REDUCE ? TICKS : 3); L.layoutP = tick / TICKS; }
    const set = (id, on, ok, p, txt) => { const s = $('#st-' + id); s.classList.toggle('on', on); s.classList.toggle('ok', ok); s.querySelector('.pb i').style.transform = `scaleX(${p})`; $('#st-' + id + '-n').textContent = txt; };
    const mb = (FULL.bytes / 1e6);
    set('fetch', true, L.fetchP >= 1, L.fetchP, `${(mb * L.fetchP).toFixed(1)} / ${mb.toFixed(1)} МБ`);
    set('parse', L.fetchP >= 1, L.parseP >= 1, L.parseP, `${nf0.format(Math.round(FULL.nodes * L.parseP))} узлов · ${nf0.format(Math.round(FULL.edges * L.parseP))} связей`);
    set('layout', L.parseP >= 1, L.layoutP >= 1, L.layoutP, `тик ${tick} / ${TICKS}`);
    if (L.layoutP >= 1 && !L.done) {
      L.done = now;
      $('#stages').style.opacity = 0;
      const tr = `opacity ${REDUCE ? 160 : 900}ms cubic-bezier(.16,1,.3,1)`;
      $('#rail').style.transition = $('#cmd').style.transition = tr;
      $('#rail').style.opacity = 1; $('#cmd').style.opacity = 1;
      setScreen(initialScreen(), true);
      const fs = params.get('focus');
      if (fs === 'expand') setTimeout(() => select(GA.expand), REDUCE ? 0 : 500);
    }
    if (L.done && now - L.done > 1200) { state.loader = null; $('#loader').style.display = 'none'; }
  }
  function initialScreen() { const h = location.hash.slice(1).split('?')[0]; return SCREENS[h] ? h : 'memory'; }

  // ------------------------------------------------------------------ glass object: the loader mark only
  function stretchInv(vx, vy, s) { // inverse of: stretch by s along velocity, 1/sqrt(s) across
    const a = Math.atan2(vy, vx), c = Math.cos(a), si = Math.sin(a);
    const p = 1 / s, q = Math.sqrt(s);
    // R^T diag(p,q) R, column-major; GL y is up, so flip vy sign
    const m00 = c * c * p + si * si * q, m01 = c * si * (p - q), m11 = si * si * p + c * c * q;
    return new Float32Array([m00, -m01, 0, -m01, m11, 0, 0, 0, 1]);
  }
  const I3 = new Float32Array([1, 0, 0, 0, 1, 0, 0, 0, 1]);

  // ------------------------------------------------------------------ nav pill spring
  const pill = { y: 0, v: 0, st: 1, sv: 0, init: false };
  function pillStep(dt) {
    const btn = $(`#rail nav button[data-go="${state.screen === 'spec' ? '__' : state.screen}"]`);
    const el = $('#pill');
    if (!btn) { el.style.opacity = 0; return; }
    el.style.opacity = 1;
    const ty = btn.offsetTop;
    if (!pill.init || REDUCE) { pill.y = ty; pill.v = 0; pill.init = true; }
    const om = 16, z = 0.86;
    pill.v += (om * om * (ty - pill.y) - 2 * z * om * pill.v) * dt; pill.y += pill.v * dt;
    const target = 1 + clamp(Math.abs(pill.v) / 1800, 0, 0.55) * 0.9;
    pill.sv += (169 * (target - pill.st) - 2 * 0.32 * 13 * pill.sv) * dt; pill.st += pill.sv * dt;
    const h = 34 * pill.st, w = 100 / Math.sqrt(pill.st);
    el.style.transform = `translateY(${pill.y - (h - 34) / 2}px)`; el.style.height = h + 'px'; el.style.width = w + 'px';
  }

  // ------------------------------------------------------------------ glass slabs from DOM
  const SLAB = {
    pill: { r: 17, bevel: 12, strength: 0.45, absorb: 0.15, disp: 1.0, rim: 1.0, sheen: 1.2 },
    pillbar: { r: 22, bevel: 14, strength: 0.55, absorb: 0.5, disp: 0.8, rim: 0.9, sheen: 0.8 },
    slab: { r: 14, bevel: 18, strength: 0.6, absorb: 0.62, disp: 0.7, rim: 0.85, sheen: 0.6 },
    'slab-clear': { r: 14, bevel: 18, strength: 0.6, absorb: 0.22, disp: 0.7, rim: 0.85, sheen: 0.6 },
    bar: { r: 16, bevel: 12, strength: 0.9, absorb: 0.78, disp: 0.8, rim: 1.0, sheen: 0.9 },
  };
  let secRGB = [0.36, 0.88, 0.9];
  function collectSlabs() {
    const out = [];
    const sc = getComputedStyle(document.body).getPropertyValue('--sec').match(/[\d.]+/g);
    if (sc && sc.length >= 3) secRGB = sc.slice(0, 3).map((v) => +v / 255);
    for (const el of $$('[data-glass]')) {
      if (out.length >= 10) break;
      const scr = el.closest('.screen');
      let pres = 1;
      if (scr) { if (!scr.classList.contains('on') && !scr.classList.contains('leave')) continue; pres = +getComputedStyle(scr).opacity; }
      if (el.closest('#rail')) pres *= +$('#rail').style.opacity || 0;
      if (el.closest('#cmd')) pres *= +getComputedStyle($('#cmd')).opacity || 0;
      if (el.id === 'pill') pres *= +el.style.opacity || 0;
      const cs = getComputedStyle(el);
      if (cs.display === 'none' || pres <= 0.01) continue;
      const r = el.getBoundingClientRect();
      if (r.bottom < 0 || r.top > VH || r.width < 4) continue;
      const k = SLAB[el.dataset.glass];
      let tint = null;
      if (el.id === 'pill') tint = [...secRGB, 0.85];
      else if (el.id === 'cmd') tint = [...secRGB, 0.25];
      else if (el.id === 't-detail') tint = [...GC.w, 0.3];
      else if (el.dataset.tint) tint = [...rgb(el.dataset.tint), 0.32];
      out.push({ x: r.left, y: r.top, w: r.width, h: r.height, presence: pres, ...k, r: Math.min(k.r, r.height / 2), tint });
    }
    return out;
  }

  // ------------------------------------------------------------------ icons (tools/build_icons.py -> icons.js)
  const IC = window.AU_ICONS || { icons: {}, category: {}, attr: '' };
  (function sprite() {
    const box = document.createElement('div');
    box.innerHTML = `<svg xmlns="http://www.w3.org/2000/svg" style="position:absolute;width:0;height:0" aria-hidden="true">${Object.entries(IC.icons).map(([n, b]) => `<symbol id="i-${n}" viewBox="0 0 24 24" ${IC.attr}>${b}</symbol>`).join('')}</svg>`;
    document.body.prepend(box.firstChild);
  })();
  const ico = (name, cls = '', style = '') => (IC.icons[name] ? `<svg class="ic ${cls}" style="${style}" aria-hidden="true"><use href="#i-${name}"/></svg>` : '');
  const TYPE_ICON = { decision: 'decision', problem: 'problem', solution: 'solution', concept: 'concept', session: 'session', task: 'task', work_log: 'work_log', user_fact: 'user_fact', person: 'person', doc: 'doc', skill: 'skill', file: 'file', dependency: 'dependency', server: 'server', project: 'project', reminder: 'reminder', module: 'module', crate: 'module', digest: 'digest', config: 'config' };
  const ATLAS_NAMES = [...new Set(Object.values(TYPE_ICON))];
  nodes.forEach((n) => { const nm = TYPE_ICON[n.t]; n.icon = nm ? ATLAS_NAMES.indexOf(nm) : -1; });
  let atlasReady = false;
  function buildAtlas(ggl2) { // node-type icons rasterised and turned into one signed distance atlas
    const CELL = 64, cols = 8, rows = Math.ceil(ATLAS_NAMES.length / cols);
    const W = cols * CELL, H = rows * CELL;
    const cv = document.createElement('canvas'); cv.width = W; cv.height = H;
    const c = cv.getContext('2d');
    let left = ATLAS_NAMES.length;
    ATLAS_NAMES.forEach((nm, k) => {
      const img = new Image();
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="${CELL}" height="${CELL}" ${IC.attr.replace('currentColor', '#fff').replace('1.5', '1.7')}>${IC.icons[nm].replace(/currentColor/g, '#fff')}</svg>`;
      img.onload = () => { c.drawImage(img, (k % cols) * CELL, Math.floor(k / cols) * CELL, CELL, CELL); if (--left === 0) finish(); };
      img.onerror = () => { if (--left === 0) finish(); };
      img.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
    });
    function finish() {
      const px = c.getImageData(0, 0, W, H).data, N2 = W * H, INF = 1e20;
      const fin = new Float64Array(N2), fout = new Float64Array(N2);
      for (let i = 0; i < N2; i++) { const inside = px[i * 4 + 3] > 127; fin[i] = inside ? INF : 0; fout[i] = inside ? 0 : INF; }
      const pass = (g, len, count, idx) => { const f = new Float64Array(len), d = new Float64Array(len), v = new Int32Array(len), z = new Float64Array(len + 1); for (let a = 0; a < count; a++) { for (let q = 0; q < len; q++) f[q] = g[idx(a, q)]; let k2 = 0; v[0] = 0; z[0] = -INF; z[1] = INF; for (let q = 1; q < len; q++) { let sv; do { const r = v[k2]; sv = (f[q] + q * q - (f[r] + r * r)) / (2 * q - 2 * r); if (sv <= z[k2]) k2--; else break; } while (k2 >= 0); k2++; v[k2] = q; z[k2] = sv; z[k2 + 1] = INF; } k2 = 0; for (let q = 0; q < len; q++) { while (z[k2 + 1] < q) k2++; const r = v[k2]; d[q] = (q - r) * (q - r) + f[r]; } for (let q = 0; q < len; q++) g[idx(a, q)] = d[q]; } };
      for (const g of [fin, fout]) { pass(g, W, H, (y, x) => y * W + x); pass(g, H, W, (x, y) => y * W + x); }
      const out = new Float32Array(N2);
      for (let i = 0; i < N2; i++) out[i] = fout[i] > 0 ? Math.sqrt(fout[i]) - 0.5 : -(Math.sqrt(fin[i]) - 0.5);
      // rows stay top-down: image row y lands at v = y / H, and screen-down vP.y maps to growing v
      ggl2.setAtlas(out, W, H, cols, rows, CELL);
      atlasReady = true;
    }
  }

  // ------------------------------------------------------------------ time replay
  const timeAt = (sv) => -Math.expm1((1 - sv) * LOGA);           // log-scaled: recent weeks get room
  const sAt = (t) => 1 - Math.log1p(-t) / LOGA;
  const BIG_DAYS = (() => {                                        // days with the most writing, named by a release or a task of that day
    const byDay = new Map();
    for (const n of nodes) { const d = Math.floor(n.age); if (!byDay.has(d)) byDay.set(d, []); byDay.get(d).push(n); }
    return [...byDay.entries()].sort((a, b) => b[1].length - a[1].length).slice(0, 7).map(([d, arr]) => {
      const rel = arr.find((n) => /релиз|release|v\d+\.\d+/i.test(n.label)) || arr.find((n) => n.t === 'task') || arr.find((n) => n.t === 'decision') || arr[0];
      const t = stripProj(rel.label).replace(/^T\d+\s*/, '');
      return { age: d + 0.5, n: arr.length, label: t.length > 34 ? t.slice(0, 33) + '…' : t };
    }).sort((a, b) => b.age - a.age);
  })();
  function buildTimeline() {
    const T2 = $('#m-time');
    const MN = ['янв', 'фев', 'мар', 'апр', 'май', 'июн', 'июл', 'авг', 'сен', 'окт', 'ноя', 'дек'];
    const now = new Date('2026-10-15T19:00');
    let ticks = '';
    for (let m = 0; m < 7; m++) { const d = new Date(now.getFullYear(), now.getMonth() - m, 1); const age = (now - d) / 864e5; if (age > AGE_MAX) break; ticks += `<span class="tk" style="left:${(sAt(-age) * 100).toFixed(2)}%">${MN[d.getMonth()]}</span>`; }
    ticks += BIG_DAYS.map((b) => `<span class="bd" style="left:${(sAt(-b.age) * 100).toFixed(2)}%" title="${esc(b.label)} · ${b.n} записей"></span>`).join('');
    T2.innerHTML = `<button class="play" aria-label="Проиграть память">${ico('fin-income', 'pl')}</button><div class="tr"><div class="fill"></div>${ticks}<div class="hd" role="slider" tabindex="0" aria-label="Момент памяти"></div></div><span class="when num"></span><span class="say"></span>`;
    $('.play', T2).onclick = togglePlay;
    const tr = $('.tr', T2);
    let drag = false;
    const setX = (x) => { const r = tr.getBoundingClientRect(); state.replay.s = clamp((x - r.left) / r.width, 0, 1); state.replay.playing = false; };
    tr.addEventListener('pointerdown', (e) => { drag = true; tr.setPointerCapture(e.pointerId); setX(e.clientX); });
    tr.addEventListener('pointermove', (e) => { if (drag) setX(e.clientX); });
    tr.addEventListener('pointerup', () => { drag = false; });
    $('.hd', T2).addEventListener('keydown', (e) => { const d = e.key === 'ArrowLeft' ? -0.02 : e.key === 'ArrowRight' ? 0.02 : 0; if (d) { state.replay.s = clamp(state.replay.s + d, 0, 1); e.preventDefault(); } });
  }
  function togglePlay() {
    const R = state.replay;
    if (REDUCE) { R.s = R.s >= 0.999 ? 0 : 1; return; }       // reduced motion: the timeline jumps instead of playing
    if (!R.playing && R.s >= 0.999) R.s = 0;
    R.playing = !R.playing;
  }
  let sayT = 0;
  function replayStep(dt, now) {
    const R = state.replay;
    if (R.playing) { R.s = Math.min(1, R.s + dt / 20); if (R.s >= 1) R.playing = false; }
    R.t = timeAt(R.s);
    R.fade = Math.max(0.004, Math.abs(R.t - timeAt(Math.max(0, R.s - 0.01))));
    if (R.playing && R.prevT != null && state.screen === 'memory') {
      let k = 0;
      for (const e of treeEdges) { const b = Math.max(e.c.birth, e.p.birth); if (b > R.prevT && b <= R.t && !e.work && k < 5) { pulses.push({ A: e.c, B: e.p, bend: 0.04, t0: now, dur: 520, amp: 2 }); k++; } }
      for (const b of BIG_DAYS) if (-b.age > R.prevT && -b.age <= R.t) { $('#m-time .say').textContent = b.label; sayT = now; }
    }
    R.prevT = R.t;
    const T2 = $('#m-time'); if (!T2.firstChild) return;
    $('.hd', T2).style.left = $('.fill', T2).style.width = (R.s * 100).toFixed(2) + '%';
    $('.when', T2).textContent = R.s >= 0.999 ? 'сейчас' : new Date(Date.parse('2026-10-15T19:00') + R.t * 864e5).toLocaleDateString('ru-RU', { day: 'numeric', month: 'short' });
    $('.play', T2).classList.toggle('on', R.playing);
    $('.say', T2).classList.toggle('on', now - sayT < 1800);
    T2.classList.toggle('active', R.s < 0.999);
  }

  // ------------------------------------------------------------------ neural field (GL)
  const ggl = glass ? window.GraphGL.init(glass.gl) : null;
  if (ggl) buildAtlas(ggl);
  const EB = new Float32Array(16000 * 20), NB = new Float32Array(16000 * 16);
  let fitK = 1;
  const pulses = [];
  let lastAmbient = 0;
  const nbIdx = links.filter((l) => l.rel !== 'belongs_to');
  const crossIdx = nbIdx.filter((l) => l.source.hub !== l.target.hub);
  function inward(n, t0, hop) { // a pulse runs to the parent, then on toward the root
    if (pulses.length > 320 || hop > 9) return;
    const p = parent[n.i];
    if (p === -1) return;
    const B = p === ROOT ? rootN : nodes[p];
    if (B !== rootN && B.a < 0.05) return;
    pulses.push({ A: n, B, bend: 0.04, t0, dur: p === ROOT ? 900 : 420 + hash(n.i) * 220, amp: 1.2 + 0.15 * hop, then: (t) => { if (B !== rootN) inward(B, t, hop + 1); } });
  }
  function fireFrom(f, from, hop, t0) { // focus: what grew from this node, wave outward, one hop then the next
    if (pulses.length > 320 || hop > 3) return;
    let k = 0;
    const A = nodes[f];
    const outs = [...kids[f], ...adj[f].filter(([j, r]) => r !== 'belongs_to' && parent[f] !== j).map(([j]) => j)];
    for (const j of outs) {
      if (j === from) continue;
      const B = nodes[j];
      pulses.push({ A, B, bend: 0.08, t0: t0 + hash(j) * 90, dur: 480 + hash(j + 3) * 240, amp: 2.6 / hop, then: (t) => { if (focusOf === state.focusRoot) fireFrom(j, f, hop + 1, t); } });
      if (++k >= (hop === 1 ? 40 : 6)) break;
    }
  }
  function curvePt(A, B, bend, t) {
    const dx = B.sx - A.sx, dy = B.sy - A.sy, L = Math.hypot(dx, dy) || 1;
    const cx = (A.sx + B.sx) / 2 - (dy / L) * bend * L, cy = (A.sy + B.sy) / 2 + (dx / L) * bend * L;
    const s1 = 1 - t; return [s1 * s1 * A.sx + 2 * s1 * t * cx + t * t * B.sx, s1 * s1 * A.sy + 2 * s1 * t * cy + t * t * B.sy];
  }
  const colTmp = [0, 0, 0];
  function colOf(n, sat) {
    const c = GC[n.g]; const L = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    colTmp[0] = lerp(L * 0.86, c[0], sat); colTmp[1] = lerp(L * 0.93, c[1], sat); colTmp[2] = lerp(L * 1.05, c[2], sat);
    return colTmp;
  }
  const CONF_I = [0.62, 1.0, 0.82, 0.62, 0.5];
  function buildGraph(now, scr, L) {
    if (!ggl) return null;
    const zr = clamp(cam.k / fitK, 0.15, 60);
    const memo = scr === 'memory' || scr === 'spec';
    const loaderOn = L && !L.done;
    const glyphZ = memo ? sstep(2.2, 4.2, zr) : 0;
    const satZ = loaderOn ? 0.55 : memo ? lerp(0.5, 1, sstep(1.4, 4, zr)) : 1;
    const focus = focusOf;
    const hp = state.hoverProj;
    for (const G of ganglia.values()) { const area = Math.PI * Math.pow(G.rr * cam.k, 2) || 1; G.f = clamp(Math.sqrt(0.0026 / (G.members.length / area)), 0.2, 1.0); }
    let ne = 0, nn = 0;
    const putE = (A, B, bend, w, t0, t1, ca, a0, cb, a1, birth = Math.max(A.birth ?? -1e6, B.birth ?? -1e6)) => {
      if (ne >= 16000) return; const o = ne++ * 20;
      EB[o + 16] = birth;
      EB[o] = A.sx; EB[o + 1] = A.sy; EB[o + 2] = B.sx; EB[o + 3] = B.sy; EB[o + 4] = bend; EB[o + 5] = w; EB[o + 6] = t0; EB[o + 7] = t1;
      EB[o + 8] = ca[0]; EB[o + 9] = ca[1]; EB[o + 10] = ca[2]; EB[o + 11] = a0; EB[o + 12] = cb[0]; EB[o + 13] = cb[1]; EB[o + 14] = cb[2]; EB[o + 15] = a1;
    };
    const putN = (x, y, size, ext, c, I, halo, shape, glyph, icon = -1, birth = -1e6, sup = 1e9) => {
      if (nn >= 16000 || !(I > 0.002)) return; const o = nn++ * 16;
      NB[o + 12] = birth; NB[o + 13] = sup; NB[o + 14] = replayOn ? 1 : 0;
      NB[o] = x; NB[o + 1] = y; NB[o + 2] = size; NB[o + 3] = ext; NB[o + 4] = c[0]; NB[o + 5] = c[1]; NB[o + 6] = c[2]; NB[o + 7] = I; NB[o + 8] = halo; NB[o + 9] = shape; NB[o + 10] = glyph; NB[o + 11] = icon;
    };
    const iconZ = memo && atlasReady ? sstep(4.6, 7.5, zr) : 0;
    const replayOn = memo && !loaderOn && state.replay.s < 0.999;
    // dendrites converging on the root: the chain tree at rest, every edge of a node on focus
    const eBase = loaderOn ? 0.12 * sstep(0.45, 1, L.layoutP) : memo ? lerp(0.22, 0.4, sstep(1, 4, zr)) * (scr === 'spec' ? 0.15 : 1) : 0.0;
    const chain = state.chain;
    if (eBase > 0) {
      const onScr = (n) => n.sx > -80 && n.sx < VW + 80 && n.sy > -80 && n.sy < VH + 80;
      // at rest: chain-tree edges only, work edges never, strongest first, at most ~850 on screen
      const cand = [];
      if (!chain) for (const e of treeEdges) {
        if ((e.work || e.rel === 'belongs_to') && focus == null) continue; // membership is not a chain: no spokes
        const A = e.c, B = e.p;
        if (A.a < 0.03 || B.a < 0.03 || (!onScr(A) && !onScr(B))) continue;
        if (focus != null && A.i !== focus && B.i !== focus) { cand.push([e, e.imp * 0.1]); continue; }
        cand.push([e, e.imp]);
      }
      cand.sort((x, y) => y[1] - x[1]);
      const keep = cand.slice(0, 850);
      const budgetK = clamp(Math.sqrt(320 / Math.max(keep.length, 1)), 0.4, 1.2);
      for (const [e] of keep) {
        const A = e.c, B = e.p;
        const lw = Math.log2(1 + leaves[A.i]);
        let al = eBase * budgetK * Math.min(A.a, B.a, 1) * (0.45 + 0.25 * Math.min(lw, 4));
        let w = 0.7 + 0.55 * Math.min(lw, 5);
        let sat = satZ;
        if (focus != null) { if (A.i === focus || B.i === focus) { al = 0.9; w += 0.4; sat = 1; } else al *= 0.1; }
        if (hp != null && A.hub !== hp) al *= 0.22;
        if (al < 0.004) continue;
        putE(A, B, 0.08 * (hash(e.li) - 0.5), w, 0, 1, colOf(A, sat).slice(), al * 0.55, colOf(B, sat).slice(), al);
        if (zr > 2.2 && al > 0.08) { const [x, y] = curvePt(A, B, 0.08 * (hash(e.li) - 0.5), 0.965); putN(x, y, 1.5, 2.2, colOf(B, sat), al * 1.5, 0.6, 0, 0); }
      }
      // projects feed the root: one luminous axon per ganglion
      if (!chain) for (const G of ganglia.values()) {
        if (G.key < 0) continue;
        const H = nodes[G.key]; if (H.a < 0.05) continue;
        const al = (hp != null && hp !== G.key ? 0.06 : 0.35) * (loaderOn ? sstep(0.6, 1, L.layoutP) : 1) * (focus != null ? 0.3 : 1);
        putE(H, rootN, 0.06, 1.2 + Math.log2(1 + G.members.length) * 0.35, 0, 1, GC[G.dom], al * 0.5, [0.91, 0.9, 0.88], al);
      }
      // focus: every edge of the node, context included
      if (focus != null && !chain) for (const [j, rel] of adj[focus]) {
        if (rel === 'belongs_to') continue;
        const A = nodes[focus], B = nodes[j];
        if (parent[j] === focus || parent[focus] === j) continue; // already drawn as tree edge
        const long = Math.hypot(A.sx - B.sx, A.sy - B.sy) > Math.min(VW, VH) * 0.35 && A.hub === B.hub;
        putE(A, B, 0.18, 1.2, 0, 1, colOf(A, 1).slice(), long ? 0.3 : 0.7, colOf(B, 1).slice(), long ? 0.3 : 0.7);
      }
      // chain focus: the path, its stubs, and the run to the root
      if (chain) {
        const seq = chain.seq;
        for (let k = 0; k + 1 < seq.length; k++) { const A = seq[k], B = seq[k + 1]; putE(A, B, 0.04, 2.2, 0, 1, colOf(A, 1).slice(), 0.85, colOf(B, 1).slice(), 0.85); }
        for (const [c, m] of chain.stubs) { const A = nodes[c], B = nodes[m]; putE(A, B, 0.1, 1.1, 0, 1, colOf(A, 1).slice(), 0.45, colOf(B, 1).slice(), 0.45); }
      }
    }
    // action potentials: inward by default (the field converges on the root), outward on focus
    if (memo && !loaderOn && !REDUCE) {
      if (!chain && now - lastAmbient > 170 && treeEdges.length) {
        lastAmbient = now;
        for (let tries = 0; tries < 6; tries++) {
          const e = treeEdges[Math.floor(hash(now * 1.37 + tries) * treeEdges.length) % treeEdges.length];
          if (e.c.depth >= 3 && e.c.a > 0.2 && !e.work) { inward(e.c, now, 0); break; }
        }
      }
      if (chain && now - (chain.lastRun || 0) > 2600) { chain.lastRun = now; chain.seq.forEach((A, k) => { const B = chain.seq[k + 1]; if (B) pulses.push({ A, B, bend: 0.04, t0: now + k * 360, dur: 380, amp: 2.4 }); }); }
      for (let k = pulses.length - 1; k >= 0; k--) {
        const P = pulses[k]; const u = (now - P.t0) / P.dur;
        if (u < 0) continue;
        if (u >= 1) { pulses.splice(k, 1); if (P.then) P.then(now); continue; }
        const head = u, tail = Math.max(0, u - 0.22);
        const ca = P.A === rootN ? [0.91, 0.9, 0.88] : colOf(P.A, 1).slice();
        putE(P.A, P.B, P.bend, 2.0, tail, head, ca, 0, ca, P.amp);
        const [x, y] = curvePt(P.A, P.B, P.bend, head); putN(x, y, 2.2, 3, ca, P.amp * 0.8, 1.4, 0, 0);
      }
    } else pulses.length = 0;
    // the root: one soma everything converges on
    if (memo && chain && chain.pos.has(ROOT)) { const pr = chain.pos.get(ROOT), p0 = morphT(7, now); rootN.sx = lerp(rootN.bx ?? rootN.sx, pr[0], p0); rootN.sy = lerp(rootN.by ?? rootN.sy, pr[1], p0); putN(rootN.sx, rootN.sy, 8, 3.5, [0.95, 0.94, 0.92], 2.4, 3, 0, 0); }
    if (memo && !chain) { const [rx, ry] = project(rootN); rootN.sx = rx; rootN.sy = ry; putN(rx, ry, 7 * clamp(Math.pow(zr, 0.3), 1, 2), 3.5, [0.95, 0.94, 0.92], loaderOn ? L.layoutP * 2 : 2.2, 3, 0, 0); }
    // neurons
    for (const n of nodes) {
      if (n.a <= 0.01) continue;
      const inTasks = scr === 'tasks' && taskTargets.has(n.i);
      if (inTasks) { const tt = taskTargets.get(n.i); const sc = n.st === 'active' ? GC.w : n.st === 'blocked' ? GC.p : n.st === 'done' ? GC.o : GC.i; // backlog steel, only active amber
        putN(n.sx, n.sy, tt[2] ? 6.5 : tt[4] ? 4.2 : 5.2, 2.6, sc, (tt[2] ? 2.4 : n.st === 'active' ? 1.9 : tt[4] ? 0.45 : 0.9) * Math.min(n.a, 1), 1.6, 3, 1); continue; }
      if (scr === 'tasks' || scr === 'finance') { putN(n.sx, n.sy, 1.6, 2.4, colOf(n, 0.6), n.a * 0.6, 0.8, 0, 0); continue; }
      const G = ganglia.get(n.hub);
      const isF = n.i === focus, near = focus != null && focusSet.has(n.i);
      const recency = n.age < 7 ? 1.75 : n.age < 30 ? 1.1 : 0.72;
      let I = loaderOn ? n.a * 1.7 * CONF_I[n.c] : n.a * recency * CONF_I[n.c] * (G ? G.f || 1 : 1);
      let sat = satZ;
      const leaf = n.nd <= 1 && !n.attn && n.t !== 'project';
      let size = (1.5 + 0.85 * Math.log2(1 + n.nd)) * clamp(Math.pow(zr, 0.45), 0.85, 2.8);
      let ext = 3.2, halo = 2.0, glyph = glyphZ;
      if (loaderOn) { size = lerp(1.7, size, sstep(0.2, 1, L.layoutP)); glyph = 0; halo = 1.6; }
      else if (leaf && zr < 1.8) { size = 1.25; ext = 2.2; halo = 0.7; I *= 0.85; } // dust at far zoom
      if (n.attn && !loaderOn) { size = Math.max(size, 4.2); glyph = 1; I = Math.max(I, 1.5 * n.a); sat = 1; halo = 2.6; }
      if (n.t === 'project') { size = Math.max(size, 4.6); I *= 1.2; }
      if (hp != null && n.hub === hp) { I *= 1.35; sat = Math.max(sat, 0.9); }
      if (isF) { size *= 1.7; I = 2.6; sat = 1; glyph = Math.max(glyph, 0.6); }
      else if (near) { I *= 1.5; sat = 1; }
      if (scr === 'spec') I *= 0.5;
      let gl2 = glyph;
      if (iconZ > 0 && n.icon >= 0) { gl2 = glyph + iconZ; size = lerp(size, Math.max(size, 9), iconZ); ext = Math.max(ext, 2.4); } // far dot -> mid silhouette -> close icon
      putN(n.sx, n.sy, size, ext, colOf(n, sat), I, halo, n.shape, gl2, n.icon, n.birth, n.supAt);
    }
    return ggl.draw({ vw: VW, vh: VH, ratio: DPR, edges: EB, nEdges: ne, nodes: NB, nNodes: nn, tNow: replayOn ? state.replay.t : 0, tFade: replayOn ? state.replay.fade : 0.001 });
  }

  // ganglion geometry (centroid and a robust radius), recomputed while the layout moves
  let geomTick = -1;
  function gangliaGeom() {
    if (geomTick === tick) return; geomTick = tick;
    for (const G of ganglia.values()) {
      let x = 0, y = 0; for (const n of G.members) { x += n.x; y += n.y; } x /= G.members.length; y /= G.members.length;
      const ds = G.members.map((n) => Math.hypot(n.x - x, n.y - y)).sort((a, b) => a - b);
      G.cx = x; G.cy = y; G.rr = Math.max(ds[Math.floor(ds.length * 0.9)] || 8, 8);
    }
  }
  const agoTxt = (d) => (d < 1 / 24 ? 'только что' : d < 1 ? `${Math.max(1, Math.round(d * 24))} ч назад` : `${Math.round(d)} дн. назад`);
  const ICON = { task: (c) => `<svg width="9" height="9" viewBox="0 0 10 10"><rect x="1.5" y="1.5" width="7" height="7" fill="${c}"/></svg>`, prob: (c) => `<svg width="10" height="9" viewBox="0 0 10 9"><path d="M5 .5 9.5 8.5H.5z" fill="${c}"/></svg>` };
  const labelEls = new Map();
  function buildLabels() {
    const box = $('#m-labels'); box.innerHTML = '';
    const maxN = Math.max(...[...projHub.values()].map((p) => p.n));
    for (const G of ganglia.values()) {
      const el = document.createElement('button'); el.className = 'plabel';
      const info = G.info;
      const fs = info ? 11 + 3 * Math.sqrt(info.n / maxN) : 11;
      el.dataset.k = G.key;
      el.innerHTML = `<span class="pn" style="font-size:${fs.toFixed(1)}px">${esc(G.label)}</span><span class="pm">${info ? `<span class="num">${nf0.format(info.n)}</span> узлов <i class="act ${info.last < 7 ? 'hot' : ''}"></i>${agoTxt(info.last)}` : `<span class="num">${G.members.length}</span> в выборке`}</span>`;
      el.onclick = () => flyToGanglion(G);
      el.onpointerenter = () => { state.hoverProj = G.key; showCard(G); };
      el.onpointerleave = () => { state.hoverProj = null; showCard(null); };
      box.append(el); labelEls.set(G.key, el);
    }
  }
  function updateLabels(scr, L) {
    const box = $('#m-labels');
    const on = scr === 'memory' && (!L || L.done);
    box.style.opacity = on ? (L ? sstep(0, 900, performance.now() - L.done) : 1) : 0;
    if (!on) return;
    gangliaGeom();
    const placed = [];
    const railOff = $('#s-memory').getBoundingClientRect().left;
    const order = [...ganglia.values()].sort((a, b) => (b.info ? b.info.n : 0) - (a.info ? a.info.n : 0));
    const zr = cam.k / fitK;
    for (const G of order) {
      const el = labelEls.get(G.key); if (!el) continue;
      const sector = G.lx != null && cam.k / fitK < 2.2;
      const [cx, cy] = sector ? project({ x: G.lx, y: G.ly }) : project({ x: G.cx, y: G.cy });
      const rpx = sector ? 0 : G.rr * cam.k;
      const w = el.offsetWidth, h = el.offsetHeight;
      let x = cx - w / 2, y = cy - rpx - h - 8;
      const hit = (x0, y0) => placed.some((r) => x0 < r[0] + r[2] + 6 && x0 + w + 6 > r[0] && y0 < r[1] + r[3] + 4 && y0 + h + 4 > r[1]);
      if (hit(x, y) || y < 8) y = cy + rpx + 8;
      const vis = !hit(x, y) && zr < 7 && (sector || rpx > 6) && x > 140 && x + w < VW - 10 && y > 6 && y + h < VH - 84 && !(state.sel != null && Math.abs(y + h - (nodes[state.sel].sy - 16)) < 30 && Math.abs(cx - nodes[state.sel].sx) < 240);
      el.style.transform = `translate(${(x - railOff).toFixed(1)}px, ${y.toFixed(1)}px)`;
      el.classList.toggle('off', !vis);
      el.classList.toggle('hl', state.hoverProj === G.key);
      el.classList.toggle('dim', state.hoverProj != null && state.hoverProj !== G.key);
      if (vis) placed.push([x, y, w, h]);
      const pmEl = el.querySelector('.pm');
      if (state.replay.s < 0.999) { const born = G.members.filter((m) => m.birth <= state.replay.t).length; el.classList.toggle('off', !vis || born === 0); pmEl.dataset.r = '1'; pmEl.innerHTML = `<span class="num">${nf0.format(born)}</span> в выборке к этой дате`; }
      else if (pmEl.dataset.r) { delete pmEl.dataset.r; const info = G.info; pmEl.innerHTML = info ? `<span class="num">${nf0.format(info.n)}</span> узлов <i class="act ${info.last < 7 ? 'hot' : ''}"></i>${agoTxt(info.last)}` : `<span class="num">${G.members.length}</span> в выборке`; }
    }
    updateLegend(zr);
  }
  function showCard(G, e) {
    const c = $('#m-card');
    if (!G) { c.classList.remove('on'); return; }
    const info = G.info;
    const cnt = { k: 0, w: 0, p: 0, o: 0, i: 0 }; G.members.forEach((m) => cnt[m.g]++);
    const tot = G.members.length;
    c.innerHTML = `<h3>${esc(G.label)}</h3>
      <dl><dt>узлов</dt><dd class="num">${nf0.format(info ? info.n : tot)}</dd>
      <dt>активные задачи</dt><dd class="num">${info ? info.act : '—'}</dd>
      <dt>открытые проблемы</dt><dd class="num">${info ? nf0.format(info.open) : '—'}</dd>
      ${info && info.blk ? `<dt>в блоке</dt><dd class="num">${info.blk}</dd>` : ''}
      <dt>активность</dt><dd>${info ? agoTxt(info.last) : '—'}</dd>
      <dt>без связей с корнем</dt><dd class="num">${G.orphans} из ${G.members.length} в выборке</dd></dl>
      <div class="mix">${Object.entries(cnt).filter(([, v]) => v).map(([k, v]) => `<i style="flex:${v};background:${GHEX[k]}" title="${GNAME[k]} ${v}"></i>`).join('')}</div>
      <p>клик — приблизить проект</p>`;
    c.dataset.tint = GHEX[G.dom];
    c.classList.add('on');
  }
  function flyToGanglion(G) {
    gangliaGeom();
    const aw = VW - 132 - 120, ah = VH - 200;
    fly({ x: G.cx, y: G.cy, k: Math.min(aw, ah) / (G.rr * 2.6) }, 820);
    state.hoverProj = null; showCard(null);
  }
  let legendMode = '';
  function updateLegend(zr) {
    const mode = zr < 2.6 ? 'over' : 'near';
    if (mode === legendMode) return; legendMode = mode;
    const L = $('#m-legend');
    const dot = (hex, s = 8) => `<i class="sw" style="width:${s}px;height:${s}px;background:${hex};box-shadow:0 0 8px ${hex}"></i>`;
    if (mode === 'over') {
      L.innerHTML = `<p class="lg"><span class="neb"></span>облако — проект, размер — число узлов</p>
        <p class="lg"><span class="core"></span>яркое ядро — записано за 7 дней</p>
        <p class="lg">${ico('task', '', 'color:' + GHEX.w)}активная задача</p>
        <p class="lg">${ico('problem', '', 'color:' + GHEX.p)}открытая проблема</p>
        <p class="lg ax"><span class="axon"></span>длинная линия — связь между проектами</p>
        <div class="groups">${Object.keys(GHEX).map((k) => `<span>${dot(GHEX[k])}${GNAME[k]}</span>`).join('')}</div>`;
    } else {
      const order = ['decision', 'concept', 'solution', 'problem', 'task', 'session', 'work_log', 'user_fact', 'project', 'doc'];
      L.innerHTML = '<div class="types"></div>' + `<div class="groups">${Object.keys(GHEX).map((k) => `<span>${dot(GHEX[k])}${GNAME[k]}</span>`).join('')}</div>`;
      const T2 = L.querySelector('.types');
      for (const t of order) { const s2 = document.createElement('span'); s2.insertAdjacentHTML('beforeend', ico(TYPE_ICON[t], '', 'color:' + GHEX[GROUP[t] || 'i'])); s2.append(document.createTextNode(t.replace('_', ' '))); T2.append(s2); }
    }
  }

  // ------------------------------------------------------------------ render
  const tmp = [0, 0, 0];
  let last = performance.now(), fpsAcc = [], mouse = { x: -1e4, y: -1e4 }, vignette = null;
  function frame(now) {
    const dt = Math.min((now - last) / 1000, 0.05); last = now;
    fpsAcc.push(dt); if (fpsAcc.length > 60) fpsAcc.shift();
    if (camFly) { const p = inOut(clamp((now - camFly.t0) / camFly.dur, 0, 1)); cam.x = lerp(camFly.from.x, camFly.to.x, p); cam.y = lerp(camFly.from.y, camFly.to.y, p); cam.k = Math.exp(lerp(Math.log(camFly.from.k), Math.log(camFly.to.k), p)); if (p >= 1) camFly = null; }
    cam.ox = lerp(cam.ox || originX(), originX(), REDUCE ? 1 : 0.12); cam.oy = VH / 2 - 10;
    loaderStep(now);
    updateFocus();
    replayStep(dt, now); pillStep(dt);

    const c = ctx;
    c.setTransform(DPR, 0, 0, DPR, 0, 0);
    c.globalCompositeOperation = 'source-over';
    c.fillStyle = '#070a12'; c.fillRect(0, 0, VW, VH);
    c.fillStyle = vignette || (vignette = (() => { const g = c.createRadialGradient(VW * 0.55, VH * 0.45, 0, VW * 0.55, VH * 0.45, Math.max(VW, VH) * 0.75); g.addColorStop(0, 'rgba(20,34,52,0.35)'); g.addColorStop(1, 'rgba(0,0,0,0)'); return g; })());
    c.fillRect(0, 0, VW, VH);
    const L = state.loader;
    const scr = state.screen;

    // ---- graph points: targets per screen, blended by the morph
    if (scr === 'tasks') readTaskTargets();
    if (L && !L.done) {
      if (L.layoutP > 0 || tick > 0) { if (!camFly) { Object.assign(cam, fitTarget()); fitK = cam.k; } }
      const mx = markXf();
      for (const n of nodes) {
        const [mx0, my0] = markPoint(n.mu);
        const px = mx.cx + (mx0 - MK.c[0]) * mx.s, py = mx.cy + (my0 - MK.c[1]) * mx.s;
        const born = L.parseP > n.mu ? 1 : 0;
        const drift = L.parseP * 22 * (hash(n.i) - 0.5), drift2 = L.parseP * 22 * (hash(n.i + 91.7) - 0.5);
        const [lx, ly] = project(n);
        const p = REDUCE ? (L.layoutP >= 1 ? 1 : 0) : outExpo(clamp((L.layoutP - n.mu * 0.35) / 0.65, 0, 1));
        n.sx = lerp(px + drift, lx, p); n.sy = lerp(py + drift2, ly, p);
        n.a = born * lerp(0.28, CONF_LOOK[n.c][0] > 0.9 ? 1 : 0.85, p);
      }
    } else {
      const showGraph = scr === 'memory' || scr === 'spec' || (L && L.done);
      for (const n of nodes) {
        if (scr === 'tasks') {
          const tt = taskTargets.get(n.i);
          if (tt) { tmp[0] = tt[0]; tmp[1] = tt[1]; tmp[2] = 1.3; } else { memoryTarget(n, now, tmp); tmp[2] = 0; }
        } else if (scr === 'finance') { memoryTarget(n, now, tmp); tmp[2] = 0; }
        else memoryTarget(n, now, tmp);
        const p = morphT(n.i, now);
        n.sx = lerp(n.bx, tmp[0], p); n.sy = lerp(n.by, tmp[1], p); n.a = lerp(n.ba, tmp[2], p);
        if (!showGraph && p >= 1 && tmp[2] === 0) n.a = 0;
      }
    }

    // ---- 2D fallback for the graph (no WebGL2); the GL path is buildGraph()
    const edgeA = L && !L.done ? sstep(0.5, 1, L.layoutP) : 1;
    if (!glass && edgeA > 0) {
      c.lineWidth = 1;
      const focus = focusOf;
      c.beginPath();
      for (const l of links) { const a = l.source, b = l.target; if (a.a < 0.05 || b.a < 0.05) continue; if (focus != null && (a.i === focus || b.i === focus)) continue; c.moveTo(a.sx, a.sy); c.lineTo(b.sx, b.sy); }
      const avgA = scr === 'memory' || scr === 'spec' || (L && !L.done) ? 1 : 0.4;
      c.strokeStyle = `rgba(232,230,225,${(focus != null ? 0.018 : 0.05) * edgeA * avgA * (scr === 'spec' ? 0.4 : 1)})`;
      c.stroke();
      if (focus != null && scr === 'memory') {
        const f = nodes[focus];
        c.strokeStyle = 'rgba(232,230,225,0.32)'; c.beginPath();
        for (const [j] of adj[focus]) { c.moveTo(f.sx, f.sy); c.lineTo(nodes[j].sx, nodes[j].sy); }
        c.stroke();
        // light pulse travelling along each hovered edge
        c.globalCompositeOperation = 'lighter';
        adj[focus].forEach(([j, , dir], k) => {
          const u = ((now / 900 + k * 0.137) % 1); const a2 = dir === 'out' ? u : 1 - u;
          const x = lerp(f.sx, nodes[j].sx, a2), y = lerp(f.sy, nodes[j].sy, a2);
          const g = c.createRadialGradient(x, y, 0, x, y, 7); g.addColorStop(0, 'rgba(255,255,255,0.9)'); g.addColorStop(1, 'rgba(255,255,255,0)');
          c.fillStyle = g; c.fillRect(x - 7, y - 7, 14, 14);
        });
        c.globalCompositeOperation = 'source-over';
      }
    }

    if (!glass) {
    c.globalCompositeOperation = 'lighter';
    const zk = clamp(Math.pow(cam.k / (fitTarget().k || 1), 0.35), 0.8, 2.4);
    for (const n of nodes) {
      if (n.a <= 0.01) continue;
      const sp = sprite(n.t, n.c);
      const inTasks = scr === 'tasks' && taskTargets.has(n.i);
      const degS = 1 + Math.min(Math.log2(1 + n.deg) * 0.12, 0.6);
      let s = (inTasks ? 1.25 : zk * degS * (1 - 0.3 * n.z)) / 4;
      if (n.i === focusOf) s *= 1.8;
      if (L && !L.done) s *= lerp(0.4, 1, sstep(0, 0.8, L.layoutP));
      const w = sp.S * s;
      c.globalAlpha = Math.min(n.a, 1);
      c.drawImage(sp.cv, n.sx - w / 2, n.sy - w / 2, w, w);
      if (inTasks) { const tt = taskTargets.get(n.i); if (tt[3] || tt[2]) { const g = c.createRadialGradient(n.sx, n.sy, 0, n.sx, n.sy, 14); g.addColorStop(0, `rgba(232,230,225,${tt[2] ? 0.5 : 0.28})`); g.addColorStop(1, 'rgba(232,230,225,0)'); c.fillStyle = g; c.fillRect(n.sx - 14, n.sy - 14, 28, 28); } }
    }
    c.globalAlpha = 1;
    c.globalCompositeOperation = 'source-over';

    // project labels
    if ((scr === 'memory') && (!L || L.done)) {
      c.font = '11px Geist, sans-serif'; c.textAlign = 'center';
      const selN = state.sel != null ? nodes[state.sel] : null;
      for (const [i, p] of projHub) { const n = nodes[i]; if (n.a < 0.2) continue; if (selN && Math.abs(n.sy - 12 - (selN.sy - 16)) < 24 && Math.abs(n.sx - selN.sx) < 220) continue; c.fillStyle = `rgba(232,230,225,${0.5 * Math.min(n.a, 1)})`; c.fillText(p.label.length > 26 ? p.label.slice(0, 25) + '…' : p.label, n.sx, n.sy - 12); }
      if (state.sel != null) { const n = nodes[state.sel]; c.fillStyle = 'rgba(232,230,225,0.95)'; c.font = '13px Geist, sans-serif'; const t = stripProj(n.label); c.fillText(t.length > 54 ? t.slice(0, 53) + '…' : t, n.sx, n.sy - 16); }
    }

    }
    updateLabels(scr, L);
    updateChainLabels();
    { const rl = $('#m-root'); const on = scr === 'memory' && (!L || L.done) && !state.chain; rl.style.opacity = on ? 1 : 0; if (on) rl.style.transform = `translate(${(rootN.sx - $('#s-memory').getBoundingClientRect().left - 40).toFixed(1)}px, ${(rootN.sy + 14).toFixed(1)}px)`; }

    // ---- finance: columns of light built from entries
    const finOn = scr === 'finance' || state.prev === 'finance' && now - state.morph0 < MORPH + 300;
    if (finOn) {
      readFinGeom();
      const G = finGeom, { a: ra, b: rb } = state.fin;
      const narrowed = !(ra === 0 && rb === months.length - 1);
      c.globalCompositeOperation = 'lighter';
      for (const e of ENT) {
        let tx, ty, th, ta;
        if (scr === 'finance' && e.m >= 0 && !e.void && e.kind < 2) {
          const cx = G.r.left + G.colW * (e.m + 0.5) + (e.kind ? -6 : 6) * 0;
          th = Math.max(e.usd * G.scale - 1, 0.6);
          ty = e.kind === 1 ? G.axis - (e.base * G.scale) - th : G.axis + e.base * G.scale + 1;
          const loud = e.m === state.fin.hover || (narrowed && e.m >= ra && e.m <= rb);
          e.loud = loud;
          tx = cx; ta = e.m >= ra && e.m <= rb ? (loud ? (e.kind === 1 ? 0.95 : 0.8) : (e.kind === 1 ? 0.5 : 0.4)) : 0.1;
        } else { tx = e.fx; ty = e.fy; th = 2; ta = 0; }
        const p = morphT(e.id, now);
        e.sx = lerp(e.fx, tx, p); e.sy = lerp(e.fy, ty, p); e.h = lerp(e.fh, th, p); e.a = lerp(e.fa, ta, p);
        if (e.a < 0.01) continue;
        const w = lerp(2, Math.min(14, G.colW * 0.14), p);
        const fin = (scr === 'finance' ? p : 0) * (e.loud ? 1 : 0.55); // field light -> green / coral; quiet months stay half-saturated
        c.fillStyle = e.kind === 1 ? `rgba(${lerp(232, 124, fin) | 0},${lerp(230, 240, fin) | 0},${lerp(225, 169, fin) | 0},${e.a})` : `rgba(${lerp(232, 255, fin) | 0},${lerp(230, 92, fin) | 0},${lerp(225, 122, fin) | 0},${e.a})`;
        c.fillRect(e.sx - w / 2, e.sy, w, Math.max(e.h - 1.5, 0.6)); // 1.5px surface gap between entries
      }
      // soft column glow
      if (scr === 'finance') {
        const p = sstep(0.4, 1, (now - state.morph0) / MORPH);
        for (let m = 0; m < months.length; m++) {
          const cx = G.r.left + G.colW * (m + 0.5), on = m >= ra && m <= rb ? 1 : 0.25;
          const mi = Math.round(+F.months[m].income * 100) * G.scale, me = Math.round(+F.months[m].expense * 100) * G.scale;
          const gi = c.createLinearGradient(cx - 20, 0, cx + 20, 0); gi.addColorStop(0, 'rgba(124,240,169,0)'); gi.addColorStop(0.5, `rgba(124,240,169,${0.06 * p * on})`); gi.addColorStop(1, 'rgba(124,240,169,0)');
          c.fillStyle = gi; c.fillRect(cx - 20, G.axis - mi, 40, mi);
          const ge = c.createLinearGradient(cx - 20, 0, cx + 20, 0); ge.addColorStop(0, 'rgba(255,92,122,0)'); ge.addColorStop(0.5, `rgba(255,92,122,${0.05 * p * on})`); ge.addColorStop(1, 'rgba(255,92,122,0)');
          c.fillStyle = ge; c.fillRect(cx - 20, G.axis, 40, me);
        }
        c.globalCompositeOperation = 'source-over';
        c.fillStyle = `rgba(232,230,225,${0.25 * p})`; c.fillRect(G.r.left, G.axis, G.r.width, 1);
        // category light bars, seen through glass
        c.globalCompositeOperation = 'lighter';
        for (const [x, y, w, h, share, inc] of G.bars) {
          if (y > VH || y + h < 0) continue;
          const bw = Math.max(w * share * p * (1 / Math.max(...G.bars.filter((q) => q[5] === inc).map((q) => q[4]))), 1.5);
          // sequential, one hue: share sets the lightness (coral for expense, green for income)
          const rel = share / Math.max(...G.bars.filter((q) => q[5] === inc).map((q) => q[4]));
          const k = 0.35 + 0.65 * Math.sqrt(rel);
          c.fillStyle = inc ? `rgba(${124 * k | 0},${240 * k | 0},${169 * k | 0},0.95)` : `rgba(${255 * k | 0},${92 * k | 0},${122 * k | 0},0.95)`;
          c.fillRect(x, y + 1, bw, h - 2);
        }
        // exchanges: arcs between currency lines
        if (G.exR && G.exR.bottom > 0 && G.exR.top < VH) {
          const R = G.exR, curs = F.exCurs, yOf = (cur) => R.top + 14 + curs.indexOf(cur) * 52;
          c.globalCompositeOperation = 'source-over';
          c.strokeStyle = 'rgba(232,230,225,0.14)'; c.lineWidth = 1;
          for (const cu of curs) { c.beginPath(); c.moveTo(R.left + 40, yOf(cu)); c.lineTo(R.right, yOf(cu)); c.stroke(); }
          c.globalCompositeOperation = 'lighter';
          const span = months.length;
          for (const e of F.exView || []) {
            if (curs.indexOf(e.cur) < 0 || curs.indexOf(e.ccur) < 0) continue;
            const day = +e.date.slice(8, 10) / 31;
            const x = R.left + 48 + ((e.m + day) / span) * (R.width - 56);
            const y0 = yOf(e.cur), y1 = yOf(e.ccur);
            const bulge = 10 + Math.log10(1 + e.usd / 100) * 12;
            c.strokeStyle = `rgba(92,225,230,${0.35 + 0.5 * p})`; c.lineWidth = 1.2;
            c.beginPath(); c.moveTo(x, y0); c.bezierCurveTo(x + bulge, y0, x + bulge, y1, x, y1); c.stroke();
            c.fillStyle = 'rgba(160,245,248,0.95)'; c.fillRect(x - 1, y1 - 1, 2, 2);
          }
        }
        c.globalCompositeOperation = 'source-over';
      }
      c.globalCompositeOperation = 'source-over';
    }

    // ---- loader: one stroke of light draws the mark
    if (L) {
      const mx = markXf();
      const drawP = L.fetchP, fade = 1 - sstep(0, 1, L.parseP);
      if (fade > 0) {
        c.save(); c.translate(mx.cx, mx.cy); c.scale(mx.s, mx.s); c.translate(-MK.c[0], -MK.c[1]);
        const upto = drawP * strokes.total;
        const pathTo = () => { c.beginPath(); for (const s of strokes) { if (s.len[0] > upto && s === strokes[0]) break; let started = false; for (let i = 0; i < s.length; i++) { if (s.len[i] > upto) { if (i > 0 && started) { const t = (upto - s.len[i - 1]) / (s.len[i] - s.len[i - 1]); c.lineTo(lerp(s[i - 1][0], s[i][0], t), lerp(s[i - 1][1], s[i][1], t)); } break; } if (!started) { c.moveTo(s[i][0], s[i][1]); started = true; } else c.lineTo(s[i][0], s[i][1]); } } };
        c.globalCompositeOperation = 'lighter';
        c.lineCap = 'round'; c.lineJoin = 'round';
        pathTo(); c.strokeStyle = `rgba(232,230,225,${0.12 * fade})`; c.lineWidth = 9 / mx.s; c.stroke();
        c.strokeStyle = `rgba(232,230,225,${0.95 * fade})`; c.lineWidth = 1.6 / mx.s; c.stroke();
        c.restore();
        if (drawP < 1) { const [hx, hy] = markPoint(drawP); const x = mx.cx + (hx - MK.c[0]) * mx.s, y = mx.cy + (hy - MK.c[1]) * mx.s; const g = c.createRadialGradient(x, y, 0, x, y, 16); g.addColorStop(0, 'rgba(255,255,255,1)'); g.addColorStop(1, 'rgba(255,255,255,0)'); c.fillStyle = g; c.fillRect(x - 16, y - 16, 32, 32); }
        c.globalCompositeOperation = 'source-over';
      }
    }

    // ---- composite through glass
    if (glass) {
      let obj = { x: 0, y: 0, z: 0, size: 1, extent: 1.1, morph: 1, flat: 1, presence: 0, thick: 0.05, inv: I3 };
      if (L && !REDUCE) {
        const mx = markXf();
        const pres = sstep(0.7, 1, L.fetchP) * (1 - sstep(0.82, 1, L.layoutP));
        const morph = quintic(sstep(0.42, 0.72, L.layoutP));
        const flat = 1 - 0.96 * sstep(0.7, 0.95, L.layoutP);
        obj = { x: mx.cx, y: mx.cy, z: 0.3 * mx.s * MK.R, size: mx.s * MK.R, extent: 1.12, morph, flat, presence: pres, thick: 0.045, inv: I3, aa: true };
      }
      const graphTex = buildGraph(now, scr, L);
      glass.frame({ scene, slabs: collectSlabs(), obj, vw: VW, vh: VH, ratio: DPR, graphTex });
    }
    if (params.has('fps') && $('#fps')) { const avg = fpsAcc.reduce((x, y) => x + y, 0) / fpsAcc.length; $('#fps').textContent = `${(1 / avg).toFixed(0)} fps · ${(avg * 1000).toFixed(1)} мс/кадр`; }
    window.__frames = (window.__frames || 0) + 1;
    requestAnimationFrame(frame);
  }

  // ------------------------------------------------------------------ input
  const pick = (x, y) => {
    let best = null, bd = 14 * 14;
    for (const n of nodes) { if (n.a < 0.1) continue; const d = (n.sx - x) ** 2 + (n.sy - y) ** 2; if (d < bd) { bd = d; best = n.i; } }
    return best;
  };
  let dragC = null;
  glCanvas.addEventListener('pointerdown', (e) => { if (state.screen !== 'memory' || state.loader) return; dragC = { x: e.clientX, y: e.clientY, cx: cam.x, cy: cam.y, moved: false }; glCanvas.setPointerCapture(e.pointerId); });
  const onMove = (e) => {
    mouse.x = e.clientX; mouse.y = e.clientY;
    if (dragC) { const dx = e.clientX - dragC.x, dy = e.clientY - dragC.y; if (Math.abs(dx) + Math.abs(dy) > 3) dragC.moved = true; cam.x = dragC.cx - dx / cam.k; cam.y = dragC.cy - dy / cam.k; camFly = null; return; }
    if (state.screen !== 'memory' || state.loader) return;
    if (state.chain) { const h2 = pick(e.clientX, e.clientY); if (h2 != null) tip(e, nodes[h2].t.replace('_', ' '), stripProj(nodes[h2].label)); else tip(null); glCanvas.style.cursor = h2 != null ? 'pointer' : 'default'; state.hover = null; return; }
    const far = cam.k / fitK < 1.6; // at overview the unit of attention is the project, not the neuron
    const h = far ? null : pick(e.clientX, e.clientY);
    if (h !== state.hover) { state.hover = h; }
    let hpG = null;
    if (h == null && cam.k / fitK < 3) { gangliaGeom(); for (const G of ganglia.values()) { const [cx, cy] = project({ x: G.cx, y: G.cy }); if (Math.hypot(e.clientX - cx, e.clientY - cy) < G.rr * cam.k * 1.05) { hpG = G; break; } } }
    const hpK = hpG ? hpG.key : null;
    if (hpK !== state.hoverProj) { state.hoverProj = hpK; showCard(hpG); }
    if (hpG) { const cdd = $('#m-card'); cdd.style.transform = `translate(${Math.min(e.clientX + 18, VW - 280)}px, ${Math.min(e.clientY + 18, VH - 260)}px)`; }
    if (h != null) tip(e, nodes[h].t.replace('_', ' ') + ' · ' + (GA.conf[nodes[h].c] || 'без оценки'), stripProj(nodes[h].label)); else tip(null);
    glCanvas.style.cursor = h != null ? 'pointer' : 'grab';
  };
  glCanvas.addEventListener('pointermove', onMove);
  glCanvas.addEventListener('pointerup', (e) => { const d = dragC; dragC = null; if (d && !d.moved) { const h = cam.k / fitK < 1.6 ? null : pick(e.clientX, e.clientY); if (h == null && state.hoverProj != null) { flyToGanglion(ganglia.get(state.hoverProj)); return; } select(h); } });
  glCanvas.addEventListener('pointerleave', () => { state.hover = null; state.hoverProj = null; showCard(null); tip(null); });
  glCanvas.addEventListener('wheel', (e) => {
    if (state.screen !== 'memory') return; e.preventDefault(); camFly = null;
    const f = Math.exp(-e.deltaY * 0.0015), wx = cam.x + (e.clientX - cam.ox) / cam.k, wy = cam.y + (e.clientY - cam.oy) / cam.k;
    cam.k *= f; cam.x = wx - (e.clientX - cam.ox) / cam.k; cam.y = wy - (e.clientY - cam.oy) / cam.k;
  }, { passive: false });

  $$('#rail [data-go]').forEach((b) => (b.onclick = () => setScreen(b.dataset.go)));
  $('#replay').onclick = () => startLoader();
  const cmdIn = $('#cmd-in'), res = $('#cmd-results');
  let hits = [], hi = 0;
  cmdIn.addEventListener('input', () => {
    const q = cmdIn.value.trim().toLowerCase();
    hits = q.length < 2 ? [] : nodes.filter((n) => n.label.toLowerCase().includes(q)).sort((a, b) => b.deg - a.deg).slice(0, 7);
    hi = 0;
    res.innerHTML = hits.map((n, k) => `<button data-n="${n.i}" class="${k === 0 ? 'sel' : ''}"><span class="t">${n.t.replace('_', ' ')}</span><span class="l">${esc(stripProj(n.label))}</span></button>`).join('') || (q.length >= 2 ? '<button disabled><span class="l">ничего — в фазе 2 поиск идёт через /api/search по всей памяти</span></button>' : '');
    res.classList.toggle('on', q.length >= 2);
  });
  res.addEventListener('click', (e) => { const b = e.target.closest('[data-n]'); if (b) go(+b.dataset.n); });
  function go(i) { cmdIn.value = ''; res.classList.remove('on'); cmdIn.blur(); setScreen('memory'); select(i); }
  cmdIn.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && hits[hi]) go(hits[hi].i);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { hi = clamp(hi + (e.key === 'ArrowDown' ? 1 : -1), 0, hits.length - 1); $$('button', res).forEach((b, k) => b.classList.toggle('sel', k === hi)); e.preventDefault(); }
    if (e.key === 'Escape') { cmdIn.value = ''; res.classList.remove('on'); cmdIn.blur(); }
  });
  window.addEventListener('keydown', (e) => {
    if (e.target.closest('input, select, textarea')) return;
    const k = e.key;
    if (k === '1') setScreen('memory'); else if (k === '2') setScreen('tasks'); else if (k === '3') setScreen('finance'); else if (k === '0') setScreen('spec');
    else if (k === '/') { e.preventDefault(); $('#cmd').classList.remove('away'); cmdIn.focus(); }
    else if (k === ' ' && state.screen === 'memory') { e.preventDefault(); togglePlay(); }
    else if ((k === 'c' || k === 'C' || k === 'с' || k === 'С') && state.screen === 'memory' && (state.hover != null || state.sel != null)) enterChain(state.hover != null ? state.hover : state.sel);
    else if (k === 'r' || k === 'R' || k === 'к' || k === 'К') startLoader();
    else if (k === 'Escape') { if (state.chain || state.sel != null) select(null); else if (state.screen !== 'memory') setScreen('memory'); }
  });
  window.addEventListener('resize', resize);
  // on screens with scrolling content the command line stays out of the way and returns on "/"
  cmdIn.addEventListener('focus', () => $('#cmd').classList.remove('away'));
  cmdIn.addEventListener('blur', () => $('#cmd').classList.add('away'));

  // ------------------------------------------------------------------ spec
  function buildSpec() {
    const sw = [['--ink', '#070A12', 'поле, сине-чёрное'], ['--g2', '#131A26', 'глубина'], ['--g3', '#26303F', 'линии'], ['--g4', '#5D6878', 'тихий текст'], ['--bone', '#E8E6E1', 'текст'], ['знания', '#5CE1E6', 'decision, concept, solution, doc, skill'], ['работа', '#FFB547', 'task, work_log, session, reminder'], ['проблемы', '#FF5C7A', 'problem, open / blocked'], ['факты владельца', '#7CF0A9', 'user_fact, person'], ['инфраструктура', '#8FA3B8', 'file, dependency, server, project'], ['доход', '#7CF0A9', 'финансы: вверх'], ['расход', '#FF5C7A', 'финансы: вниз'], ['обмен', '#5CE1E6', 'финансы: дуги'], ['fringe', '', 'кайма дисперсии стекла']];
    const gl = ['decision', 'problem', 'solution', 'concept', 'task', 'session', 'work_log', 'user_fact', 'project', 'doc', 'skill', 'file'];
    $('#spec').innerHTML = `
<h1>Свет и стекло: спецификация интерфейса aurelius</h1>
<p class="lede">Память — нейронная сеть в темноте: узлы — нейроны, связи — аксоны, по ним бегут импульсы. Цвет — биолюминесценция по смыслу: пять групп, не оттенок на тип. Стекло преломляет этот свет. Минимализм — мало элементов и никакого шума, а не чёрно-белое; бесцветен только знак.</p>

<h2>Знак</h2>
<div class="marks"><figure><img class="inv" src="logo-mark.svg" alt=""><figcaption>logo-mark.svg — линия, currentColor</figcaption></figure><figure><img class="inv" src="logo-mark-solid.svg" alt=""><figcaption>logo-mark-solid.svg — крупные размеры</figcaption></figure><figure><img class="inv lock" src="logo.svg" alt=""><figcaption>logo.svg — знак и надпись</figcaption></figure></div>
<p>Силуэт, кольца и шесть узлов сети обведены из ui/public/logo.png (маска золота, potrace); центры узлов замерены, рёбра сети повторяют оригинал. Цвета у знака нет, только currentColor. Надпись — Geist, обведена в кривые.</p>

<h2>Правила «Aurum · свет и стекло»</h2>
<ol>
<li><b>Палитра.</b> Фон — глубокие сине-чёрные чернила #070A12 с холодной виньеткой. Пять смысловых групп: знания #5CE1E6, работа #FFB547, проблемы #FF5C7A, факты владельца #7CF0A9, инфраструктура #8FA3B8. Раздел задаёт свой тон (--sec): Память — циан, Задачи — янтарь, Финансы — пара доход/расход; пилюля, линия под заголовком и фокус-кольца берут его и перетекают при смене раздела. Текст всегда кость и серые: цвет несёт смысл, а не украшает.</li>
<li><b>Граф.</b> Нейрон — маленькая яркая сома с мягким ореолом, размер по степени, яркость по свежести и уверенности. Связь — изогнутый сужающийся аксон с градиентом от цвета источника к цвету цели и синапсом у цели; belongs_to не рисуется. Проект — ганглий: органическое облако, свежее в ядре, старое на краю. Тип — форма глифа внутри группы, глифы проявляются при приближении; на общем плане яркие знаки только у активных задач и открытых проблем. Насыщенность следует вниманию: плотное обесцвечивается к серо-тонированному свету, фокус и активное — полный цвет.</li>
<li><b>Фокус.</b> Наведение высвечивает соседей, остальное падает до 15% («тишина вокруг мысли»), по связям бежит импульс света. Клик раскрывает соседей орбитой, затем они оседают в раскладку.</li>
<li><b>Хром.</b> Почти нет: полноэкранное поле, тонкая левая колонка разделов, одна командная строка снизу («/»). Клавиши 1/2/3 — разделы, Esc — назад, пробел — проиграть память, C — цепочка, R — повтор загрузки, 0 — эта спека.</li>
<li><b>Переходы — морф тех же точек, не смена страниц.</b> Память → Задачи: точки задач остаются и скользят в дорожки статусов, остальное гаснет. → Финансы: поле остывает, точки перестраиваются на оси времени; месяцы — вертикальные столбы света из записей. С prefers-reduced-motion — простое затухание 160 мс.</li>
<li><b>Git задачи.</b> Ветка — тонкая вертикаль с засечками коммитов; открытый PR — линия, загибающаяся к main; влитый — сомкнутая с main.</li>
<li><b>Финансы, «гроссбух в темноте».</b> Итог периода огромным моноширинным табличным кеглем под тонкой стеклянной планкой; доля категорий — горизонтальные полосы света, видимые сквозь стекло; обмены — дуги между линиями валют, отдельно от расхода и дохода; отменённые записи зачёркнуты пеплом, причина по наведению; период выбирается ползунком по шкале месяцев.</li>
<li><b>Загрузка честными стадиями, без склейки.</b> fetch — знак рисуется одним штрихом света (длина штриха = доля полученных байт), знак становится стеклом; parse — штрих распадается на частицы, счётчик узлов в моно; layout — частицы летят в свои настоящие позиции раскладки, стекло плавится в плоскую линзу и растворяется — загрузчик становится графом.</li>
<li><b>Шрифты.</b> Один гротеск (Geist) и Geist Mono для чисел, id, улик и таблиц. Шкала 11 / 13 / 15 / 20 / 32 / 64; 64 — только итоги финансов. Подписи 11px капителью, трекинг +8%.</li>
<li><b>Фактура.</b> Плёночное зерно ~3%, свечение только на точках, туман глубины; никакого backdrop-blur.</li>
</ol>

<h3>Стекло</h3>
<ul>
<li><b>Объект (raymarch, FluidGlass).</b> Только знак в загрузке: SDF, ior 1.5, дисперсия 0.35 (RGB расходятся только на выходе луча у края), френель, студийный контровой свет. Луч марширует только внутри ограничивающей сферы, снаружи пиксель берётся из первого прохода как есть. Морф формы 0.85 с, quintic smootherstep.</li>
<li><b>Линза снята</b> по решению Влада: лишний элемент. Её место заняло воспроизведение времени.</li>
<li><b>Плиты (панели узла, задачи, фильтров финансов, командная строка).</b> Дёшево, в экранном пространстве: скруглённый прямоугольник с заранее заданным профилем высоты — плоский центр, скос по краю. Смещение выборки сцены вдоль градиента SDF только на скосе, R/G/B смещаются по-разному — отсюда кайма. Контровой свет сверху-слева, без размытия (frost 0), без raymarch на пиксель.</li>
<li><b>Пилюля раздела.</b> Маленькая стеклянная пилюля в левой колонке, скользит пружиной и вытягивается вдоль скорости.</li>
<li><b>Текст поверх стекла не преломляется:</b> стекло гнёт только поле, текст читается всегда.</li>
</ul>

<h2>Цепочки, корень и время</h2>
<p><b>Цепочки смысла.</b> Связи показываются не соседством, а цепочками рассуждения: направленное дерево, которое сходится в один корень. Отображение отношений aurelius_core::Relation (кто на ком висит):</p>
<table class="spec"><thead><tr><th>отношение</th><th>направление</th><th>смысл в цепочке</th></tr></thead><tbody>
<tr><td class="mono">contains</td><td>цель висит на источнике</td><td>журнал, решение, проблема принадлежат задаче</td></tr>
<tr><td class="mono">subtask_of, implements, blocks, refines, next_step</td><td>источник висит на цели</td><td>подзадача — задаче, реализация — решению, уточнение — исходному факту</td></tr>
<tr><td class="mono">solves</td><td>решение висит на проблеме</td><td>problem ← solution</td></tr>
<tr><td class="mono">supersedes</td><td>старый факт висит на новом</td><td>линия замены; старый тускнеет, когда рождается новый</td></tr>
<tr><td class="mono">caused_by, depends_on, prerequisite, verified_by</td><td>причина / зависимость / улика висит на следствии</td><td>почему → что</td></tr>
<tr><td class="mono">belongs_to</td><td>только членство</td><td>задачи, решения, сессии без цепочки висят на проекте; ребро не рисуется</td></tr>
<tr><td class="mono">related_to, references, inspired_by, learned_from, defines, configures, tracked_by, uses</td><td>контекст</td><td>в покое скрыт, при фокусе виден весь</td></tr>
</tbody></table>
<p><b>Сходится в одно.</b> Корень — параметр ?root=now (по умолчанию: радиус — ранг времени от «сейчас», проект — угловой сектор) или ?root=owner (проекты вокруг одной сомы, внутри — радиальные деревья цепочек). Импульсы по умолчанию бегут внутрь, к корню; при фокусе волна идёт наружу и показывает, что выросло из узла. Проблемы, решения и журналы без пути к корню — «без связей»: пыль на краю проекта, их число в карточке проекта.</p>
<p><b>В покое</b> видны только цепочечные рёбра, без журналов и сессий, сильнейшие первыми, не больше ~850 на экране, яркость ∝ 1/√(число видимых). <b>Фокус</b> показывает все связи узла, остальное — 10%. <b>Цепочка</b> (клик или C): путь выпрямляется — вверх по течению слева, узел в центре, вниз до корня справа, боковые ветви короткими отростками; импульс проходит шаг за шагом; Esc возвращает узлы тем же морфом.</p>
<p><b>Время.</b> Шкала внизу Памяти, ручка — «сейчас». Потяни назад — память такой, какой была: узлы появляются в момент создания со вспышкой цвета группы, рёбра растут, когда оба конца существуют, заменённые факты тускнеют. Пробел — рост от первой записи до сегодня за 20 с, время в логарифме, чтобы плотные последние недели не проскакивали. Видимость, вспышка и затухание считаются в шейдере из одного uniform uTime; метки больших дней берутся из релизов и задач того дня.</p>
<h2>Палитра</h2>
<div class="sw">${sw.map(([n, h, d]) => `<div>${n === 'fringe' ? '<i class="fringe"></i>' : `<i style="background:${h}"></i>`}<span>${n}</span><span>${h || 'только стекло'}</span><span>${d}</span></div>`).join('')}</div>
<p style="margin-top:14px">Проверка dataviz (validate_palette.js, фон #070A12): пять групп проходят CVD-разделение (худшая пара коралл↔янтарь ΔE 15.2 при дейтеранопии) и контраст ≥3:1; яркость выше полосы для заливок — это свет на тёмном, не заливка столбцов, а сталь намеренно малонасыщенна. Пара доход/расход проваливает дейтеранопию (ΔE 1.5), поэтому у неё обязательное второе кодирование: доход вверх и ярче, расход вниз и темнее, знак у суммы. Текст: кость 100% / 74% / 56%; пепел #7F8796 — отменённое.</p>

<h2>Шрифт</h2>
<div class="scale">
<div><span class="mono">64 mono</span><span class="mono" style="font-size:64px;line-height:1">−24 335,00</span></div>
<div><span class="mono">32</span><span style="font-size:32px">Свет и стекло</span></div>
<div><span class="mono">20</span><span style="font-size:20px">Курсы валют и цельные сводки финансов</span></div>
<div><span class="mono">15</span><span style="font-size:15px">Claim узла и вводные абзацы</span></div>
<div><span class="mono">13</span><span>Основной текст интерфейса, строки списков и таблиц</span></div>
<div><span class="mono">11 caps</span><span class="cap">Подписи, заголовки дорожек, +8%</span></div>
<div><span class="mono">12 mono</span><span class="mono" style="font-size:12px">node verify-run.mjs "cargo test" → exit 0 · 2276059</span></div>
</div>
<p style="margin-top:10px">Одна насыщенность (Geist Regular): иерархия держится размером и яркостью, не жирностью. Кириллица в моно падает на JetBrains Mono — в фазе 2 нужен Geist Mono с кириллицей или JetBrains Mono целиком.</p>

<h2>Иконки</h2>
<p>Своя система, без библиотек: сетка 24, живая зона 16, линия 1.5, круглые концы и стыки, открытые линии. Каждая иконка читается на 12 px и сводится к простому силуэту среднего плана (ромб, треугольник, круг, квадрат), поэтому LOD непрерывен: далеко — точка, средний план — силуэт, близко — иконка из SDF-атласа в том же инстанс-проходе. В графе и легенде иконка берёт цвет группы, в тексте — кость. Файлы: icons.svg (спрайт), icons/*.svg, сборка tools/build_icons.py.</p>
<div id="icon-sheet"></div>
<h2>Кодирование графа</h2>
<div class="glyphs" id="spec-glyphs"></div>
<p style="margin-top:12px">Цвет — группа, форма — точный тип. Размер — степень узла (log). Свежее 7 дней — яркое ядро ганглия. На общем плане листья — пыль 1–1.5 px, без глифов; глифы проявляются плавно после ×2.2 масштаба.</p>

<h2>Раскладка экрана</h2>
<div class="layoutmap">
<div style="left:8px;top:8px;bottom:8px;width:90px">колонка разделов 112px · пилюля · знак</div>
<div style="left:112px;right:8px;top:8px;bottom:8px;box-shadow:none;color:var(--g4)">поле на весь экран (граф / дорожки / столбы)</div>
<div style="right:16px;top:16px;bottom:48px;width:28%">стеклянная плита 420–430px: узел или задача</div>
<div style="left:38%;width:30%;bottom:12px;height:28px">командная строка, «/»</div>
<div style="left:120px;bottom:12px;width:150px;height:60px">легенда: форма и свет</div>
</div>

<h2>Движение</h2>
<table class="spec"><thead><tr><th>что</th><th>длительность</th><th>кривая</th><th>как</th></tr></thead><tbody>
<tr><td>наведение, нажатие</td><td class="mono">120 мс</td><td class="mono">out-expo</td><td>цвет текста, подложка строки</td></tr>
<tr><td>смена содержимого плиты</td><td class="mono">220 мс</td><td class="mono">out-expo</td><td>opacity; плита не прыгает размером</td></tr>
<tr><td>вход раздела, панели, ступени</td><td class="mono">740 мс (0.45 с видимо)</td><td class="mono">linear() пружина, bounce 0</td><td>opacity + clip-path + translateY 10px; кривая сгенерирована по модели пружины Motion (visualDuration 0.45)</td></tr>
<tr><td>смена раздела</td><td class="mono">740 мс вход / 290 мс выход</td><td class="mono">пружина / (.5,0,.75,0)</td><td>DOM: clip-path reveal; поле: морф точек; тон раздела перетекает через @property --sec</td></tr><tr><td>импульсы</td><td class="mono">1.1–1.8 с фон, 0.52–0.78 с активация</td><td class="mono">линейно по кривой</td><td>фон — несколько в секунду по экрану; наведение — волна от узла: хоп 1, затем хоп 2 и 3</td></tr>
<tr><td>морф точек</td><td class="mono">900 мс + разброс до 280 мс</td><td class="mono">out-expo</td><td>позиция, яркость, высота сегмента</td></tr>
<tr><td>камера (перелёт к узлу)</td><td class="mono">640–720 мс</td><td class="mono">in-out cubic</td><td>центр и масштаб (масштаб — в логарифме)</td></tr>
<tr><td>орбита соседей</td><td class="mono">420 мс раскрытие, 760 мс оседание</td><td class="mono">out-expo / in-out</td><td>соседи на кольце, затем в раскладку</td></tr>
<tr><td>пилюля раздела</td><td class="mono">пружина</td><td class="mono">ω 10–16, ζ 0.86; растяжение ω 13, ζ 0.32</td><td>позиция и растяжение вдоль скорости</td></tr>
<tr><td>морф стекла</td><td class="mono">850 мс</td><td class="mono">quintic smootherstep</td><td>знак → линза → плоскость (только загрузка)</td></tr>
<tr><td>загрузка</td><td class="mono">5–8 с, сколько идёт на самом деле</td><td class="mono">по прогрессу, не по таймеру</td><td>штрих = байты, частицы = разбор, полёт = тики раскладки</td></tr>
</tbody></table>
<h3>Никогда не анимируется</h3>
<ul><li>Денежные числа: никаких отсчётов «от нуля», итог меняется сразу и целиком.</li><li>Таблицы и порядок строк, перенос текста, ширина колонок.</li><li>Кодирование графа: форма и яркость не «играют» ради эффекта.</li><li>Прокрутка: никакого перехвата и параллакса страницы.</li><li>Ввод не блокируется дольше 200 мс; любой переход прерывается следующим.</li></ul>
<h3>prefers-reduced-motion</h3>
<p>Переходы — затухание 160 мс; точки встают на места сразу; воспроизведение времени не играет, а прыгает; орбита выключена; загрузка показывает знак и полосы стадий без полёта; зерно стоит.</p>

<h2>Навигация</h2>
<p>Память / Задачи / Финансы — левая колонка и клавиши 1/2/3; адрес #memory, #tasks, #finance. Esc: снять выделение → вернуться в Память. «/» — командная строка: поиск по меткам памяти, Enter открывает узел. Клик по связи в плите переводит фокус на соседний узел; перелёт камеры показывает, куда ушли.</p>

<h2>Компоненты</h2>
<table class="spec"><tbody>
<tr><td>Поле</td><td>2D-сцена точек (в фазе 2 — инстансы three.js), режимы memory / tasks / finance / dim</td></tr>
<tr><td>Глиф узла</td><td>форма × уверенность, спрайт; кольцо фокуса; импульс по ребру</td></tr>
<tr><td>Стеклянная плита</td><td>pill, pillbar, slab, slab-clear, bar: радиус, скос, сила преломления, поглощение, дисперсия, контровой свет</td></tr>
<tr><td>Шкала времени</td><td>ползунок «сейчас», пробел — рост памяти за 20 с, метки больших дней</td></tr><tr><td>Цепочка</td><td>клик или C: путь смысла выпрямляется, вверх по течению слева, к корню справа</td></tr>
<tr><td>Плита узла</td><td>тип, метка, claim, заметка, происхождение (уверенность, тема, изменчивость, даты, источник), улики моно, связи по отношениям</td></tr>
<tr><td>Легенда</td><td>фильтр по типу (сессии, навыки, документы — те же кнопки), шкала уверенности</td></tr>
<tr><td>Дорожки задач</td><td>бэклог / в работе / блок / готово; строка: точка, заголовок в 2 строки, приоритет столбиками, критерии n/m, ветка</td></tr>
<tr><td>Плита задачи</td><td>статус, приоритет, id; заведена → взята → закрыта; git-рельс; критерии; подзадачи; журнал; решения</td></tr>
<tr><td>Гроссбух</td><td>столбы из записей, стеклянная планка с итогом, ползунок периода</td></tr>
<tr><td>Категории</td><td>полосы света под стеклом, сумма, доля</td></tr>
<tr><td>Обмены</td><td>линии валют и дуги; таблица валют без пересчёта; источники курса</td></tr>
<tr><td>Записи</td><td>фильтры на стекле: вид, категория, поиск, отменённые; строки с курсом и источником</td></tr>
<tr><td>Загрузчик</td><td>знак-штрих, стадии fetch / parse / layout с честными числами</td></tr>
</tbody></table>
<p>Напоминания и навыки (reminder_list, skill_list) в фазе 2 приходят в ту же командную строку: «/напомни», «/навыки» показывают список в плите; отдельного раздела им не нужно.</p>

<h2>Фаза 2: как это ляжет на three.js WebGPU</h2>
<ul>
<li>three.js r18x (context7, /mrdoob/three.js): RenderPipeline + pass(scene, camera) даёт текстуру сцены; нейроны — InstancedMesh или Sprite-инстансы, аксоны — один инстанс-буфер полос с кривой в вершинном TSL-шейдере (как EDGE_VS в graph.js), импульсы — те же полосы с окном [t−0.2, t]. Свечение — bloom(scenePass, strength, radius, threshold) с высоким порогом и малым радиусом; тон — renderer.toneMapping = NeutralToneMapping или своя Fn по яркости с сохранением оттенка, как graphAt() в glass.js. Стекло — Fn-узел поверх текстуры сцены: плиты ≤10 прямоугольников, объект — ограниченный raymarch.</li>
<li>Глифы — атлас-текстура 18 форм × 5 уровней света, атрибуты инстанса: тип, уверенность, возраст, степень. Морф — два буфера позиций (from/to) и униформа времени, смешивание в вершинном шейдере; разброс — hash(id).</li>
<li>Загрузка: fetch через ReadableStream с Content-Length → длина штриха; JSON.parse в воркере с отчётом по чанкам → счётчик; тики раскладки уже в воркере → полёт частиц.</li>
<li>Замер прототипа (1600×1000, RTX 5060 через ANGLE GL, 3 507 узлов, 4 374 аксона): 56–61 fps на общем плане, среднем и крупном планах, с волной активации и при воспроизведении времени. Оценка для 33k узлов и 21k аксонов: ~0.6 млн вершин полос и 33k квадов — укладывается в кадр на WebGPU, если позиции жить в storage-буфере, а не загружаться из JS каждый кадр; плиты ≈0.3 мс, линза ≈0.5–1 мс.</li>
</ul>
`;
    const SHEET = [['типы узлов', Object.keys(IC.icons).filter((k) => !/^(st|cf|fin|cat)-/.test(k)), (k) => GHEX[GROUP[k] || (k === 'module' ? 'i' : 'i')], (k) => k.replace('_', ' ')],
      ['статусы задач', Object.keys(IC.icons).filter((k) => k.startsWith('st-')), (k) => ({ 'st-active': GHEX.w, 'st-blocked': GHEX.p, 'st-done': GHEX.o, 'st-backlog': GHEX.i, 'st-cancelled': '#4f5b6b' })[k], (k) => ({ 'st-backlog': 'бэклог', 'st-active': 'в работе', 'st-blocked': 'блок', 'st-done': 'готово', 'st-cancelled': 'отменена' })[k]],
      ['уверенность', Object.keys(IC.icons).filter((k) => k.startsWith('cf-')), () => '#e8e6e1', (k) => k.slice(3)],
      ['финансы', Object.keys(IC.icons).filter((k) => k.startsWith('fin-')), (k) => ({ 'fin-expense': 'var(--exp)', 'fin-income': 'var(--inc)', 'fin-exchange': 'var(--exc)' })[k], (k) => ({ 'fin-expense': 'расход', 'fin-income': 'доход', 'fin-exchange': 'обмен' })[k]],
      ['категории', Object.keys(IC.icons).filter((k) => k.startsWith('cat-')), () => '#e8e6e1', (k) => Object.entries(IC.category).find(([, v]) => v === k)?.[0] || k]];
    $('#icon-sheet').innerHTML = SHEET.map(([title, keys, col, lab]) => `<h3>${title} · ${keys.length}</h3><div class="isheet">${keys.map((k) => `<figure style="color:${col(k)}">${ico(k, 's24')}${ico(k, 's16')}${ico(k, 's12')}<figcaption>${esc(lab(k))}</figcaption></figure>`).join('')}</div>`).join('');
    const G = $('#spec-glyphs');
    for (const t of gl) { const s = document.createElement('span'); s.append(glyphCanvas(t, 1, 18, GHEX[GROUP[t] || 'i'])); s.append(document.createTextNode(t.replace('_', ' '))); G.append(s); }
    const C = document.createElement('div'); C.className = 'glyphs'; C.style.marginTop = '12px';
    for (const [ci, n] of [[1, 'measured — резкая точка'], [2, 'inferred — мягкий ореол'], [3, 'reported — тусклая'], [4, 'stale — полое кольцо']]) { const s = document.createElement('span'); const cv = document.createElement('canvas'); cv.width = cv.height = 36; cv.style.width = cv.style.height = '18px'; cv.getContext('2d').drawImage(sprite('concept', ci).cv, 0, 0, 36, 36); s.append(cv, document.createTextNode(n)); C.append(s); }
    G.after(C);
  }

  // ------------------------------------------------------------------ boot
  buildLegend(); statsLine(); buildLabels(); buildTimeline(); buildLanes(); showTask(); buildFinance(); buildSpec();
  resize();
  document.fonts.ready.then(() => { if (state.screen === 'finance') renderFin(); });
  if (params.has('skip')) {
    newSim(); stepSim(TICKS); fitCamera(true);
    nodes.forEach((n) => { const [x, y] = project(n); n.sx = n.bx = x; n.sy = n.by = y; n.a = n.ba = 0.8; });
    $('#loader').style.display = 'none'; $('#rail').style.opacity = 1; $('#cmd').style.opacity = 1;
    setScreen(initialScreen(), true);
    if (params.get('focus') === 'expand') select(GA.expand, true);
  } else startLoader();
  if (params.has('task')) { state.task = params.get('task'); showTask(); }
  requestAnimationFrame(frame);
  window.__au = { state, setScreen, select, nodes, cam, startLoader, enterChain, exitChain, togglePlay, setReplay: (v) => { state.replay.s = v; state.replay.playing = false; }, chainOf, ganglia, gangliaGeom, flyToGanglion, fitK: () => fitK, project, pulses, adj };
})();
