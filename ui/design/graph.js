// The neural field: nodes as somas, edges as tapered curved axons, pulses as action potentials.
// Everything is instanced and drawn additively into a half-float target, then tone-mapped
// on luminance (hue kept) inside the glass composite. No vertex buffers: geometry comes
// from gl_VertexID, per-instance data is one interleaved buffer per kind.
(function () {
  const SEG = 18;               // segments per curve
  const EDGE_F = 20;            // floats per edge/pulse instance
  const NODE_F = 16;            // floats per node instance

  const EDGE_VS = `#version 300 es
precision highp float;
layout(location=0) in vec4 aP;      // p0.xy, p1.xy (css px)
layout(location=1) in vec4 aB;      // bend, w0, t0, t1
layout(location=2) in vec4 aC0;     // source rgb, a0
layout(location=3) in vec4 aC1;     // target rgb, a1
layout(location=4) in vec4 aT;      // birth (days from now, <= 0), unused
uniform vec2 uView;
uniform vec2 uTime;                 // now of the replay (days), fade window (days)
out vec3 vCol;
out float vSide;
out float vW;
const float SEG = ${SEG}.0;
void main() {
  int i = gl_VertexID / 2;
  float side = float(gl_VertexID % 2) * 2.0 - 1.0;
  float u = float(i) / SEG;
  float t = mix(aB.z, aB.w, u);
  vec2 P0 = aP.xy, P2 = aP.zw;
  vec2 d = P2 - P0;
  float L = max(length(d), 1e-3);
  vec2 n = vec2(-d.y, d.x) / L;
  vec2 C = (P0 + P2) * 0.5 + n * aB.x * L;
  float s = 1.0 - t;
  vec2 pos = s * s * P0 + 2.0 * s * t * C + t * t * P2;
  vec2 tg = 2.0 * s * (C - P0) + 2.0 * t * (P2 - C);
  vec2 nn = normalize(vec2(-tg.y, tg.x) + 1e-6);
  float core = mix(aB.y, 0.45, t);                 // tapered: thick at the source, hairline at the target
  float w = core + 2.4;                            // room for the soft glow
  pos += nn * side * w * 0.5;
  vec2 clip = vec2(pos.x / uView.x * 2.0 - 1.0, 1.0 - pos.y / uView.y * 2.0);
  gl_Position = vec4(clip, 0.0, 1.0);
  float a = mix(aC0.a, aC1.a, u) * smoothstep(aT.x, aT.x + uTime.y, uTime.x); // an edge grows once both ends exist
  vCol = mix(aC0.rgb, aC1.rgb, t) * a;
  vSide = side;
  vW = core / w;
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
uniform vec2 uView;
uniform vec2 uTime;
out vec2 vP;
out vec4 vCol;
out vec4 vS;
out float vPx;
out float vExt;
void main() {
  vec2 q = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1)) * 2.0 - 1.0;
  float ext = aA.w;
  // time replay, entirely on the GPU: visible from birth, a flash on landing, dim once superseded
  float age = uTime.x - aT.x;
  float vis = smoothstep(0.0, uTime.y, age);
  float flash = aT.z * step(0.0, age) * exp(-age / max(uTime.y * 2.5, 1e-4));
  float dim = uTime.x >= aT.y ? 0.35 : 1.0;
  vec2 pos = aA.xy + q * aA.z * ext * (1.0 + flash * 1.4);
  gl_Position = vec4(pos.x / uView.x * 2.0 - 1.0, 1.0 - pos.y / uView.y * 2.0, 0.0, 1.0);
  vP = q * ext;
  vCol = vec4(aCol.rgb, aCol.a * vis * dim * (1.0 + 3.0 * flash)); vS = aS; vPx = 1.0 / max(aA.z, 0.5); vExt = ext;
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

  window.GraphGL = {
    init(gl) {
      // light buffer: f32 (RGBA32F + float blending) > f16 (RGBA16F) > u8 (RGBA8, pre-scaled accumulation)
      const want = (new URLSearchParams(location.search).get('light') || '').toLowerCase();
      const cbf = gl.getExtension('EXT_color_buffer_float'), fblend = gl.getExtension('EXT_float_blend');
      const cbh = gl.getExtension('EXT_color_buffer_half_float');
      let light = cbf && fblend ? 'f32' : cbf || cbh ? 'f16' : 'u8';
      if (want === 'f16' && (cbf || cbh)) light = 'f16';
      if (want === 'u8') light = 'u8';
      if (want === 'f32' && !(cbf && fblend)) light = cbf || cbh ? 'f16' : 'u8';
      const FMT = { f32: [gl.RGBA32F, gl.FLOAT], f16: [gl.RGBA16F, gl.HALF_FLOAT], u8: [gl.RGBA8, gl.UNSIGNED_BYTE] }[light];
      const hdr = light !== 'u8';
      const pe = prog(gl, EDGE_VS, EDGE_FS), pn = prog(gl, NODE_VS, NODE_FS);
      const par = gl.getExtension('KHR_parallel_shader_compile');
      let isReady = false;
      let uE, uN;
      const mkVao = (layout, stride) => {
        const vao = gl.createVertexArray(), buf = gl.createBuffer();
        gl.bindVertexArray(vao); gl.bindBuffer(gl.ARRAY_BUFFER, buf);
        let off = 0;
        layout.forEach((n, loc) => { gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, n, gl.FLOAT, false, stride * 4, off * 4); gl.vertexAttribDivisor(loc, 1); off += n; });
        gl.bindVertexArray(null);
        return { vao, buf };
      };
      const E = mkVao([4, 4, 4, 4, 4], EDGE_F), N = mkVao([4, 4, 4, 4], NODE_F);
      let uTe, uTn, uGe, uGn;
      let uAt, uGr;
      const ready = () => {
        if (isReady) return true;
        if (par && !(gl.getProgramParameter(pe, par.COMPLETION_STATUS_KHR) && gl.getProgramParameter(pn, par.COMPLETION_STATUS_KHR))) return false;
        for (const p of [pe, pn]) if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getShaderInfoLog(p.__vs) + gl.getShaderInfoLog(p.__fs) + gl.getProgramInfoLog(p));
        uE = gl.getUniformLocation(pe, 'uView'); uN = gl.getUniformLocation(pn, 'uView');
        uTe = gl.getUniformLocation(pe, 'uTime'); uTn = gl.getUniformLocation(pn, 'uTime');
        uGe = gl.getUniformLocation(pe, 'uGain'); uGn = gl.getUniformLocation(pn, 'uGain');
        uAt = gl.getUniformLocation(pn, 'uAtlas'); uGr = gl.getUniformLocation(pn, 'uGrid');
        isReady = true; return true;
      };
      const atlas = gl.createTexture(); let grid = [1, 1, 64];
      gl.bindTexture(gl.TEXTURE_2D, atlas); gl.texImage2D(gl.TEXTURE_2D, 0, gl.R16F, 1, 1, 0, gl.RED, gl.FLOAT, new Float32Array([100]));
      const tex = gl.createTexture(), fb = gl.createFramebuffer();
      let w = 0, h = 0;
      return {
        hdr, light, tex, EDGE_F, NODE_F, ready,
        setAtlas(data, w, h, cols, rows, cell) {
          gl.bindTexture(gl.TEXTURE_2D, atlas);
          gl.texImage2D(gl.TEXTURE_2D, 0, gl.R16F, w, h, 0, gl.RED, gl.FLOAT, data);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
          grid = [cols, rows, cell];
        },
        draw(f) {
          if (!ready()) return null;
          const W = Math.round(f.vw * f.ratio), H = Math.round(f.vh * f.ratio);
          if (W !== w || H !== h) {
            gl.bindTexture(gl.TEXTURE_2D, tex);
            gl.texImage2D(gl.TEXTURE_2D, 0, FMT[0], W, H, 0, gl.RGBA, FMT[1], null);
            // RGBA32F + LINEAR samples as black on ANGLE/NVIDIA even with OES_texture_float_linear (the molecule
            // regression of f59ffa4): the buffer is sampled 1:1, so NEAREST is exact and always complete
            const filt = light === 'f32' ? gl.NEAREST : gl.LINEAR;
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filt); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filt);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
            gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
            gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
            w = W; h = H;
          }
          gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
          gl.viewport(0, 0, W, H);
          gl.clearColor(0, 0, 0, 1); gl.clear(gl.COLOR_BUFFER_BIT);
          gl.enable(gl.BLEND); gl.blendFunc(gl.ONE, gl.ONE);
          if (f.nEdges) {
            gl.useProgram(pe); gl.uniform2f(uE, f.vw, f.vh); gl.uniform2f(uTe, f.tNow || 0, f.tFade || 0.01); gl.uniform1f(uGe, f.gain || 1);
            gl.bindVertexArray(E.vao); gl.bindBuffer(gl.ARRAY_BUFFER, E.buf);
            gl.bufferData(gl.ARRAY_BUFFER, f.edges.subarray(0, f.nEdges * EDGE_F), gl.STREAM_DRAW);
            gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, (SEG + 1) * 2, f.nEdges);
          }
          if (f.nNodes) {
            gl.useProgram(pn); gl.uniform2f(uN, f.vw, f.vh); gl.uniform2f(uTn, f.tNow || 0, f.tFade || 0.01); gl.uniform1f(uGn, f.gain || 1);
            gl.activeTexture(gl.TEXTURE3); gl.bindTexture(gl.TEXTURE_2D, atlas); gl.uniform1i(uAt, 3); gl.uniform3f(uGr, grid[0], grid[1], grid[2]);
            gl.bindVertexArray(N.vao); gl.bindBuffer(gl.ARRAY_BUFFER, N.buf);
            gl.bufferData(gl.ARRAY_BUFFER, f.nodes.subarray(0, f.nNodes * NODE_F), gl.STREAM_DRAW);
            gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, f.nNodes);
          }
          gl.disable(gl.BLEND);
          gl.bindVertexArray(null);
          return tex;
        },
      };
    },
  };
})();
