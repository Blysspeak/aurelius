// The analytic layout (project -> theme -> molecules, time along each branch), as one pure function.
// It runs in a Worker built from its own source (works from file:// too), so the main thread never
// pays for it; the result is a set of typed arrays, cached in IndexedDB by graph hash.
// Input: typed arrays of the graph. Output: positions and the chain tree, see the return statement.
function AU_LAYOUT(I) {
  'use strict';
  const { N, typ, age, theme, ea, eb, er, TYPES, RELS, projects, themes } = I;
  const hash = (i) => { const x = Math.sin(i * 127.1 + 311.7) * 43758.5453; return x - Math.floor(x); };
  const GROUP = { decision: 'k', concept: 'k', solution: 'k', doc: 'k', skill: 'k', digest: 'k', task: 'w', work_log: 'w', session: 'w', reminder: 'w', problem: 'p', user_fact: 'o', person: 'o', file: 'i', dependency: 'i', server: 'i', crate: 'i', project: 'i', module: 'i', config: 'i' };
  const E = ea.length;
  const progress = I.progress || (() => {});
  const nodes = new Array(N);
  for (let i = 0; i < N; i++) { const t = TYPES[typ[i]]; nodes[i] = { i, t, g: GROUP[t] || 'i', age: age[i], theme: theme[i], session: t === 'session', virtual: false, hub: -1 }; }
  const relName = RELS;
  const adjS = new Int32Array(N + 1);
  for (let k = 0; k < E; k++) { adjS[ea[k] + 1]++; adjS[eb[k] + 1]++; }
  for (let i = 0; i < N; i++) adjS[i + 1] += adjS[i];
  const adjN = new Int32Array(2 * E), adjR = new Uint8Array(2 * E), fill = adjS.slice(0, N);
  for (let k = 0; k < E; k++) { const a = ea[k], b = eb[k]; adjN[fill[a]] = b; adjR[fill[a]++] = er[k]; adjN[fill[b]] = a; adjR[fill[b]++] = er[k]; }
  const BEL = relName.indexOf('belongs_to');
  const deg = new Int32Array(N), nd = new Int32Array(N);
  for (let k = 0; k < E; k++) { deg[ea[k]]++; deg[eb[k]]++; if (er[k] !== BEL) { nd[ea[k]]++; nd[eb[k]]++; } }
  progress(0.1);
  // ---- membership: belongs_to a top project, else inherit from a non-membership neighbour (3 passes)
  const projSet = new Set(projects);
  const hubOf = new Int32Array(N).fill(-1);
  for (let k = 0; k < E; k++) {
    if (er[k] !== BEL) continue;
    if (projSet.has(eb[k])) hubOf[ea[k]] = eb[k]; else if (projSet.has(ea[k])) hubOf[eb[k]] = ea[k];
  }
  for (const p of projects) hubOf[p] = p;
  for (let pass = 0; pass < 3; pass++) for (let i = 0; i < N; i++) if (hubOf[i] < 0) { for (let q = adjS[i]; q < adjS[i + 1]; q++) { const j = adjN[q]; if (adjR[q] !== BEL && hubOf[j] >= 0) { hubOf[i] = hubOf[j]; break; } } }
  const OTHER = -2;
  for (let i = 0; i < N; i++) { if (hubOf[i] < 0) hubOf[i] = OTHER; nodes[i].hub = hubOf[i]; }
  const ganglia = new Map();
  for (const n of nodes) { if (!ganglia.has(n.hub)) ganglia.set(n.hub, { key: n.hub, members: [], x: 0, y: 0 }); ganglia.get(n.hub).members.push(n); }
  for (const G of ganglia.values()) {
    G.members.sort((a, b) => a.age - b.age);
    G.R = 6.4 * Math.sqrt(G.members.length) + 14;
    const cnt = { k: 0, w: 0, p: 0, o: 0, i: 0 }; G.members.forEach((m) => cnt[m.g]++); G.dom = Object.entries(cnt).sort((x, y) => y[1] - x[1])[0][0];
  }
  progress(0.25);
  // ---- theme nodes (virtual): one per theme with members
  const themeNodes = new Map();
  const byTheme = new Map();
  for (const n of nodes) if (n.theme >= 0 && !n.session) { if (!byTheme.has(n.theme)) byTheme.set(n.theme, []); byTheme.get(n.theme).push(n); }
  themes.forEach(([plabel, name, count], ti) => {
    const mem = byTheme.get(ti);
    if (!mem || !mem.length) return;
    const cnt = { k: 0, w: 0, p: 0, o: 0, i: 0 }; let amax = -Infinity; for (const m of mem) { cnt[m.g]++; if (m.age > amax) amax = m.age; }
    const tn = { i: nodes.length, t: 'theme', age: amax, g: Object.entries(cnt).sort((a, b) => b[1] - a[1])[0][0], hub: mem[0].hub, virtual: true, count, ti, session: false };
    nodes.push(tn); themeNodes.set(ti, tn);
    const G0 = ganglia.get(tn.hub); if (G0) G0.members.push(tn);
  });
  const NT = nodes.length;
  const ROOT = -10;
  const MOL = { subtask_of: 'fwd', contains: 'rev', solves: 'fwd', refines: 'fwd', supersedes: 'fwd', implements: 'fwd', blocks: 'fwd', caused_by: 'rev', depends_on: 'rev', prerequisite: 'rev', next_step: 'fwd', verified_by: 'rev' };
  const MOL_ORDER = Object.keys(MOL);
  const PREL = [...MOL_ORDER, 'theme', 'dust', 'belongs_to'];
  const parent = new Int32Array(NT).fill(-1), prel = new Array(NT).fill(null);
  const wouldCycle = (c, p) => { for (let q = p, k = 0; q >= 0 && k < 80; q = parent[q], k++) if (q === c) return true; return false; };
  const byRel = new Map(); for (let k = 0; k < E; k++) { const r = relName[er[k]]; if (MOL[r]) { if (!byRel.has(r)) byRel.set(r, []); byRel.get(r).push(k); } }
  for (const rel of MOL_ORDER) for (const k of byRel.get(rel) || []) {
    const S = nodes[ea[k]], Tn = nodes[eb[k]];
    if (rel === 'contains' && S.t !== 'task' && S.t !== 'work_log') continue;
    const c = MOL[rel] === 'fwd' ? S.i : Tn.i, pp = MOL[rel] === 'fwd' ? Tn.i : S.i;
    if (parent[c] !== -1 || nodes[c].session || nodes[pp].session || nodes[c].t === 'project' || nodes[pp].t === 'project' || nodes[c].hub !== nodes[pp].hub || wouldCycle(c, pp)) continue;
    parent[c] = pp; prel[c] = rel;
  }
  const molRoot = (i) => { let q = i; for (let k = 0; k < 90; k++) { const p = parent[q]; if (p < 0) return q; q = p; } return q; };
  const molSize = new Int32Array(NT);
  for (const n of nodes) if (!n.virtual && !n.session && n.t !== 'project') { n.mol = molRoot(n.i); molSize[n.mol]++; }
  for (const n of nodes) {
    if (n.virtual) { parent[n.i] = n.hub >= 0 ? n.hub : -1; prel[n.i] = 'theme'; continue; }
    if (n.session) continue;
    if (n.t === 'project') { parent[n.i] = ROOT; prel[n.i] = 'belongs_to'; continue; }
    n.unlinked = molSize[n.mol] <= 1;
    if (parent[n.i] === -1) {
      const tn = themeNodes.get(n.theme);
      if (n.unlinked) { n.dust = !!tn; if (tn) { parent[n.i] = tn.i; prel[n.i] = 'dust'; } }
      else if (tn) { parent[n.i] = tn.i; prel[n.i] = 'theme'; }
      else if (n.hub >= 0) { parent[n.i] = n.hub; prel[n.i] = 'belongs_to'; }
    }
  }
  progress(0.45);
  const kids = nodes.map(() => []);
  for (const n of nodes) if (parent[n.i] >= 0) kids[parent[n.i]].push(n.i);
  const depthOf = (i) => { let d = 0; for (let q = i; q !== ROOT && q >= 0 && d < 90; q = parent[q]) d++; return d; };
  const leaves = new Int32Array(NT);
  const countLeaves = (i) => { let sum = kids[i].length ? 0 : 1; for (const k of kids[i]) sum += countLeaves(k); leaves[i] = sum; return sum; };
  for (const p of projects) countLeaves(p);
  const reachesRoot = (i) => { let q = i; for (let k = 0; k < 90 && q >= 0; k++) q = parent[q]; return q === ROOT; };
  for (const n of nodes) { n.orphan = n.session || !reachesRoot(n.i); n.depth = n.orphan ? 99 : depthOf(n.i); }
  const TYPE_RANK = { problem: 0, solution: 1, decision: 2, concept: 3, task: 4, work_log: 5 };
  // ---- projects around the root (d3 forceCollide + forceRadial, ported: 400 ticks over ~15 bodies)
  {
    const D = [...ganglia.values()].sort((a, b) => b.R - a.R);
    D.forEach((G, k) => { const a = k * 2.39996, r = 160 + 50 * Math.sqrt(k); G.x = Math.cos(a) * r; G.y = Math.sin(a) * r; G.vx = 0; G.vy = 0; });
    const rad = D.map((G) => G.R * 1.15 + 30), rr = D.map((G) => 90 + G.R * 1.2);
    let alpha = 1; const decay = 1 - Math.pow(0.001, 1 / 300);
    let seed = 1; const jiggle = () => ((seed = (1664525 * seed + 1013904223) % 4294967296) / 4294967296 - 0.5) * 1e-6;
    for (let t = 0; t < 400; t++) {
      alpha += (0 - alpha) * decay;
      for (let it = 0; it < 4; it++) for (let i = 0; i < D.length; i++) {
        const A = D[i], ri = rad[i], ri2 = ri * ri, xi = A.x + A.vx, yi = A.y + A.vy;
        for (let j = i + 1; j < D.length; j++) {
          const B = D[j]; let rj = rad[j]; const r = ri + rj;
          let x = xi - B.x - B.vx, y = yi - B.y - B.vy, l = x * x + y * y;
          if (l < r * r) {
            if (x === 0) { x = jiggle(); l += x * x; } if (y === 0) { y = jiggle(); l += y * y; }
            l = Math.sqrt(l); l = (r - l) / l;
            const w = (rj *= rj) / (ri2 + rj);
            A.vx += (x *= l) * w; A.vy += (y *= l) * w;
            B.vx -= x * (1 - w); B.vy -= y * (1 - w);
          }
        }
      }
      for (let i = 0; i < D.length; i++) { const G = D[i]; const dx = G.x || 1e-6, dy = G.y || 1e-6, r = Math.sqrt(dx * dx + dy * dy), k = (rr[i] - r) * 0.08 * alpha / r; G.vx += dx * k; G.vy += dy * k; }
      for (const G of D) { G.x += G.vx *= 0.6; G.y += G.vy *= 0.6; }
    }
  }
  progress(0.6);
  const ax = new Float32Array(NT).fill(NaN), ay = new Float32Array(NT);
  const pol = (cx, cy, a, r) => [cx + Math.cos(a) * r, cy + Math.sin(a) * r];
  function placeMol(i, x, y, dir, spread, step, d) {
    ax[i] = x; ay[i] = y;
    const ch = kids[i].filter((k) => !nodes[k].virtual).sort((a, b) => (TYPE_RANK[nodes[a].t] ?? 6) - (TYPE_RANK[nodes[b].t] ?? 6) || nodes[a].age - nodes[b].age);
    const m = ch.length; if (!m || d > 400) return;
    const w = Math.min(spread / m, 0.62);
    ch.forEach((k, j) => { const a = dir + (j - (m - 1) / 2) * w; const [cx, cy] = pol(x, y, a, step * (1 + 0.15 * (j % 2))); placeMol(k, cx, cy, a, Math.max(spread * 0.7, 0.5), step * 0.9, d + 1); });
  }
  const pool = new Uint8Array(NT);
  const tipX = new Float32Array(NT), tipY = new Float32Array(NT);
  const thByHub = new Map(); for (const tn of themeNodes.values()) { if (!thByHub.has(tn.hub)) thByHub.set(tn.hub, []); thByHub.get(tn.hub).push(tn); }
  const kidsOfHub = new Map(); for (const n of nodes) if (!n.virtual && !n.session && !n.unlinked && parent[n.i] === n.hub && n.hub >= 0) { if (!kidsOfHub.has(n.hub)) kidsOfHub.set(n.hub, []); kidsOfHub.get(n.hub).push(n); }
  for (const G of ganglia.values()) {
    const hub = G.key >= 0 ? nodes[G.key] : null;
    const out = Math.atan2(G.y, G.x);
    if (hub) { ax[hub.i] = G.x; ay[hub.i] = G.y; }
    const petals = (thByHub.get(G.key) || []).slice();
    const noTheme = kidsOfHub.get(G.key) || [];
    if (noTheme.length) petals.push({ pseudo: true, roots: noTheme });
    const weights = petals.map((p) => Math.pow(p.pseudo ? p.roots.length * 3 : kids[p.i].length + 1, 0.7));
    const totW = weights.reduce((a, b) => a + b, 0) || 1, maxW = Math.max(...weights, 1);
    const SPAN = Math.min(Math.PI * 1.8, 0.7 + petals.length * 0.45);
    let a = out - SPAN / 2;
    petals.forEach((p, k) => {
      const span = SPAN * weights[k] / totW, th = a + span / 2; a += span;
      const R0 = 14, Rt = G.R * (0.55 + 0.6 * Math.sqrt(weights[k] / maxW));
      if (!p.pseudo) { [ax[p.i], ay[p.i]] = pol(G.x, G.y, th, R0 * 1.5); [tipX[p.i], tipY[p.i]] = pol(G.x, G.y, th, Rt * 1.04); }
      const roots = (p.pseudo ? p.roots : kids[p.i].map((kk) => nodes[kk]).filter((n) => !n.dust)).sort((x, y) => y.age - x.age);
      const M = roots.length;
      roots.forEach((r, j) => {
        const t = (j + 0.5) / Math.max(M, 1);
        const rr2 = R0 * 2.2 + (Rt - R0 * 2.2) * t;
        const aa = th + span * 0.34 * Math.sin(j * 2.39996 + k);
        const [x, y] = pol(G.x, G.y, aa, rr2);
        placeMol(r.i, x, y, aa, 1.6, 5.5 + Math.min(6, Math.sqrt(molSize[r.i])), 0);
      });
      if (!p.pseudo) {
        const dust = kids[p.i].map((kk) => nodes[kk]).filter((n) => n.dust).sort((x, y) => y.age - x.age);
        dust.forEach((n, j) => { const [x, y] = pol(G.x, G.y, th + (hash(n.i) - 0.5) * span * 0.9, R0 * 2 + (Rt - R0 * 2) * (j + 0.5) / dust.length); ax[n.i] = x; ay[n.i] = y; });
      }
    });
    const pl = G.members.filter((m) => !m.virtual && !m.session && (m.orphan || (m.unlinked && !m.dust)) && m.t !== 'project');
    const [px, py] = hub ? pol(G.x, G.y, out + Math.PI, G.R * 0.55) : [G.x, G.y];
    pl.forEach((n, j) => { const r = 3 * Math.sqrt(j + 0.5), aa = j * 2.39996; ax[n.i] = px + Math.cos(aa) * r; ay[n.i] = py + Math.sin(aa) * r; pool[n.i] = 1; });
    for (const m of G.members) if (m.session) { ax[m.i] = G.x; ay[m.i] = G.y; }
  }
  for (let i = 0; i < NT; i++) if (Number.isNaN(ax[i])) { ax[i] = 0; ay[i] = 0; }
  progress(0.85);
  // time: a superseded fact dims when its successor is born
  const SUP = relName.indexOf('supersedes');
  const supAt = new Float32Array(NT).fill(1e9);
  for (let k = 0; k < E; k++) if (er[k] === SUP) supAt[eb[k]] = Math.min(supAt[eb[k]], -age[ea[k]]);
  const flags = new Uint8Array(NT), depth = new Uint8Array(NT), prelC = new Uint8Array(NT).fill(255), hubA = new Int32Array(NT);
  for (const n of nodes) {
    flags[n.i] = (n.dust ? 1 : 0) | (pool[n.i] ? 2 : 0) | (n.unlinked ? 4 : 0) | (n.orphan ? 8 : 0);
    depth[n.i] = Math.min(n.depth, 255); hubA[n.i] = n.hub;
    if (prel[n.i]) prelC[n.i] = PREL.indexOf(prel[n.i]);
  }
  const TN = NT - N, th = { ti: new Int32Array(TN), g: new Uint8Array(TN), age: new Float32Array(TN), count: new Int32Array(TN) };
  for (let k = 0; k < TN; k++) { const tn = nodes[N + k]; th.ti[k] = tn.ti; th.g[k] = 'kwpoi'.indexOf(tn.g); th.age[k] = tn.age; th.count[k] = tn.count; }
  const gang = [...ganglia.values()].map((G) => ({ key: G.key, x: G.x, y: G.y, R: G.R, dom: G.dom }));
  const xs = ax.slice().sort(), ys = ay.slice().sort(), qq = (arr, p) => arr[Math.floor(p * (arr.length - 1))];
  const bounds = { x0: qq(xs, 0.015), x1: qq(xs, 0.985), y0: qq(ys, 0.015), y1: qq(ys, 0.985) }; // the camera fit, robust to outliers
  progress(1);
  return { N, NT, PREL, ax, ay, tipX, tipY, parent, prel: prelC, flags, depth, leaves, hub: hubA, deg, nd, supAt, th, gang, bounds };
}
// transferables of a result, for postMessage and IndexedDB
AU_LAYOUT.transfer = (r) => [r.ax, r.ay, r.tipX, r.tipY, r.parent, r.prel, r.flags, r.depth, r.leaves, r.hub, r.deg, r.nd, r.supAt, r.th.ti, r.th.g, r.th.age, r.th.count].map((a) => a.buffer);
AU_LAYOUT.VERSION = 'L4';
