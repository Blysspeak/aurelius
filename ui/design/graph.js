// The neural field: nodes as somas, edges as tapered curved axons, pulses as action potentials.
// Two kinds of draws, all instanced, drawn additively into a float light buffer and tone-mapped on
// luminance (hue kept) inside the glass composite:
//   field   — every node, every molecule edge and its synapse, uploaded ONCE in world coordinates.
//             Camera, zoom, replay time, focus, hover project, type filter, chain and screen fades
//             are uniforms; per-node focus/session/overlay bits live in a R8UI texture that is
//             re-uploaded only when they change. No per-frame CPU work grows with the graph.
//   overlay — the few dynamic things built per frame in screen space: spines, axons to the root,
//             pulses, focus arcs, the chain path, task points.
// Runs inside the render worker (render.js); AU_GRAPH(self) installs GraphGL.
function AU_GRAPH(G) {
  const SEG = 18;               // segments per curve
  const EDGE_F = 20;            // floats per overlay edge/pulse instance
  const NODE_F = 16;            // floats per overlay node instance
  const FNODE_F = 24;           // floats per field node
  const FEDGE_F = 20;           // floats per field edge

  const COMMON = `
uniform vec2 uView;
uniform vec2 uTime;                 // now of the replay (days), fade window (days)
float outExpo(float t) { return t >= 1.0 ? 1.0 : 1.0 - exp2(-10.0 * t); }
vec2 toClip(vec2 p) { return vec2(p.x / uView.x * 2.0 - 1.0, 1.0 - p.y / uView.y * 2.0); }
`;
  // one axon: quadratic curve, tapered strip, gradient source -> target, grows in once both ends exist
  const AXON = `
out vec3 vCol;
out float vSide;
out float vW;
const float SEG = ${SEG}.0;
void axon(vec2 P0, vec2 P2, float bend, float w0, float t0, float t1, vec3 c0, float a0, vec3 c1, float a1, float birth) {
  int i = gl_VertexID / 2;
  float side = float(gl_VertexID % 2) * 2.0 - 1.0;
  float u = float(i) / SEG;
  float t = mix(t0, t1, u);
  vec2 d = P2 - P0;
  float L = max(length(d), 1e-3);
  vec2 n = vec2(-d.y, d.x) / L;
  vec2 C = (P0 + P2) * 0.5 + n * bend * L;
  float s = 1.0 - t;
  vec2 pos = s * s * P0 + 2.0 * s * t * C + t * t * P2;
  vec2 tg = 2.0 * s * (C - P0) + 2.0 * t * (P2 - C);
  vec2 nn = normalize(vec2(-tg.y, tg.x) + 1e-6);
  float core = mix(w0, 0.45, t);                   // tapered: thick at the source, hairline at the target
  float w = core + 2.4;                            // room for the soft glow
  pos += nn * side * w * 0.5;
  gl_Position = vec4(toClip(pos), 0.0, 1.0);
  float a = mix(a0, a1, u) * smoothstep(birth, birth + uTime.y, uTime.x);
  vCol = mix(c0, c1, t) * a;
  vSide = side;
  vW = core / w;
}
void cull() { gl_Position = vec4(2.0, 2.0, 2.0, 1.0); vCol = vec3(0.0); vSide = 0.0; vW = 0.0; }
`;
  // one soma: a quad with replay visibility, landing flash and superseded dimming, all on the GPU
  const SOMA = `
out vec2 vP;
out vec4 vCol;
out vec4 vS;
out float vPx;
out float vExt;
void soma(vec2 at, float size, float ext, vec3 c, float I, float halo, float shape, float glyph, float icon, float birth, float sup, float flashOn) {
  vec2 q = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1)) * 2.0 - 1.0;
  float age = uTime.x - birth;
  float vis = smoothstep(0.0, uTime.y, age);
  float flash = flashOn * step(0.0, age) * exp(-age / max(uTime.y * 2.5, 1e-4));
  float dim = uTime.x >= sup ? 0.35 : 1.0;
  vec2 pos = at + q * size * ext * (1.0 + flash * 1.4);
  gl_Position = vec4(toClip(pos), 0.0, 1.0);
  vP = q * ext;
  vCol = vec4(c, I * vis * dim * (1.0 + 3.0 * flash)); vS = vec4(halo, shape, glyph, icon); vPx = 1.0 / max(size, 0.5); vExt = ext;
}
void cull() { gl_Position = vec4(2.0, 2.0, 2.0, 1.0); vP = vec2(0.0); vCol = vec4(0.0); vS = vec4(0.0); vPx = 1.0; vExt = 1.0; }
`;
  // the field's shared state: camera, zoom, selection, modes, the per-node flag texture
  const FIELD = `
precision highp usampler2D;
uniform vec4 uCam;    // cam x, cam y, k, origin x
uniform vec4 uCam2;   // origin y, flash on (replay), unused, unused
uniform vec4 uZoom;   // zoom ratio, glyph mix, icon mix, saturation
uniform vec4 uLoad;   // loader on, parse progress, layout progress, unused
uniform vec4 uMark;   // mark centre x, y, scale, unused
uniform vec4 uSel;    // focus on, focus id, hovered project (-1), type filter (-1)
uniform vec4 uMode;   // session highlight on, spec screen, ms since the morph started, unused
uniform vec4 uChain;  // chain amount from, to; screen alpha from, to (staggered by the morph)
uniform vec4 uEdge;   // edge base alpha, budget gain, importance threshold, unused
uniform usampler2D uFlags; // bit 1 focus, 2 near the focus, 4 in the hovered session, 8 drawn by the overlay
uint dyn(float id) { int i = int(id + 0.5); return texelFetch(uFlags, ivec2(i & 1023, i >> 10), 0).r; }
float morph(float h) { return outExpo(clamp((uMode.z - h * 280.0) / 900.0, 0.0, 1.0)); }
vec2 world(vec2 w) { return vec2(uCam.w + (w.x - uCam.x) * uCam.z, uCam2.x + (w.y - uCam.y) * uCam.z); }
vec3 colOf(vec3 c, float sat) { float L = dot(c, vec3(0.2126, 0.7152, 0.0722)); return mix(vec3(L * 0.86, L * 0.93, L * 1.05), c, sat); }
bool bit(float f, float b) { return mod(floor(f / b), 2.0) > 0.5; }
`;

  const EDGE_VS = `#version 300 es
precision highp float;
layout(location=0) in vec4 aP;      // p0.xy, p1.xy (css px)
layout(location=1) in vec4 aB;      // bend, w0, t0, t1
layout(location=2) in vec4 aC0;     // source rgb, a0
layout(location=3) in vec4 aC1;     // target rgb, a1
layout(location=4) in vec4 aT;      // birth (days from now, <= 0), unused
${COMMON}${AXON}
void main() { axon(aP.xy, aP.zw, aB.x, aB.y, aB.z, aB.w, aC0.rgb, aC0.a, aC1.rgb, aC1.a, aT.x); }`;

  // molecule edges of the whole graph, in world coordinates
  const FEDGE_VS = `#version 300 es
precision highp float;
layout(location=0) in vec4 aP;      // A world xy, B world xy
layout(location=1) in vec4 aB;      // bend, log2(1 + leaves of the child), importance, birth
layout(location=2) in vec4 aC0;     // A rgb, A id
layout(location=3) in vec4 aC1;     // B rgb, B id
layout(location=4) in vec4 aH;      // A project, A rest alpha, B rest alpha, hash
${COMMON}${FIELD}${AXON}
float edgeAlpha(out float w, out float sat) {
  float lw = aB.y;
  float al = uEdge.x * uEdge.y * min(min(aH.y, aH.z), 1.0) * (0.45 + 0.25 * min(lw, 4.0));
  w = 0.7 + 0.55 * min(lw, 5.0); sat = uZoom.w;
  if (uSel.x > 0.5) { if (abs(aC0.w - uSel.y) < 0.5 || abs(aC1.w - uSel.y) < 0.5) { al = 0.9; w += 0.4; sat = 1.0; } else al *= 0.1; }
  if (uSel.z >= 0.0 && abs(aH.x - uSel.z) > 0.5) al *= 0.22;
  if (uMode.x > 0.5) al *= ((dyn(aC0.w) & 4u) != 0u && (dyn(aC1.w) & 4u) != 0u) ? 1.0 : 0.12;
  float m = morph(aH.w);
  al *= (1.0 - mix(uChain.x, uChain.y, m)) * mix(uChain.z, uChain.w, m);
  return al;
}
void main() {
  bool inc = uSel.x > 0.5 && (abs(aC0.w - uSel.y) < 0.5 || abs(aC1.w - uSel.y) < 0.5);
  if ((aB.z < uEdge.z && !inc) || uEdge.x <= 0.0) { cull(); return; }   // strongest first: the importance threshold keeps ~850 on screen
  float w, sat; float al = edgeAlpha(w, sat);
  if (al < 0.004) { cull(); return; }
  vec2 P0 = world(aP.xy), P2 = world(aP.zw);
  if (max(P0.x, P2.x) < -80.0 || min(P0.x, P2.x) > uView.x + 80.0 || max(P0.y, P2.y) < -80.0 || min(P0.y, P2.y) > uView.y + 80.0) { cull(); return; }
  axon(P0, P2, aB.x, w, 0.0, 1.0, colOf(aC0.rgb, sat), al * 0.55, colOf(aC1.rgb, sat), al, aB.w);
}`;

  const EDGE_FS = `#version 300 es
precision highp float;
in vec3 vCol;
in float vSide;
in float vW;
uniform float uGain;
out vec4 o;
void main() {
  float x = abs(vSide);
  float coreA = 1.0 - smoothstep(vW * 0.5, vW * 0.5 + 0.18, x);
  float glow = exp(-x * x * 6.0) * 0.35;
  o = vec4(vCol * (coreA + glow) * uGain, 1.0);
}`;

  const NODE_VS = `#version 300 es
precision highp float;
layout(location=0) in vec4 aA;   // x, y, size(px radius), extent multiplier
layout(location=1) in vec4 aCol; // rgb, intensity
layout(location=2) in vec4 aS;   // halo, shape, glyph mix (0..1 silhouette, 1..2 icon), icon index
layout(location=3) in vec4 aT;   // birth, superseded-at, flash on/off, unused
${COMMON}${SOMA}
void main() { soma(aA.xy, aA.z, aA.w, aCol.rgb, aCol.a, aS.x, aS.y, aS.z, aS.w, aT.x, aT.y, aT.z); }`;

  // every node of the graph: the per-node logic of the old buildGraph(), now on the GPU
  const FNODE_VS = `#version 300 es
precision highp float;
layout(location=0) in vec4 aW;   // world x, y, mark point x, y (mark coords)
layout(location=1) in vec4 aC;   // rgb, confidence intensity
layout(location=2) in vec4 aP;   // size base (degree), recency, density coeff of its project, loader order mu
layout(location=3) in vec4 aF;   // type, project, birth, superseded-at
layout(location=4) in vec4 aK;   // shape, icon, flags (1 attn, 2 project, 4 theme, 8 dust/pool, 16 leaf, 32 session), age dim
layout(location=5) in vec4 aX;   // log2(1 + theme count), hash, id, rest alpha (1 measured, 0.85 else)
${COMMON}${FIELD}${SOMA}
void main() {
  uint d = dyn(aX.z);
  float fl = aK.z;
  bool attn = bit(fl, 1.0), proj = bit(fl, 2.0), virt = bit(fl, 4.0), dust = bit(fl, 8.0), leaf = bit(fl, 16.0), sess = bit(fl, 32.0);
  bool isF = (d & 1u) != 0u, near = (d & 2u) != 0u;
  if ((d & 8u) != 0u || sess) { cull(); return; }
  vec2 scr = world(aW.xy);
  float mA = aX.w;
  bool loader = uLoad.x > 0.5;
  float a;
  if (loader) {                                  // the loader: from the stroke of the mark to the layout
    vec2 mp = uMark.xy + (aW.zw - vec2(512.0, 511.5)) * uMark.z;
    float born = uLoad.y > aP.w ? 1.0 : 0.0;
    vec2 drift = uLoad.y * 22.0 * (vec2(aX.y, fract(aX.y * 91.7 + 0.37)) - 0.5);
    float p = outExpo(clamp((uLoad.z - aP.w * 0.35) / 0.65, 0.0, 1.0));
    scr = mix(mp + drift, scr, p);
    a = born * mix(0.28, mA, p);
  } else {
    a = mA * (1.0 - 0.4 * aK.w);                 // older = dimmer
    if (uMode.x > 0.5) a = (d & 4u) != 0u ? 1.3 : 0.12 * mA;
    else {
      if (uSel.z >= 0.0 && abs(aF.y - uSel.z) > 0.5) a *= 0.22;
      if (uSel.w >= 0.0 && abs(aF.x - uSel.w) > 0.5) a *= 0.08;
      if (uSel.x > 0.5) a *= (isF || near) ? 1.25 : 0.15;   // silence around the thought
    }
    float m = morph(aX.y);
    a = mix(a, 0.08 * mA, mix(uChain.x, uChain.y, m));      // chain: everything else falls to ~8%
    a *= mix(uChain.z, uChain.w, m);                        // screen fade: tasks/finance 0, spec 0.07
    a = clamp(a, 0.0, 1.3);
  }
  if (a <= 0.01) { cull(); return; }
  if (scr.x < -60.0 || scr.x > uView.x + 60.0 || scr.y < -60.0 || scr.y > uView.y + 60.0) { cull(); return; }
  float zr = uZoom.x;
  float I = loader ? a * 1.7 * aC.w : a * aP.y * aC.w * clamp(uCam.z * aP.z, 0.2, 1.0);
  float sat = uZoom.w;
  float size = aP.x * clamp(pow(zr, 0.45), 0.85, 2.8);
  float ext = 3.2, halo = 2.0, glyph = uZoom.y;
  if (loader) { size = mix(1.7, size, smoothstep(0.2, 1.0, uLoad.z)); glyph = 0.0; halo = 1.6; }
  else if (leaf && zr < 1.8) { size = 1.25; ext = 2.2; halo = 0.7; I *= 0.85; }   // dust at far zoom
  if (attn && !loader) { size = max(size, 4.2); glyph = 1.0; I = max(I, 1.5 * a); sat = 1.0; halo = 2.6; }
  if (proj) { size = max(size, 4.6); I *= 1.2; }
  if (uSel.z >= 0.0 && abs(aF.y - uSel.z) < 0.5) { I *= 1.35; sat = max(sat, 0.9); }
  if (isF) { size *= 1.7; I = 2.6; sat = 1.0; glyph = max(glyph, 0.6); } else if (near) { I *= 1.5; sat = 1.0; }
  if (uMode.y > 0.5) I *= 0.5;
  if (dust && !isF && !near) { I *= 0.35; size = min(size, 1.3); glyph = 0.0; halo = 0.6; }
  if (virt) { size = 3.2 + aX.x * 0.5; I = max(I, 1.1) * min(a, 1.0); glyph = 1.0; }
  float g2 = glyph;
  if (uZoom.z > 0.0 && aK.y >= 0.0 && !loader) { g2 = glyph + uZoom.z; size = mix(size, max(size, 9.0), uZoom.z); ext = max(ext, 2.4); } // far dot -> mid silhouette -> close icon
  if (I <= 0.002) { cull(); return; }
  soma(scr, size, ext, colOf(aC.rgb, sat), I, halo, aK.x, g2, aK.y, aF.z, aF.w, uCam2.y);
}`;

  // the synapse at the end of each visible molecule edge (close zoom)
  const FSYN_VS = `#version 300 es
precision highp float;
layout(location=0) in vec4 aP;
layout(location=1) in vec4 aB;
layout(location=2) in vec4 aC0;
layout(location=3) in vec4 aC1;
layout(location=4) in vec4 aH;
${COMMON}${FIELD}${SOMA}
void main() {
  bool inc = uSel.x > 0.5 && (abs(aC0.w - uSel.y) < 0.5 || abs(aC1.w - uSel.y) < 0.5);
  if (uZoom.x <= 2.2 || (aB.z < uEdge.z && !inc) || uEdge.x <= 0.0) { cull(); return; }
  float lw = aB.y;
  float al = uEdge.x * uEdge.y * min(min(aH.y, aH.z), 1.0) * (0.45 + 0.25 * min(lw, 4.0));
  float sat = uZoom.w;
  if (uSel.x > 0.5) { if (abs(aC0.w - uSel.y) < 0.5 || abs(aC1.w - uSel.y) < 0.5) { al = 0.9; sat = 1.0; } else al *= 0.1; }
  if (uSel.z >= 0.0 && abs(aH.x - uSel.z) > 0.5) al *= 0.22;
  float m = morph(aH.w);
  al *= (1.0 - mix(uChain.x, uChain.y, m)) * mix(uChain.z, uChain.w, m);
  if (al <= 0.08) { cull(); return; }
  vec2 P0 = world(aP.xy), P2 = world(aP.zw);
  vec2 dd = P2 - P0; float L = max(length(dd), 1e-3);
  vec2 C = (P0 + P2) * 0.5 + vec2(-dd.y, dd.x) / L * aB.x * L;
  float t = 0.965, s = 1.0 - t;
  vec2 pos = s * s * P0 + 2.0 * s * t * C + t * t * P2;
  if (pos.x < -20.0 || pos.x > uView.x + 20.0 || pos.y < -20.0 || pos.y > uView.y + 20.0) { cull(); return; }
  soma(pos, 1.5, 2.2, colOf(aC1.rgb, sat), al * 1.5, 0.6, 0.0, 0.0, -1.0, aB.w, 1e9, 0.0);
}`;

  const NODE_FS = `#version 300 es
precision highp float;
in vec2 vP;
in vec4 vCol;
in vec4 vS;
in float vPx;
in float vExt;
uniform sampler2D uAtlas;
uniform vec3 uGrid; // cols, rows, atlas px per icon cell
uniform float uGain;
out vec4 o;
float sdShape(vec2 p, float s) {
  vec2 a = abs(p);
  if (s < 0.5) return length(p) - 0.9;                                   // circle
  if (s < 1.5) return (a.x + a.y * 0.8) - 1.0;                          // diamond
  if (s < 2.5) { p.y = -p.y + 0.15; return max(abs(p.x) * 0.87 + p.y * 0.5, -p.y) - 0.62; } // triangle
  if (s < 3.5) return max(a.x, a.y) - 0.8;                              // square
  if (s < 4.5) return min(max(a.x - 1.0, a.y - 0.26), max(a.y - 1.0, a.x - 0.26)); // plus
  if (s < 5.5) return length(p) - 0.55;                                  // dot
  if (s < 6.5) return min(abs(length(p) - 0.82) - 0.14, length(p) - 0.28); // ring with core
  if (s < 7.5) { float an = atan(p.y, p.x); float r = 0.75 + 0.35 * cos(an * 4.0); return length(p) - r; } // star
  if (s < 8.5) return min(abs(length(p) - 0.95) - 0.1, abs(length(p) - 0.55) - 0.1); // double ring
  return max(a.x - 1.0, a.y - 0.28);                                     // bar
}
void main() {
  float d = length(vP);
  float soma = exp(-d * d * 5.0) * 1.4 * (1.0 - smoothstep(vExt * 0.6, vExt, d));                    // bright small body
  float halo = exp(-d * d / max(vS.x * vS.x, 0.01)) * 0.22 * (1.0 - smoothstep(vExt * 0.55, vExt, d)); // soft halo, no quad edge
  float g = 1.0 - smoothstep(-vPx, vPx, sdShape(vP, vS.y));
  float body = mix(soma, g * 1.15 + soma * 0.25, clamp(vS.z, 0.0, 1.0));
  float im = clamp(vS.z - 1.0, 0.0, 1.0);
  if (im > 0.0 && vS.w >= 0.0) {               // close zoom: the full line icon from the SDF atlas
    vec2 iu = vP / 3.0 + 0.5;                    // the 24-unit icon cell spans vP in [-1.5, 1.5]
    float ia = 0.0;
    if (iu.x > 0.0 && iu.y > 0.0 && iu.x < 1.0 && iu.y < 1.0) {
      vec2 cell = vec2(mod(vS.w, uGrid.x), floor(vS.w / uGrid.x));
      float dd = texture(uAtlas, (cell + iu) / uGrid.xy).r;
      float aa = 0.75 * uGrid.z / 3.0 * vPx;     // one screen pixel in atlas pixels
      ia = 1.0 - smoothstep(-aa, aa, dd);
    }
    body = mix(body, ia * 1.25 + soma * 0.12, im);
  }
  o = vec4(vCol.rgb * vCol.a * (body + halo) * uGain, 1.0);
}`;

  function sh(gl, type, src) { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); return s; }
  function prog(gl, vs, fs) { const p = gl.createProgram(); const a = sh(gl, gl.VERTEX_SHADER, vs), b = sh(gl, gl.FRAGMENT_SHADER, fs); gl.attachShader(p, a); gl.attachShader(p, b); gl.linkProgram(p); p.__fs = b; p.__vs = a; return p; } // parallel compile: checked in ready()
  const UNIF = ['uView', 'uTime', 'uGain', 'uAtlas', 'uGrid', 'uCam', 'uCam2', 'uZoom', 'uLoad', 'uMark', 'uSel', 'uMode', 'uChain', 'uEdge', 'uFlags'];

  G.GraphGL = {
    init(gl, want) {
      // light buffer: f32 (RGBA32F + float blending) > f16 (RGBA16F) > u8 (RGBA8, pre-scaled accumulation)
      want = (want || '').toLowerCase();
      const cbf = gl.getExtension('EXT_color_buffer_float'), fblend = gl.getExtension('EXT_float_blend');
      const cbh = gl.getExtension('EXT_color_buffer_half_float');
      let light = cbf && fblend ? 'f32' : cbf || cbh ? 'f16' : 'u8';
      if (want === 'f16' && (cbf || cbh)) light = 'f16';
      if (want === 'u8') light = 'u8';
      if (want === 'f32' && !(cbf && fblend)) light = cbf || cbh ? 'f16' : 'u8';
      const FMT = { f32: [gl.RGBA32F, gl.FLOAT], f16: [gl.RGBA16F, gl.HALF_FLOAT], u8: [gl.RGBA8, gl.UNSIGNED_BYTE] }[light];
      const hdr = light !== 'u8';
      const P = { e: prog(gl, EDGE_VS, EDGE_FS), n: prog(gl, NODE_VS, NODE_FS), fe: prog(gl, FEDGE_VS, EDGE_FS), fn: prog(gl, FNODE_VS, NODE_FS), fs: prog(gl, FSYN_VS, NODE_FS) };
      const par = gl.getExtension('KHR_parallel_shader_compile');
      let isReady = false;
      const U = {};
      const mkVao = (layout, stride, usage) => {
        const vao = gl.createVertexArray(), buf = gl.createBuffer();
        gl.bindVertexArray(vao); gl.bindBuffer(gl.ARRAY_BUFFER, buf);
        let off = 0;
        layout.forEach((n, loc) => { gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, n, gl.FLOAT, false, stride * 4, off * 4); gl.vertexAttribDivisor(loc, 1); off += n; });
        gl.bindVertexArray(null);
        return { vao, buf, cap: 0, usage };
      };
      const E = mkVao([4, 4, 4, 4, 4], EDGE_F), N = mkVao([4, 4, 4, 4], NODE_F);
      const FE = mkVao([4, 4, 4, 4, 4], FEDGE_F), FN = mkVao([4, 4, 4, 4, 4, 4], FNODE_F);
      let nFE = 0, nFN = 0;
      const flagTex = gl.createTexture(); let flagRows = 1;
      gl.activeTexture(gl.TEXTURE4); gl.bindTexture(gl.TEXTURE_2D, flagTex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8UI, 1024, 1, 0, gl.RED_INTEGER, gl.UNSIGNED_BYTE, new Uint8Array(1024));
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      const ready = () => {
        if (isReady) return true;
        const ps = Object.values(P);
        if (par && !ps.every((p) => gl.getProgramParameter(p, par.COMPLETION_STATUS_KHR))) return false;
        for (const p of ps) if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getShaderInfoLog(p.__vs) + gl.getShaderInfoLog(p.__fs) + gl.getProgramInfoLog(p));
        for (const [k, p] of Object.entries(P)) { U[k] = {}; for (const u of UNIF) U[k][u] = gl.getUniformLocation(p, u); }
        isReady = true; return true;
      };
      const atlas = gl.createTexture(); let grid = [1, 1, 64];
      gl.activeTexture(gl.TEXTURE3); gl.bindTexture(gl.TEXTURE_2D, atlas); gl.texImage2D(gl.TEXTURE_2D, 0, gl.R16F, 1, 1, 0, gl.RED, gl.FLOAT, new Float32Array([100]));
      const tex = gl.createTexture(), fb = gl.createFramebuffer();
      let w = 0, h = 0;
      const upload = (V, data, n) => { // grow-only buffer; no reallocation per frame
        gl.bindBuffer(gl.ARRAY_BUFFER, V.buf);
        if (data.length > V.cap) { V.cap = Math.max(data.length, V.cap * 2); gl.bufferData(gl.ARRAY_BUFFER, V.cap * 4, gl.DYNAMIC_DRAW); }
        if (n) gl.bufferSubData(gl.ARRAY_BUFFER, 0, data, 0, n);
      };
      const setU = (u, F) => { // field uniforms from the frame's float block
        gl.uniform4f(u.uCam, F[0], F[1], F[2], F[3]); gl.uniform4f(u.uCam2, F[4], F[5], F[6], F[7]);
        gl.uniform4f(u.uZoom, F[8], F[9], F[10], F[11]); gl.uniform4f(u.uLoad, F[12], F[13], F[14], F[15]);
        gl.uniform4f(u.uMark, F[16], F[17], F[18], F[19]); gl.uniform4f(u.uSel, F[20], F[21], F[22], F[23]);
        gl.uniform4f(u.uMode, F[24], F[25], F[26], F[27]); gl.uniform4f(u.uChain, F[28], F[29], F[30], F[31]);
        gl.uniform4f(u.uEdge, F[32], F[33], F[34], F[35]); gl.uniform1i(u.uFlags, 4);
      };
      return {
        hdr, light, tex, EDGE_F, NODE_F, FNODE_F, FEDGE_F, ready,
        setAtlas(data, aw, ah, cols, rows, cell) {
          gl.activeTexture(gl.TEXTURE3); gl.bindTexture(gl.TEXTURE_2D, atlas);
          gl.texImage2D(gl.TEXTURE_2D, 0, gl.R16F, aw, ah, 0, gl.RED, gl.FLOAT, data);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
          grid = [cols, rows, cell];
        },
        setField(nodes, nNodes, edges, nEdges) { // once per layout: the whole graph in world coordinates
          gl.bindBuffer(gl.ARRAY_BUFFER, FN.buf); gl.bufferData(gl.ARRAY_BUFFER, nodes.subarray(0, nNodes * FNODE_F), gl.STATIC_DRAW);
          gl.bindBuffer(gl.ARRAY_BUFFER, FE.buf); gl.bufferData(gl.ARRAY_BUFFER, edges.subarray(0, nEdges * FEDGE_F), gl.STATIC_DRAW);
          nFN = nNodes; nFE = nEdges;
        },
        setFlags(bytes) { // 1024 per row; only when focus / session / overlay membership changes
          const rows = Math.max(1, Math.ceil(bytes.length / 1024));
          gl.activeTexture(gl.TEXTURE4); gl.bindTexture(gl.TEXTURE_2D, flagTex);
          if (rows !== flagRows) { gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8UI, 1024, rows, 0, gl.RED_INTEGER, gl.UNSIGNED_BYTE, null); flagRows = rows; }
          gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
          gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, 1024, rows, gl.RED_INTEGER, gl.UNSIGNED_BYTE, bytes.length === rows * 1024 ? bytes : (() => { const b = new Uint8Array(rows * 1024); b.set(bytes); return b; })());
        },
        draw(f) {
          if (!ready()) return null;
          const W = Math.round(f.vw * f.ratio), H = Math.round(f.vh * f.ratio);
          if (W !== w || H !== h) {
            gl.activeTexture(gl.TEXTURE5); gl.bindTexture(gl.TEXTURE_2D, tex);
            gl.texImage2D(gl.TEXTURE_2D, 0, FMT[0], W, H, 0, gl.RGBA, FMT[1], null);
            // RGBA32F + LINEAR samples as black on ANGLE/NVIDIA even with OES_texture_float_linear (the molecule
            // regression of f59ffa4): the buffer is sampled 1:1, so NEAREST is exact and always complete
            const filt = light === 'f32' ? gl.NEAREST : gl.LINEAR;
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filt); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filt);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
            gl.bindTexture(gl.TEXTURE_2D, null);
            gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
            gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
            w = W; h = H;
          }
          gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
          gl.viewport(0, 0, W, H);
          gl.clearColor(0, 0, 0, 1); gl.clear(gl.COLOR_BUFFER_BIT);
          gl.enable(gl.BLEND); gl.blendFunc(gl.ONE, gl.ONE);
          gl.activeTexture(gl.TEXTURE3); gl.bindTexture(gl.TEXTURE_2D, atlas);
          gl.activeTexture(gl.TEXTURE4); gl.bindTexture(gl.TEXTURE_2D, flagTex);
          const base = (u) => { gl.uniform2f(u.uView, f.vw, f.vh); gl.uniform2f(u.uTime, f.tNow || 0, f.tFade || 0.001); gl.uniform1f(u.uGain, f.gain || 1); };
          const atl = (u) => { gl.uniform1i(u.uAtlas, 3); gl.uniform3f(u.uGrid, grid[0], grid[1], grid[2]); };
          if (nFE && f.field) {
            gl.useProgram(P.fe); base(U.fe); setU(U.fe, f.field);
            gl.bindVertexArray(FE.vao); gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, (SEG + 1) * 2, nFE);
          }
          if (f.nEdges) {
            gl.useProgram(P.e); base(U.e);
            gl.bindVertexArray(E.vao); upload(E, f.edges, f.nEdges * EDGE_F);
            gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, (SEG + 1) * 2, f.nEdges);
          }
          if (nFN && f.field) {
            gl.useProgram(P.fn); base(U.fn); setU(U.fn, f.field); atl(U.fn);
            gl.bindVertexArray(FN.vao); gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, nFN);
            if (f.field[8] > 2.2 && nFE) { gl.useProgram(P.fs); base(U.fs); setU(U.fs, f.field); atl(U.fs); gl.bindVertexArray(FE.vao); gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, nFE); }
          }
          if (f.nNodes) {
            gl.useProgram(P.n); base(U.n); atl(U.n);
            gl.bindVertexArray(N.vao); upload(N, f.nodes, f.nNodes * NODE_F);
            gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, f.nNodes);
          }
          gl.disable(gl.BLEND);
          gl.bindVertexArray(null);
          return tex;
        },
      };
    },
  };
}
if (typeof window !== 'undefined') AU_GRAPH(window);
