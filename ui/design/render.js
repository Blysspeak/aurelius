// Render host: the WebGL2 context, the glass composite and the neural field live in a Worker on an
// OffscreenCanvas, so creating the context (~110-150 ms in the GPU process handshake) and every GL call
// stay off the main thread. The worker is built from the sources of AU_GLASS, AU_GRAPH and
// AU_RENDER_MAIN (Function.toString), so it also runs from file://. Without OffscreenCanvas the same
// code runs in-thread.
function AU_RENDER_MAIN(scope, post, sync) {
  let glass = null, ggl = null, want = '', latest = null, scheduled = false, shownSent = null, failed = false;
  const pend = { atlas: null, field: null, flags: null };
  const raf = scope.requestAnimationFrame ? (f) => scope.requestAnimationFrame(f) : (f) => setTimeout(f, 0);
  function draw() {
    scheduled = false;
    const f = latest; latest = null;
    if (!f || !glass) return;
    if (!ggl && glass.ready()) {
      try { ggl = scope.GraphGL.init(glass.gl, want); } catch (e) { post({ type: 'error', msg: String(e && e.message || e) }); }
      if (ggl) {
        post({ type: 'light', light: ggl.light });
        if (pend.atlas) ggl.setAtlas(...pend.atlas);
        if (pend.field) ggl.setField(...pend.field);
        if (pend.flags) ggl.setFlags(pend.flags);
      }
    }
    let graphTex = null;
    if (ggl) { try { graphTex = ggl.draw(f.g); } catch (e) { if (!failed) post({ type: 'error', msg: String(e && e.message || e) }); failed = true; } }
    const shown = glass.frame({ scene: f.scene, slabs: f.slabs, obj: f.obj, vw: f.vw, vh: f.vh, ratio: f.ratio, graphTex, graphGain: f.g.gain });
    if (f.scene && f.scene.close) f.scene.close();
    const s2 = !!shown && !!graphTex;
    if (s2 !== shownSent) { shownSent = s2; post({ type: 'shown', shown: !!shown, graph: s2 }); }
  }
  return function onMessage(m) {
    switch (m.type) {
      case 'init': want = m.want || ''; glass = scope.Glass.init(m.canvas); post({ type: 'init', ok: !!glass }); break;
      case 'resize': if (m.canvas) { m.canvas.width = m.w; m.canvas.height = m.h; } else if (glass) { glass.gl.canvas.width = m.w; glass.gl.canvas.height = m.h; } break;
      case 'mark': if (glass) glass.setMark(m.data, m.n); break;
      case 'atlas': pend.atlas = [m.data, m.w, m.h, m.cols, m.rows, m.cell]; if (ggl) ggl.setAtlas(...pend.atlas); break;
      case 'field': pend.field = [m.nodes, m.nNodes, m.edges, m.nEdges]; if (ggl) ggl.setField(...pend.field); break;
      case 'flags': pend.flags = m.data; if (ggl) ggl.setFlags(m.data); break;
      case 'frame':
        if (latest && latest.scene && !m.scene) m.scene = latest.scene;   // keep a pending scene upload
        else if (latest && latest.scene && latest.scene.close) latest.scene.close();
        latest = m;
        if (sync) draw(); else if (!scheduled) { scheduled = true; raf(draw); }
        break;
    }
  };
}

window.AURender = {
  // returns { send(msg, transfer), worker: bool } or null when there is no WebGL2 at all
  create(canvas, want, onMsg) {
    const offOK = typeof canvas.transferControlToOffscreen === 'function' && typeof OffscreenCanvas !== 'undefined' && !new URLSearchParams(location.search).has('nowkr');
    if (offOK) {
      const src = [AU_GLASS, AU_GRAPH, AU_RENDER_MAIN].map((f) => f.toString()).join('\n') +
        '\nAU_GLASS(self); AU_GRAPH(self);\nconst __h = AU_RENDER_MAIN(self, (m, t) => postMessage(m, t || []));\nonmessage = (e) => __h(e.data);';
      const w = new Worker(URL.createObjectURL(new Blob([src], { type: 'text/javascript' })));
      w.onmessage = (e) => onMsg(e.data);
      w.onerror = (e) => onMsg({ type: 'error', msg: e.message });
      const off = canvas.transferControlToOffscreen();
      w.postMessage({ type: 'init', canvas: off, want }, [off]);
      return { worker: true, send: (m, t) => w.postMessage(m, t || []) };
    }
    if (!window.WebGL2RenderingContext) return null;
    const h = AU_RENDER_MAIN(window, (m) => queueMicrotask(() => onMsg(m)), true);
    h({ type: 'init', canvas, want });
    return { worker: false, send: (m) => h(m) };
  },
};
