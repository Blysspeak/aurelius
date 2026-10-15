// Light and glass: the WebGL2 composite over a 2D scene canvas.
// Pass 1 (cheap, full screen): scene texture + glass slabs. A slab is a rounded rect with a
//   precomputed height profile: flat centre, bevelled edge. Refraction offsets the scene lookup
//   along the SDF gradient on the bevel only; R, G and B take different offsets there
//   (dispersion), so colour exists only as the fringe at slab edges. No blur, no raymarch.
// Pass 2 (bounded): one raymarched glass object (the loader mark, then the memory lens), ported
//   from React Bits FluidGlass: biconvex lens SDF, ior 1.5, dispersion 0.35, fresnel, studio rim.
//   Every pixel outside the object's bounding sphere returns pass 1 untouched.
(function () {
  const VS = `#version 300 es
const vec2 c[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
void main() { gl_Position = vec4(c[gl_VertexID], 0.0, 1.0); }`;

  const MAX = 10;
  const FS1 = `#version 300 es
precision highp float;
uniform sampler2D uScene;
uniform vec2 uView;
uniform float uRatio;
uniform vec4 uRect[${MAX}];   // x, y, w, h in css px, top-left origin
uniform vec4 uP[${MAX}];      // radius, bevel, presence, refraction strength
uniform vec4 uQ[${MAX}];      // absorb, dispersion, rim, sheen
uniform int uCount;
out vec4 o;

float sdRound(vec2 p, vec2 b, float r) {
  vec2 q = abs(p) - b + r;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
vec3 sceneAt(vec2 css) {
  vec2 uv = vec2(css.x, uView.y - css.y) / uView;
  return texture(uScene, clamp(uv, vec2(0.0), vec2(1.0))).rgb;
}
void main() {
  vec2 w = gl_FragCoord.xy / uRatio;
  vec2 css = vec2(w.x, uView.y - w.y);
  vec3 col = sceneAt(css);
  for (int i = 0; i < ${MAX}; i++) {
    if (i >= uCount) break;
    vec4 r = uRect[i]; vec4 P = uP[i]; vec4 Q = uQ[i];
    vec2 c = r.xy + r.zw * 0.5;
    vec2 hb = r.zw * 0.5;
    float d = sdRound(css - c, hb, P.x);
    if (d > 2.0) continue;
    vec2 e = vec2(0.75, 0.0);
    vec2 g = vec2(sdRound(css + e.xy - c, hb, P.x) - sdRound(css - e.xy - c, hb, P.x),
                  sdRound(css + e.yx - c, hb, P.x) - sdRound(css - e.yx - c, hb, P.x));
    g = g / max(length(g), 1e-4);
    float t = clamp(-d / P.y, 0.0, 1.0);           // 0 on the rim, 1 where the flat centre starts
    float slope = (1.0 - t) * (1.0 - t);           // derivative of a rounded bevel profile
    vec2 off = g * slope * P.w * P.y;              // edge bends the field outward: centre looks magnified
    float k = Q.y * slope;
    vec3 refr = vec3(sceneAt(css + off * (1.0 + k)).r, sceneAt(css + off).g, sceneAt(css + off * (1.0 - k)).b);
    vec3 body = refr * (1.0 - Q.x) + vec3(0.012);
    vec2 L = normalize(vec2(-0.42, -0.55));         // light from top-left (css y down)
    float facing = max(dot(-g, L), 0.0);
    float rim = smoothstep(1.4, 0.0, abs(d + 0.6)) * (0.18 + 0.82 * facing) * Q.z;
    float sheen = (1.0 - t) * facing * Q.w;
    body += vec3(rim * 0.62 + sheen * 0.10);
    float a = smoothstep(0.9, -0.9, d) * P.z;
    col = mix(col, body, a);
  }
  o = vec4(col, 1.0);
}`;

  const FS2 = `#version 300 es
precision highp float;
uniform sampler2D uBg;
uniform sampler2D uMark;
uniform vec2 uView;
uniform float uRatio;
uniform float uFocal;
uniform vec3 uCenter;     // gl px, y up; z = elevation above the field
uniform mat3 uInverse;    // rotation + inverse stretch
uniform float uSize;      // px per shape unit
uniform float uBound;     // px
uniform float uMorph;     // 0 = mark, 1 = lens
uniform float uFlat;      // 1 = full lens, -> 0 melts into the plane
uniform float uIor;
uniform float uDispersion;
uniform float uPresence;
uniform float uThick;
uniform float uAA;
out vec4 o;

float sdMark2(vec2 p) {
  vec2 uv = vec2(p.x, -p.y) * 0.5 + 0.5;
  vec2 cl = clamp(uv, vec2(0.002), vec2(0.998));
  float d = texture(uMark, cl).r;
  return d + length((uv - cl) * 2.0);
}
float sdMark(vec3 p) {
  float d2 = sdMark2(p.xy);
  vec2 w = vec2(d2 + 0.012, abs(p.z) - uThick);
  return min(max(w.x, w.y), 0.0) + length(max(w, 0.0)) - 0.03;
}
float sdLens(vec3 p) {
  p.z /= max(uFlat, 0.04);
  float a = length(p - vec3(0.0, 0.0, -1.85)) - 2.13;
  float b = length(p - vec3(0.0, 0.0, 1.85)) - 2.13;
  float h = clamp(0.5 - 0.5 * (b - a) / 0.2, 0.0, 1.0);
  return (mix(b, a, h) + 0.2 * h * (1.0 - h)) * max(uFlat, 0.04);
}
float field(vec3 p) {
  if (uMorph <= 0.0) return sdMark(p);
  if (uMorph >= 1.0) return sdLens(p);
  return mix(sdMark(p), sdLens(p), uMorph);
}
float map(vec3 w) { return field(uInverse * (w - uCenter) / uSize) * uSize * 0.85; }
vec3 normalAt(vec3 w) {
  float e = 1.1;
  vec2 k = vec2(1.0, -1.0);
  return normalize(k.xyy * map(w + k.xyy * e) + k.yyx * map(w + k.yyx * e) + k.yxy * map(w + k.yxy * e) + k.xxx * map(w + k.xxx * e));
}
vec3 bg(vec2 w) { return texture(uBg, clamp(w / uView, vec2(0.0), vec2(1.0))).rgb; }
vec2 landing(vec3 p, vec3 d, vec2 fb) { if (d.z > -0.0005) return fb; return p.xy + d.xy * (-p.z / d.z); }
vec3 studio(vec3 d) {
  vec2 a = vec2(atan(d.x, d.z), d.y);
  float key = smoothstep(0.42, 0.22, abs(a.x + 0.62)) * smoothstep(0.3, 0.12, abs(a.y - 0.5));
  float strip = smoothstep(0.12, 0.05, abs(a.x - 0.95)) * smoothstep(0.8, 0.35, abs(a.y - 0.05));
  return vec3(18.0) * key + vec3(8.0) * strip + vec3(smoothstep(0.0, 1.0, d.z) * 0.04);
}
vec3 render(vec2 w) {
  vec3 base = bg(w);
  vec3 cam = vec3(uView * 0.5, uFocal);
  vec3 rd = normalize(vec3(w, 0.0) - cam);
  vec3 oc = cam - uCenter;
  float b = dot(oc, rd);
  float c = dot(oc, oc) - uBound * uBound;
  float disc = b * b - c;
  if (disc <= 0.0) return base;
  float sq = sqrt(disc);
  float t = max(-b - sq, 0.0), tEnd = -b + sq;
  float pixel = 1.0 / uRatio, eps = 0.3 * pixel;
  bool hit = false;
  float closest = 1e5, closestT = t;
  for (int i = 0; i < 110; i++) {
    float d = map(cam + rd * t);
    if (d < closest) { closest = d; closestT = t; }
    if (d < eps) { hit = true; break; }
    t += max(d, eps * 0.5);
    if (t > tEnd) break;
  }
  if (!hit) {
    float cover = clamp(1.0 - closest / (pixel * 1.4), 0.0, 1.0) * uPresence;
    if (cover > 0.0) { vec3 n = normalAt(cam + rd * closestT); base += vec3(0.16) * cover * pow(1.0 - abs(n.z), 2.0); }
    return base;
  }
  vec3 p = cam + rd * t;
  vec3 n = normalAt(p);
  vec3 inside = refract(rd, n, 1.0 / uIor);
  vec3 q = p + inside * eps * 4.0;
  vec3 exitDir = inside;
  bool escaped = false;
  for (int bounce = 0; bounce < 2; bounce++) {
    for (int i = 0; i < 56; i++) {
      float d = map(q);
      if (d > -eps) break;
      q += inside * max(-d * 0.9, eps);
    }
    vec3 n2 = normalAt(q);
    vec3 out1 = refract(inside, -n2, uIor);
    if (dot(out1, out1) > 0.0) { exitDir = out1; escaped = true; break; }
    inside = reflect(inside, -n2);
    q += inside * eps * 4.0;
  }
  if (exitDir.z > -0.2) exitDir = normalize(vec3(exitDir.xy, -0.2 - abs(exitDir.z)));
  vec2 hitT = landing(q, exitDir, w);
  vec3 tr = vec3(bg(hitT));
  if (escaped && uDispersion > 0.0) {
    vec3 n2 = normalAt(q);
    float s = uDispersion * 0.06;
    vec3 dR = refract(inside, -n2, uIor - s);
    vec3 dB = refract(inside, -n2, uIor + s);
    if (dot(dR, dR) > 0.0 && dR.z < -0.2) tr.r = bg(landing(q, dR, hitT)).r;
    if (dot(dB, dB) > 0.0 && dB.z < -0.2) tr.b = bg(landing(q, dB, hitT)).b;
  }
  float cosi = clamp(-dot(rd, n), 0.0, 1.0);
  float f0 = pow((uIor - 1.0) / (uIor + 1.0), 2.0);
  float fres = f0 + (1.0 - f0) * pow(1.0 - cosi, 5.0);
  vec3 md = reflect(rd, n);
  vec3 refl = md.z < -0.0005 ? bg(landing(p, md, w)) : clamp(studio(md), 0.0, 1.0);
  vec3 glass = mix(tr, refl, fres);
  vec2 facing = normalize(vec2(-0.42, 0.55));
  float rimL = pow(1.0 - cosi, 3.0) * (0.25 + 0.75 * abs(dot(normalize(n.xy + 1e-5), facing))) * 0.6;
  glass = vec3(rimL) + glass * (1.0 - rimL);
  return mix(base, glass, uPresence);
}
void main() {
  vec2 w = gl_FragCoord.xy / uRatio;
  if (uPresence <= 0.0) { o = vec4(bg(w), 1.0); return; }
  if (uAA < 0.5) { o = vec4(render(w), 1.0); return; }
  // 4 rotated-grid sub-pixel rays: the loader mark is the first thing the owner sees
  float px = 1.0 / uRatio;
  vec3 acc = render(w + vec2(0.125, 0.375) * px) + render(w + vec2(-0.375, 0.125) * px)
           + render(w + vec2(-0.125, -0.375) * px) + render(w + vec2(0.375, -0.125) * px);
  o = vec4(acc * 0.25, 1.0);
}`;

  function compile(gl, type, src) {
    const s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
    return s;
  }
  function program(gl, fs) {
    const p = gl.createProgram();
    gl.attachShader(p, compile(gl, gl.VERTEX_SHADER, VS));
    gl.attachShader(p, compile(gl, gl.FRAGMENT_SHADER, fs));
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
    const u = {};
    const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS);
    for (let i = 0; i < n; i++) {
      const info = gl.getActiveUniform(p, i);
      const name = info.name.replace(/\[0\]$/, '');
      u[name] = gl.getUniformLocation(p, info.name);
    }
    return { p, u };
  }
  function tex(gl, filter) {
    const t = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return t;
  }

  window.Glass = {
    init(canvas) {
      const gl = canvas.getContext('webgl2', { antialias: false, alpha: false, premultipliedAlpha: false, preserveDrawingBuffer: true });
      if (!gl) return null;
      let p1, p2;
      try { p1 = program(gl, FS1); p2 = program(gl, FS2); } catch (e) { console.error(e); return null; }
      const sceneTex = tex(gl, gl.LINEAR);
      const fbTex = tex(gl, gl.LINEAR);
      const markTex = tex(gl, gl.LINEAR);
      const fb = gl.createFramebuffer();
      gl.bindVertexArray(gl.createVertexArray());
      let fw = 0, fh = 0;
      const rect = new Float32Array(MAX * 4), P = new Float32Array(MAX * 4), Q = new Float32Array(MAX * 4);
      const api = {
        gl,
        setMark(data, n) {
          gl.bindTexture(gl.TEXTURE_2D, markTex);
          gl.texImage2D(gl.TEXTURE_2D, 0, gl.R16F, n, n, 0, gl.RED, gl.FLOAT, data);
        },
        frame(f) {
          const W = canvas.width, H = canvas.height;
          if (W !== fw || H !== fh) {
            gl.bindTexture(gl.TEXTURE_2D, fbTex);
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, W, H, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
            gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
            gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, fbTex, 0);
            fw = W; fh = H;
          }
          gl.bindTexture(gl.TEXTURE_2D, sceneTex);
          gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
          gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, f.scene);
          gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
          // pass 1
          gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
          gl.viewport(0, 0, W, H);
          gl.useProgram(p1.p);
          const n = Math.min(f.slabs.length, MAX);
          for (let i = 0; i < n; i++) {
            const s = f.slabs[i];
            rect.set([s.x, s.y, s.w, s.h], i * 4);
            P.set([s.r, s.bevel, s.presence, s.strength], i * 4);
            Q.set([s.absorb, s.disp, s.rim, s.sheen], i * 4);
          }
          gl.uniform4fv(p1.u.uRect, rect); gl.uniform4fv(p1.u.uP, P); gl.uniform4fv(p1.u.uQ, Q);
          gl.uniform1i(p1.u.uCount, n);
          gl.uniform2f(p1.u.uView, f.vw, f.vh); gl.uniform1f(p1.u.uRatio, f.ratio);
          gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, sceneTex); gl.uniform1i(p1.u.uScene, 0);
          gl.drawArrays(gl.TRIANGLES, 0, 3);
          // pass 2
          gl.bindFramebuffer(gl.FRAMEBUFFER, null);
          gl.useProgram(p2.p);
          const o = f.obj;
          gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, fbTex); gl.uniform1i(p2.u.uBg, 0);
          gl.activeTexture(gl.TEXTURE1); gl.bindTexture(gl.TEXTURE_2D, markTex); gl.uniform1i(p2.u.uMark, 1);
          gl.uniform2f(p2.u.uView, f.vw, f.vh); gl.uniform1f(p2.u.uRatio, f.ratio);
          gl.uniform1f(p2.u.uFocal, 1800);
          gl.uniform3f(p2.u.uCenter, o.x, f.vh - o.y, o.z);
          gl.uniformMatrix3fv(p2.u.uInverse, false, o.inv);
          gl.uniform1f(p2.u.uSize, o.size);
          gl.uniform1f(p2.u.uBound, o.size * o.extent);
          gl.uniform1f(p2.u.uMorph, o.morph); gl.uniform1f(p2.u.uFlat, o.flat);
          gl.uniform1f(p2.u.uIor, 1.5); gl.uniform1f(p2.u.uDispersion, 0.35);
          gl.uniform1f(p2.u.uPresence, o.presence); gl.uniform1f(p2.u.uThick, o.thick); gl.uniform1f(p2.u.uAA, o.aa ? 1 : 0);
          gl.drawArrays(gl.TRIANGLES, 0, 3);
        },
      };
      return api;
    },
  };
})();
