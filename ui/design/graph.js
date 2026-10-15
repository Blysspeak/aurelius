// The neural field: nodes as somas, edges as tapered curved axons, pulses as action potentials.
// Everything is instanced and drawn additively into a half-float target, then tone-mapped
// on luminance (hue kept) inside the glass composite. No vertex buffers: geometry comes
// from gl_VertexID, per-instance data is one interleaved buffer per kind.
(function () {
  const SEG = 18;               // segments per curve
  const EDGE_F = 16;            // floats per edge/pulse instance
  const NODE_F = 11;            // floats per node instance

  const EDGE_VS = `#version 300 es
precision highp float;
layout(location=0) in vec4 aP;      // p0.xy, p1.xy (css px)
layout(location=1) in vec4 aB;      // bend, w0, t0, t1
layout(location=2) in vec4 aC0;     // source rgb, a0
layout(location=3) in vec4 aC1;     // target rgb, a1
uniform vec2 uView;
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
  float a = mix(aC0.a, aC1.a, u);
  vCol = mix(aC0.rgb, aC1.rgb, t) * a;
  vSide = side;
  vW = core / w;
}`;
  const EDGE_FS = `#version 300 es
precision highp float;
in vec3 vCol;
in float vSide;
in float vW;
out vec4 o;
void main() {
  float x = abs(vSide);
  float coreA = 1.0 - smoothstep(vW * 0.5, vW * 0.5 + 0.18, x);
  float glow = exp(-x * x * 6.0) * 0.35;
  o = vec4(vCol * (coreA + glow), 1.0);
}`;

  const NODE_VS = `#version 300 es
precision highp float;
layout(location=0) in vec4 aA;   // x, y, size(px radius), extent multiplier
layout(location=1) in vec4 aCol; // rgb, intensity
layout(location=2) in vec3 aS;   // halo, shape, glyph mix
uniform vec2 uView;
out vec2 vP;
out vec4 vCol;
out vec3 vS;
out float vPx;
out float vExt;
void main() {
  vec2 q = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1)) * 2.0 - 1.0;
  float ext = aA.w;
  vec2 pos = aA.xy + q * aA.z * ext;
  gl_Position = vec4(pos.x / uView.x * 2.0 - 1.0, 1.0 - pos.y / uView.y * 2.0, 0.0, 1.0);
  vP = q * ext;
  vCol = aCol; vS = aS; vPx = 1.0 / max(aA.z, 0.5); vExt = ext;
}`;
  const NODE_FS = `#version 300 es
precision highp float;
in vec2 vP;
in vec4 vCol;
in vec3 vS;
in float vPx;
in float vExt;
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
  float body = mix(soma, g * 1.15 + soma * 0.25, vS.z);
  o = vec4(vCol.rgb * vCol.a * (body + halo), 1.0);
}`;

  function sh(gl, type, src) { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s)); return s; }
  function prog(gl, vs, fs) { const p = gl.createProgram(); gl.attachShader(p, sh(gl, gl.VERTEX_SHADER, vs)); gl.attachShader(p, sh(gl, gl.FRAGMENT_SHADER, fs)); gl.linkProgram(p); if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p)); return p; }

  window.GraphGL = {
    init(gl) {
      const hdr = !!gl.getExtension('EXT_color_buffer_float');
      const pe = prog(gl, EDGE_VS, EDGE_FS), pn = prog(gl, NODE_VS, NODE_FS);
      const uE = gl.getUniformLocation(pe, 'uView'), uN = gl.getUniformLocation(pn, 'uView');
      const mkVao = (layout, stride) => {
        const vao = gl.createVertexArray(), buf = gl.createBuffer();
        gl.bindVertexArray(vao); gl.bindBuffer(gl.ARRAY_BUFFER, buf);
        let off = 0;
        layout.forEach((n, loc) => { gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, n, gl.FLOAT, false, stride * 4, off * 4); gl.vertexAttribDivisor(loc, 1); off += n; });
        gl.bindVertexArray(null);
        return { vao, buf };
      };
      const E = mkVao([4, 4, 4, 4], EDGE_F), N = mkVao([4, 4, 3], NODE_F);
      const tex = gl.createTexture(), fb = gl.createFramebuffer();
      let w = 0, h = 0;
      return {
        hdr, tex, EDGE_F, NODE_F,
        draw(f) {
          const W = Math.round(f.vw * f.ratio), H = Math.round(f.vh * f.ratio);
          if (W !== w || H !== h) {
            gl.bindTexture(gl.TEXTURE_2D, tex);
            gl.texImage2D(gl.TEXTURE_2D, 0, hdr ? gl.RGBA16F : gl.RGBA8, W, H, 0, gl.RGBA, hdr ? gl.HALF_FLOAT : gl.UNSIGNED_BYTE, null);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
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
            gl.useProgram(pe); gl.uniform2f(uE, f.vw, f.vh);
            gl.bindVertexArray(E.vao); gl.bindBuffer(gl.ARRAY_BUFFER, E.buf);
            gl.bufferData(gl.ARRAY_BUFFER, f.edges.subarray(0, f.nEdges * EDGE_F), gl.STREAM_DRAW);
            gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, (SEG + 1) * 2, f.nEdges);
          }
          if (f.nNodes) {
            gl.useProgram(pn); gl.uniform2f(uN, f.vw, f.vh);
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
