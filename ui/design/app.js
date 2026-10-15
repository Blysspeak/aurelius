// aurelius — light and glass. Design prototype, phase 1. Plain JS, no build step.
// Data: window.AU from snapshot.js (real snapshot of the owner's database, kept out of git).
(async () => {
  'use strict';
  const AU = window.AU;
  const yieldFrame = () => new Promise((r) => requestAnimationFrame(() => r())); // boot runs in slices between frames
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
  // the WebGL context is created after the first frames (context creation alone cost ~200 ms of the first task);
  // until then, and without WebGL2, the 2D scene is shown on #fallback
  const WANT_GL = !params.has('nogl') && !!window.WebGL2RenderingContext;
  let glass = null;
  const scene = document.createElement('canvas');
  document.body.classList.add('nogl');
  const ctx = scene.getContext('2d');
  const fbCtx = $('#fallback').getContext('2d');
  let VW = 0, VH = 0, DPR = 1;
  function resize() {
    DPR = Math.min(window.devicePixelRatio || 1, 2); vignette = null;
    VW = innerWidth; VH = innerHeight;
    for (const c of [scene, glCanvas, $('#fallback')]) { c.width = Math.round(VW * DPR); c.height = Math.round(VH * DPR); }
    fitCamera(true);
    requestAnimationFrame(measureLegend);
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
    // profile polyline, sampled from MK.sil every 18 path units once offline (getPointAtLength cost ~300 ms at boot)
    const SIL = [499.5,259.2,497.7,259.4,495.9,259.6,494.1,259.8,492.3,260,490.6,260.3,488.8,260.6,487,260.9,485.2,261.2,483.5,261.5,481.7,261.8,479.9,262.2,478.2,262.6,476.4,262.9,474.7,263.3,472.9,263.7,471.1,264.1,469.4,264.5,467.6,264.9,465.9,265.4,464.1,265.8,462.4,266.3,460.7,266.7,458.9,267.2,457.2,267.7,455.5,268.2,453.7,268.7,452,269.2,450.3,269.8,448.6,270.3,446.9,270.9,445.2,271.5,443.5,272.1,441.8,272.7,440.1,273.4,438.5,274.1,436.9,274.9,435.3,275.7,433.7,276.7,432.2,277.6,430.6,278.4,428.9,279,427.1,279.3,425.4,280,423.8,280.7,422.2,281.4,420.5,282.2,418.9,282.9,417.2,283.5,415.4,283.9,413.7,284.2,411.9,284.3,410.1,284.2,408.3,283.9,406.5,283.5,404.8,283,403.1,282.3,401.5,281.6,399.9,280.8,398.3,280,396.6,279.3,395,278.6,393.3,278,391.5,277.5,389.7,277.4,388.1,278.2,386.9,279.5,386.4,281.2,387,282.9,387.6,284.6,388.2,286.3,388.6,288,388.9,289.8,389.1,291.6,389,293.4,388.8,295.2,388.4,296.9,387.7,298.6,386.8,300.2,385.7,301.6,384.4,302.9,383,304,381.5,305,379.9,305.9,378.3,306.6,376.6,307.3,374.9,307.8,373.2,308.4,371.5,309.1,369.9,309.7,368.2,310.4,366.6,311.2,365,312,363.4,312.8,361.8,313.7,360.3,314.6,358.7,315.6,357.3,316.6,355.8,317.7,354.4,318.8,353,320,351.7,321.2,350.4,322.5,349.2,323.8,348.1,325.2,346.9,326.6,345.9,328,344.9,329.6,344,331.1,343.1,332.7,342.3,334.3,341.6,335.9,340.9,337.6,340.3,339.3,339.8,341,339.3,342.8,338.9,344.5,338.6,346.3,338.4,348.1,338.3,349.9,338.2,351.7,338.2,353.5,338.3,355.3,338.5,357.1,338.7,358.8,339,360.6,339.3,362.4,339.7,364.1,340.2,365.9,340.7,367.6,341.3,369.3,342,371,342.7,372.6,343.5,374.2,344.4,375.8,345.4,377.3,346.5,378.7,347.7,380.1,348.9,381.4,350,382.8,350.8,384.4,351.4,386.1,351.5,387.9,351.4,389.7,351.2,391.5,350.9,393.3,350.5,395,350,396.8,349.5,398.5,349,400.2,348.5,402,347.9,403.7,347.4,405.4,346.8,407.1,346.3,408.8,345.7,410.5,345.2,412.2,344.6,413.9,344,415.7,343.4,417.4,342.9,419.1,342.3,420.8,341.7,422.5,341.2,424.2,340.6,425.9,340.1,427.6,339.5,429.3,339,431.1,338.5,432.8,338,434.5,337.5,436.3,337.1,438,336.7,439.8,336.4,441.5,336.2,443.3,336.1,445.1,336.2,446.9,336.5,448.7,337,450.4,337.6,452.1,338.4,453.7,339.2,455.4,339.9,457,340.6,458.7,341.1,460.4,341.4,462.2,341.4,464,341.2,465.7,340.8,467.5,340.4,469.3,339.9,471,339.3,472.7,338.7,474.4,338,476,337.3,477.7,336.7,479.4,336,481,335.3,482.7,334.5,484.3,333.8,486,333.1,487.6,332.4,489.3,331.6,490.9,330.9,492.6,330.1,494.2,329.4,495.8,328.6,497.5,327.9,499.1,327.2,500.8,326.4,502.4,325.7,504,324.9,505.7,324.1,507.3,323.4,508.9,322.6,510.6,321.9,512.2,321.2,513.9,320.4,515.5,319.7,517.1,318.9,518.8,318.2,520.4,317.4,522,316.7,523.7,315.9,525.3,315.2,527,314.4,528.6,313.7,530.2,313,531.9,312.3,533.5,311.5,535.2,310.8,536.8,310.1,538.5,309.5,540.2,308.8,541.8,308.2,543.5,307.7,545.3,307.2,547,306.9,548.8,307.1,550.6,307.8,552.2,309,553.5,310.3,554.7,311.7,555.9,313.2,556.9,314.9,557.5,316.6,557.8,318.4,557.8,320.2,557.8,322,557.7,323.8,557.7,325.6,557.7,327.4,557.7,329.2,557.8,331,558,332.8,558.3,334.5,558.9,336.1,559.7,337.5,560.9,338.7,562.2,339.8,563.6,340.9,565.1,341.5,566.7,342,568.5,342.2,570.3,342.3,572.1,342.2,573.9,342,575.6,341.7,577.4,341.2,579.2,340.7,580.9,340,582.5,339.3,584.2,338.6,585.8,338.2,587.6,339.1,589.2,340.2,590.5,341.4,591.9,342.5,593.3,343.5,594.8,344.3,596.4,344.9,598.1,345.2,599.9,345.3,601.7,345.2,603.5,344.9,605.3,344.6,607,344.1,608.8,343.7,610.5,343.3,612.3,343.4,614.1,344.1,615.7,345.2,617.1,346.5,618.4,347.8,619.6,349.1,620.9,350.4,622.2,351.6,623.4,352.8,624.8,353.9,626.2,354.9,627.7,355.7,629.3,356.2,631,356.4,632.8,356.3,634.6,356,636.4,355.5,638.1,354.8,639.8,354.1,641.4,353.3,643.1,352.5,644.6,351.6,646.2,350.7,647.8,349.9,649.4,349.1,651,348.6,652.7,348.5,654.5,348.7,656.3,349.1,658.1,349.5,659.8,350,661.6,350.4,663.3,351,665,351.5,666.7,352.2,668.4,353,670,353.8,671.6,354.8,673.1,355.9,674.5,357.2,675.8,358.6,676.9,360.1,677.9,361.7,678.8,363.3,679.6,365,680.3,366.6,680.9,368.4,681.4,370.1,682,371.8,682.4,373.6,682.9,375.3,683.2,377.1,683.5,378.9,683.7,380.7,683.8,382.5,683.8,384.3,683.8,386.1,683.7,387.9,683.6,389.7,683.4,391.5,683.1,393.2,682.8,395,682.5,396.8,682.1,398.5,681.7,400.3,681.3,402,680.8,403.8,680.4,405.5,679.9,407.2,679.4,408.9,678.9,410.7,678.3,412.4,677.8,414.1,677.2,415.8,676.6,417.5,676.1,419.2,675.5,420.9,674.9,422.6,674.3,424.3,673.7,426,673,427.7,672.4,429.3,671.8,431,671.2,432.7,670.5,434.4,669.9,436.1,669.2,437.7,668.6,439.4,667.9,441.1,667.3,442.8,666.6,444.4,665.9,446.1,665.3,447.8,664.6,449.5,663.9,451.1,663.3,452.8,662.6,454.5,662,456.2,661.4,457.9,660.7,459.6,660.2,461.3,659.7,463.1,659.2,464.8,659,466.6,658.9,468.4,658.9,470.2,659.3,471.8,660.1,473.3,661.1,474.6,662.3,475.9,663.6,477,665,477.9,666.6,478.7,668.2,479.1,669.9,479.3,671.7,479.4,673.5,479.1,675.3,478.5,677,477.7,678.6,476.7,680.1,475.5,681.5,474.4,682.9,473.1,684.2,471.9,685.5,470.6,686.7,469.3,688,468,689.3,466.7,690.5,465.4,691.8,464.1,693,462.8,694.2,461.5,695.5,460.2,696.7,458.9,698,457.6,699.2,457,700.7,457,702.5,457,704.3,457,706.1,457,707.9,457,709.7,457,711.5,457,713.3,457,715.1,457,716.9,457,718.7,457,720.5,457,722.3,457,724.1,457,725.9,457,727.7,457,729.5,457,731.3,457,733.1,457,734.9,457,736.7,457,738.5,457.1,740.3,457.1,742.1,457.1,743.9,457.1,745.7,457.2,747.5,457.2,749.3,457.3,751.1,457.3,752.9,457.4,754.7,457.5,756.5,457.6,758.3,457.7,760.1,458.6,761.6,460.3,762.4,462,762.9,463.8,763.2,465.5,763.5,467.3,763.7,469.1,763.8,470.9,764,472.7,764.1,474.5,764.2,476.3,764.3,478.1,764.4,479.9,764.4,481.7,764.5,483.5,764.5,485.3,764.6,487.1,764.6,488.9,764.7,490.7,764.7,492.5,764.7,494.3,764.8,496.1,764.8,497.9,764.8,499.7,764.8,501.5,764.8,503.3,764.7,505.1,764.7,506.9,764.6,508.7,764.6,510.5,764.5,512.3,764.4,514.1,764.3,515.9,764.2,517.7,764.1,519.5,764,521.3,763.8,523.1,763.7,524.9,763.5,526.6,763.4,528.4,763.2,530.2,763,532,762.8,533.8,762.5,535.6,762.3,537.4,762.1,539.2,761.8,540.9,761.6,542.7,761.3,544.5,761,546.3,760.7,548,760.4,549.8,760,551.6,759.7,553.3,759.4,555.1,759,556.9,758.6,558.6,758.2,560.4,757.8,562.1,757.4,563.9,757,565.6,756.6,567.4,756.1,569.1,755.7,570.8,755.2,572.6,754.7,574.3,754.2,576,753.7,577.8,753.2,579.5,752.7,581.2,752.1,582.9,751.6,584.6,751,586.3,750.4,588,749.9,589.7,749.3,591.4,748.6,593.1,748,594.8,747.4,596.5,746.7,598.2,746.1,599.8,745.4,601.5,744.7,603.2,744.1,604.8,743.4,606.5,742.6,608.1,741.9,609.8,741.2,611.4,740.5,613,739.7,614.7,738.9,616.3,738.2,617.9,737.4,619.5,736.6,621.1,735.8,622.8,735,624.4,734.2,625.9,733.3,627.5,732.5,629.1,731.6,630.7,730.8,632.3,729.9,633.9,729,635.4,728.1,637,727.2,638.5,726.3,640.1,725.4,641.6,724.5,643.2,723.6,644.7,722.6,646.2,721.7,647.8,720.7,649.3,719.8,650.8,718.8,652.4,717.9,653.9,716.9,655.4,715.9,656.8,714.8,658.2,713.7,659.3,712.3,660,710.6,660.1,708.8,660,707,659.7,705.2,659.4,703.5,659.1,701.7,658.7,699.9,658.4,698.2,658.1,696.4,657.7,694.6,657.4,692.9,657.1,691.1,656.7,689.3,656.4,687.6,656.1,685.8,655.8,684,655.4,682.2,655.1,680.5,654.8,678.7,654.5,676.9,654.1,675.2,653.8,673.4,653.5,671.6,653.2,669.9,652.8,668.1,652.5,666.3,652.2,664.5,651.9,662.8,651.5,661,651.2,659.2,650.9,657.5,650.6,655.7,650.3,653.9,649.9,652.1,649.6,650.4,649.3,648.6,649,646.8,648.7,645.1,648.4,643.3,648.1,641.5,647.8,639.7,647.5,638,647.2,636.2,646.9,634.4,646.6,632.6,646.3,630.9,646,629.1,645.7,627.3,645.5,625.5,645.2,623.7,645,622,644.8,620.2,644.6,618.4,644.7,616.6,645.3,614.9,646,613.2,646.8,611.6,647.6,610,648.4,608.3,649.2,606.7,650,605.1,650.9,603.6,651.7,602,652.6,600.4,653.5,598.8,654.4,597.3,655.2,595.7,656.1,594.1,657,592.5,657.9,591,658.7,589.4,659.6,587.8,660.5,586.3,661.4,584.7,662.2,583.1,663.1,581.5,664,580,664.8,578.4,665.7,576.8,666.6,575.2,667.5,573.7,668.3,572.1,669.2,570.5,670.1,568.9,670.9,567.3,671.8,565.8,672.6,564.2,673.5,562.6,674.4,561,675.2,559.4,676.1,557.8,676.9,556.3,677.8,554.7,678.6,553.1,679.5,551.5,680.3,549.9,681.1,548.3,682,546.7,682.8,545.1,683.7,543.5,684.5,541.9,685.3,540.3,686.2,538.8,687,537.2,687.9,535.6,688.8,534,689.8,532.5,690.7,531,691.8,529.6,692.9,528.2,694.2,526.9,695.6,525.7,697.1,524.7,698.7,523.9,700.3,523.2,702,522.5,703.7,522,705.4,521.3,707,520.6,708.6,519.7,710.1,518.8,711.6,517.7,713,516.6,714.3,515.3,715.5,514,716.6,512.6,717.6,511.1,718.5,509.5,719.2,507.9,719.8,506.2,720.3,504.4,720.7,502.7,720.9,500.9,721,499.1,721,497.3,720.8,495.5,720.5,493.7,720.2,492,719.7,490.2,719,488.5,718.4,486.9,717.7,485.2,717,483.5,716.5,481.8,716.1,480.1,715.8,478.3,715.7,476.5,715.6,474.7,715.7,472.9,715.7,471.1,715.9,469.3,716,467.5,716.2,465.7,716.4,463.9,716.7,462.1,716.9,460.4,717.2,458.6,717.4,456.8,717.7,455,717.9,453.2,718.1,451.4,718.2,449.6,718.4,447.9,718.5,446.1,718.6,444.3,718.7,442.5,718.7,440.7,718.7,438.9,718.7,437.1,718.7,435.3,718.6,433.5,718.6,431.7,718.5,429.9,718.3,428.1,718.2,426.3,718,424.5,717.9,422.7,717.7,420.9,717.4,419.1,717.2,417.3,716.9,415.6,716.6,413.8,716.3,412,716,410.2,715.7,408.5,715.3,406.7,714.9,404.9,714.5,403.2,714.1,401.4,713.6,399.7,713.2,398,712.7,396.2,712.2,394.5,711.7,392.8,711.1,391.1,710.6,389.3,710,387.6,709.4,385.9,708.8,384.3,708.1,382.6,707.5,380.9,706.8,379.2,706.1,377.6,705.4,375.9,704.7,374.2,704,372.6,703.2,371,702.4,369.4,701.6,367.7,700.8,366.1,700,364.5,699.1,362.9,698.3,361.4,697.4,359.8,696.5,358.2,695.6,356.7,694.6,355.2,693.7,353.6,692.7,352.1,691.7,350.6,690.7,349.1,689.7,347.6,688.7,346.2,687.6,344.7,686.6,343.2,685.5,341.8,684.5,340.3,683.4,338.9,682.3,337.5,681.2,336,680.1,334.6,678.9,333.2,677.8,331.8,676.6,330.4,675.5,329.1,674.3,327.7,673.1,326.3,671.9,325,670.7,323.7,669.5,322.3,668.3,321,667,319.7,665.8,318.4,664.5,317.1,663.3,315.8,662,314.6,660.7,313.3,659.4,312.1,658.1,310.9,656.7,309.6,655.4,308.4,654.1,307.2,652.7,306.1,651.3,304.9,650,303.7,648.6,302.6,647.2,301.5,645.8,300.3,644.3,299.2,642.9,298.1,641.5,297,640,296,638.6,294.9,637.1,293.9,635.6,292.9,634.1,291.8,632.6,290.8,631.1,289.9,629.6,288.9,628.1,287.9,626.6,287,625,286.1,623.5,285.1,621.9,284.2,620.3,283.3,618.8,282.5,617.2,281.6,615.6,280.8,614,280,612.4,279.1,610.8,278.3,609.2,277.6,607.5,276.8,605.9,276,604.2,275.3,602.6,274.6,600.9,273.9,599.3,273.2,597.6,272.5,595.9,271.9,594.3,271.2,592.6,270.6,590.9,270,589.2,269.4,587.5,268.8,585.8,268.2,584,267.7,582.3,267.2,580.6,266.6,578.9,266.1,577.1,265.7,575.4,265.2,573.7,264.7,571.9,264.3,570.2,263.9,568.4,263.5,566.7,263.1,564.9,262.7,563.1,262.3,561.4,262,559.6,261.7,557.8,261.4,556,261.1,554.3,260.8,552.5,260.5,550.7,260.3,548.9,260,547.1,259.8,545.3,259.6,543.6,259.4,541.8,259.2,540,259.1,538.2,258.9,536.4,258.8,534.6,258.7,532.8,258.6,531,258.5,529.2,258.4,527.4,258.3,525.6,258.3,523.8,258.3,522,258.3,520.2,258.3,518.4,258.3,516.6,258.3,514.8,258.3,513,258.4,511.2,258.4,509.4,258.5,507.6,258.6,505.8,258.7,504,258.8,502.2,259,500.4,259.1];
    const sil = []; for (let k = 0; k < SIL.length; k += 2) sil.push([SIL[k], SIL[k + 1]]);
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
  // pre-loader: the stroke of light starts on the first frame, while the rest of the boot runs in slices
  const PRE_T0 = performance.now();
  let preOn = !params.has('skip');
  (function pre() {
    if (!preOn) return;
    const fb = $('#fallback'), c = fb.getContext('2d'), W = innerWidth, H = innerHeight, d = Math.min(devicePixelRatio || 1, 2);
    if (fb.width !== Math.round(W * d)) { fb.width = Math.round(W * d); fb.height = Math.round(H * d); }
    document.body.classList.add('nogl');
    c.setTransform(d, 0, 0, d, 0, 0); c.fillStyle = '#070a12'; c.fillRect(0, 0, W, H);
    const t = clamp((performance.now() - PRE_T0) / 2300, 0, 1), p = clamp(1 - Math.pow(1 - t, 1.6) * (1 - 0.25 * t), 0, 1);
    const r = Math.min(W, H) * 0.2, k = r / 338, upto = p * strokes.total;
    c.save(); c.translate(W / 2, H / 2 - 20); c.scale(k, k); c.translate(-MK.c[0], -MK.c[1]);
    c.beginPath(); for (const st of strokes) { let started = false; for (let i = 0; i < st.length; i++) { if (st.len[i] > upto) break; if (!started) { c.moveTo(st[i][0], st[i][1]); started = true; } else c.lineTo(st[i][0], st[i][1]); } }
    c.strokeStyle = 'rgba(232,230,225,0.95)'; c.lineWidth = 1.6 / k; c.lineCap = 'round'; c.lineJoin = 'round'; c.stroke(); c.restore();
    requestAnimationFrame(pre);
  })();
  await yieldFrame();

  // signed distance field of the extruded mark for the glass raymarch
  // signed distance field of the mark, computed off the main thread (it used to block the first second of the loader)
  const SDF_WORKER = `onmessage = (ev) => {
    const { alpha, N } = ev.data, INF = 1e20, N2 = N * N;
    const fin = new Float64Array(N2), fout = new Float64Array(N2);
    for (let i = 0; i < N2; i++) { const inside = alpha[i] > 127; fin[i] = inside ? INF : 0; fout[i] = inside ? 0 : INF; }
    const f = new Float64Array(N), d = new Float64Array(N), v = new Int32Array(N), z = new Float64Array(N + 1);
    const pass = (g, get, set) => { for (let q = 0; q < N; q++) f[q] = get(g, q); let k = 0; v[0] = 0; z[0] = -INF; z[1] = INF; for (let q = 1; q < N; q++) { let s; do { const r = v[k]; s = (f[q] + q * q - (f[r] + r * r)) / (2 * q - 2 * r); if (s <= z[k]) k--; else break; } while (k >= 0); k++; v[k] = q; z[k] = s; z[k + 1] = INF; } k = 0; for (let q = 0; q < N; q++) { while (z[k + 1] < q) k++; const r = v[k]; d[q] = (q - r) * (q - r) + f[r]; } for (let q = 0; q < N; q++) set(g, q, d[q]); };
    for (const g of [fin, fout]) { for (let y = 0; y < N; y++) pass(g, (g, x) => g[y * N + x], (g, x, val) => { g[y * N + x] = val; }); for (let x = 0; x < N; x++) pass(g, (g, y) => g[y * N + x], (g, y, val) => { g[y * N + x] = val; }); }
    const out = new Float32Array(N2), tmp = new Float32Array(N2);
    for (let i = 0; i < N2; i++) out[i] = (fout[i] > 0 ? Math.sqrt(fout[i]) - 0.5 : -(Math.sqrt(fin[i]) - 0.5)) * (2 / N);
    const R = Math.round(3 * N / 1024);
    for (let p = 0; p < 2; p++) {
      for (let y = 0; y < N; y++) for (let x = 0; x < N; x++) { let a = 0, c = 0; for (let k = -R; k <= R; k++) { const xx = x + k; if (xx >= 0 && xx < N) { a += out[y * N + xx]; c++; } } tmp[y * N + x] = a / c; }
      for (let y = 0; y < N; y++) for (let x = 0; x < N; x++) { let a = 0, c = 0; for (let k = -R; k <= R; k++) { const yy = y + k; if (yy >= 0 && yy < N) { a += tmp[yy * N + x]; c++; } } out[y * N + x] = a / c; }
    }
    postMessage({ out, N }, [out.buffer]);
  };`;
  function markSDF() {
    const N = 768, cv = document.createElement('canvas'); cv.width = cv.height = N;
    const c = cv.getContext('2d'); const k = N / (2 * MK.R);
    c.setTransform(k, 0, 0, k, N / 2 - MK.c[0] * k, N / 2 - MK.c[1] * k);
    c.strokeStyle = '#fff'; c.fillStyle = '#fff'; c.lineCap = 'round'; c.lineJoin = 'round';
    for (const [r, w] of [[338, 15], [308, 9]]) { c.lineWidth = w; c.beginPath(); c.arc(MK.c[0], MK.c[1], r, 0, Math.PI * 2); c.stroke(); }
    c.save(); c.translate(0, 1024); c.scale(0.1, -0.1); c.lineWidth = 130; c.stroke(new Path2D(MK.sil)); c.restore();
    c.lineWidth = 10; for (const [a, b] of MK.edges) { c.beginPath(); c.moveTo(...MK.nodes[a]); c.lineTo(...MK.nodes[b]); c.stroke(); }
    for (const n of MK.nodes) { c.beginPath(); c.arc(n[0], n[1], 21, 0, Math.PI * 2); c.fill(); }
    const px = c.getImageData(0, 0, N, N).data, alpha = new Uint8Array(N * N);
    for (let i = 0; i < N * N; i++) alpha[i] = px[i * 4 + 3];
    try {
      const w = new Worker(URL.createObjectURL(new Blob([SDF_WORKER], { type: 'text/javascript' })));
      w.onmessage = (ev) => { glass.setMark(ev.data.out, ev.data.N); w.terminate(); };
      w.postMessage({ alpha, N }, [alpha.buffer]);
    } catch (e) { console.warn('SDF worker unavailable', e); }
  }

  await yieldFrame();
  if (WANT_GL) { glass = window.Glass.init(glCanvas); if (glass) markSDF(); }
  await yieldFrame();
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
  const projHub = new Map(GA.projects.map(([i, label, n, last, act, open, blk, lonely]) => [i, { label, n, last, act, open, blk, lonely }]));
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
  await yieldFrame();
  const LAYOUT_T0 = performance.now();
  // ---- project -> theme -> molecules (owner, 15.10). Sessions are time, not structure.
  const TH = GA.themes || [];
  for (const n of nodes) { n.theme = GA.nodes[n.i][6] ?? -1; n.session = n.t === 'session'; }
  const themeNodes = new Map();
  TH.forEach(([plabel, name, count], ti) => {
    const mem = nodes.filter((n) => n.theme === ti && !n.session && !n.virtual);
    if (!mem.length) return;
    const cnt = { k: 0, w: 0, p: 0, o: 0, i: 0 }; mem.forEach((m) => cnt[m.g]++);
    const tn = { i: nodes.length, t: 'theme', c: 1, label: name, age: Math.max(...mem.map((m) => m.age)), st: null, x: 0, y: 0, sx: 0, sy: 0, a: 0, bx: 0, by: 0, ba: 0, deg: 0, nd: 0, z: 0, g: Object.entries(cnt).sort((a, b) => b[1] - a[1])[0][0], shape: 8, icon: -1, hub: mem[0].hub, virtual: true, count, ti, open: false, attn: false };
    tn.birth = -tn.age; tn.supAt = 1e9;
    nodes.push(tn); adj.push([]); themeNodes.set(ti, tn);
    const G0 = ganglia.get(tn.hub); if (G0) G0.members.push(tn);
  });
  const NT = nodes.length;
  const ROOT = -10;
  // molecule links: logical relations only; "contains" only from a task or a work_log
  const MOL = { subtask_of: 'fwd', contains: 'rev', solves: 'fwd', refines: 'fwd', supersedes: 'fwd', implements: 'fwd', blocks: 'fwd', caused_by: 'rev', depends_on: 'rev', prerequisite: 'rev', next_step: 'fwd', verified_by: 'rev' };
  const MOL_ORDER = Object.keys(MOL);
  const parent = new Int32Array(NT).fill(-1), prel = new Array(NT).fill(null);
  const wouldCycle = (c, p) => { for (let q = p, k = 0; q >= 0 && k < 80; q = parent[q], k++) if (q === c) return true; return false; };
  for (const rel of MOL_ORDER) for (const l of links) {
    if (l.rel !== rel) continue;
    if (rel === 'contains' && l.source.t !== 'task' && l.source.t !== 'work_log') continue;
    const c = MOL[rel] === 'fwd' ? l.source.i : l.target.i, pp = MOL[rel] === 'fwd' ? l.target.i : l.source.i;
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
  const kids = nodes.map(() => []);
  for (const n of nodes) if (parent[n.i] >= 0) kids[parent[n.i]].push(n.i);
  const depthOf = (i) => { let d = 0; for (let q = i; q !== ROOT && q >= 0 && d < 90; q = parent[q]) d++; return d; };
  const leaves = new Int32Array(NT);
  const countLeaves = (i) => { let sum = kids[i].length ? 0 : 1; for (const k of kids[i]) sum += countLeaves(k); leaves[i] = sum; return sum; };
  for (const i of projHub.keys()) countLeaves(i);
  const reachesRoot = (i) => { let q = i; for (let k = 0; k < 90 && q >= 0; k++) q = parent[q]; return q === ROOT; };
  for (const n of nodes) { n.orphan = n.session || !reachesRoot(n.i); n.depth = n.orphan ? 99 : depthOf(n.i); }
  for (const G of ganglia.values()) { const real = G.members.filter((m) => !m.virtual && !m.session); G.orphans = real.filter((m) => m.unlinked).length; G.real = real.length; }
  const TYPE_RANK = { problem: 0, solution: 1, decision: 2, concept: 3, task: 4, work_log: 5 };
  // ---- layout: projects around the root; each theme is a petal; molecules along it by time, oldest at the base
  (function pack() {
    const D = [...ganglia.values()].sort((a, b) => b.R - a.R);
    D.forEach((G, k) => { const a = k * 2.39996, r = 160 + 50 * Math.sqrt(k); G.x = Math.cos(a) * r; G.y = Math.sin(a) * r; });
    const ps = d3.forceSimulation(D).force('c', d3.forceCollide((G) => G.R * 1.15 + 30).iterations(4)).force('r', d3.forceRadial((G) => 90 + G.R * 1.2, 0, 0).strength(0.08)).stop();
    for (let k = 0; k < 400; k++) ps.tick();
  })();
  const pol = (cx, cy, a, r) => [cx + Math.cos(a) * r, cy + Math.sin(a) * r];
  function placeMol(i, x, y, dir, spread, step, d) {
    const n = nodes[i]; n.ax = x; n.ay = y;
    const ch = kids[i].filter((k) => !nodes[k].virtual).sort((a, b) => (TYPE_RANK[nodes[a].t] ?? 6) - (TYPE_RANK[nodes[b].t] ?? 6) || nodes[a].age - nodes[b].age);
    const m = ch.length; if (!m) return;
    const w = Math.min(spread / m, 0.62);
    ch.forEach((k, j) => { const a = dir + (j - (m - 1) / 2) * w; const [cx, cy] = pol(x, y, a, step * (1 + 0.15 * (j % 2))); placeMol(k, cx, cy, a, Math.max(spread * 0.7, 0.5), step * 0.9, d + 1); });
  }
  const tips = [];
  for (const G of ganglia.values()) {
    const hub = G.key >= 0 ? nodes[G.key] : null;
    const out = Math.atan2(G.y, G.x);
    if (hub) { hub.ax = G.x; hub.ay = G.y; }
    const petals = [...themeNodes.values()].filter((t) => t.hub === G.key);
    const noTheme = G.members.filter((m) => !m.virtual && !m.session && !m.unlinked && parent[m.i] === G.key);
    if (noTheme.length) petals.push({ pseudo: true, label: 'без темы', count: noTheme.length, roots: noTheme, g: 'i' });
    const weights = petals.map((p) => Math.pow(p.pseudo ? p.roots.length * 3 : kids[p.i].length + 1, 0.7));
    const totW = weights.reduce((a, b) => a + b, 0) || 1, maxW = Math.max(...weights, 1);
    const SPAN = Math.min(Math.PI * 1.8, 0.7 + petals.length * 0.45);
    let a = out - SPAN / 2;
    petals.forEach((p, k) => {
      const span = SPAN * weights[k] / totW, th = a + span / 2; a += span;
      const R0 = 14, Rt = G.R * (0.55 + 0.6 * Math.sqrt(weights[k] / maxW));
      if (!p.pseudo) { [p.ax, p.ay] = pol(G.x, G.y, th, R0 * 1.5); p.th = th; p.span = span; p.Rt = Rt; const tip = { i: -20 - tips.length, sx: 0, sy: 0, a: 1, birth: p.birth, ax: 0, ay: 0, g: p.g }; [tip.ax, tip.ay] = pol(G.x, G.y, th, Rt * 1.04); p.tip = tip; tips.push(tip); }
      const roots = (p.pseudo ? p.roots : kids[p.i].map((kk) => nodes[kk]).filter((n) => !n.dust)).sort((x, y) => y.age - x.age);
      const M = roots.length;
      roots.forEach((r, j) => {
        const t = (j + 0.5) / Math.max(M, 1);
        const rr = R0 * 2.2 + (Rt - R0 * 2.2) * t;
        const aa = th + span * 0.34 * Math.sin(j * 2.39996 + k);
        const [x, y] = pol(G.x, G.y, aa, rr);
        placeMol(r.i, x, y, aa, 1.6, 5.5 + Math.min(6, Math.sqrt(molSize[r.i])), 0);
      });
      if (!p.pseudo) {
        const dust = kids[p.i].map((kk) => nodes[kk]).filter((n) => n.dust).sort((x, y) => y.age - x.age);
        dust.forEach((n, j) => { const [x, y] = pol(G.x, G.y, th + (hash(n.i) - 0.5) * span * 0.9, R0 * 2 + (Rt - R0 * 2) * (j + 0.5) / dust.length); n.ax = x; n.ay = y; });
      }
    });
    // unlinked without a theme: one small pool at the project's edge
    const pool = G.members.filter((m) => !m.virtual && !m.session && (m.orphan || (m.unlinked && !m.dust)) && m.t !== 'project');
    const [px, py] = hub ? pol(G.x, G.y, out + Math.PI, G.R * 0.55) : [G.x, G.y];
    pool.forEach((n, j) => { const r = 3 * Math.sqrt(j + 0.5), aa = j * 2.39996; n.ax = px + Math.cos(aa) * r; n.ay = py + Math.sin(aa) * r; n.pool = true; });
    for (const m of G.members) if (m.session) { m.ax = G.x; m.ay = G.y; }
  }
  for (const n of nodes) if (n.ax == null) { n.ax = 0; n.ay = 0; }
  const isWork = (c) => c.t === 'work_log' && prel[c.i] === 'contains';
  // time: birth in days from now (<= 0); a superseded fact dims when its successor is born
  for (const n of nodes) { if (n.birth == null) n.birth = -n.age; if (n.supAt == null) n.supAt = 1e9; }
  for (const l of links) if (l.rel === 'supersedes') l.target.supAt = Math.min(l.target.supAt, l.source.birth);
  for (const tn of themeNodes.values()) if (tn.tip) tn.tip.birth = tn.birth;
  const ROOT_MODE = 'owner';
  const AGE_MAX = Math.max(...nodes.map((n) => n.age)) + 0.5, LOGA = Math.log1p(AGE_MAX);
  const treeEdges = [];
  for (const n of nodes) if (parent[n.i] >= 0) {
    const kind = n.virtual ? 'spine' : prel[n.i] === 'theme' || prel[n.i] === 'belongs_to' ? 'attach' : prel[n.i] === 'dust' ? 'dust' : 'mol';
    treeEdges.push({ c: n, p: nodes[parent[n.i]], rel: prel[n.i], kind, work: kind !== 'mol', li: treeEdges.length, imp: Math.log2(1 + leaves[n.i]) + (prel[n.i] === 'solves' || prel[n.i] === 'supersedes' ? 1.5 : 0) + (n.age < 30 ? 0.6 : 0) });
  }
  const rootN = { i: ROOT, sx: 0, sy: 0, a: 1, g: 'i', hub: ROOT, nd: 99, t: 'root', x: 0, y: 0 };
  function chainOf(i) { // the molecule path: upstream main line, the node, downstream through theme and project to the root
    const down = []; for (let q = parent[i], k = 0; q >= 0 && k < 12; q = parent[q], k++) down.push(q);
    const up = []; let cur = i;
    for (let k = 0; k < 6; k++) { const ch = kids[cur].filter((x) => !nodes[x].dust && !nodes[x].virtual); if (!ch.length) break; cur = ch.sort((x, y) => leaves[y] - leaves[x] || nodes[x].age - nodes[y].age)[0]; up.push(cur); }
    const stubs = [];
    for (const m of [i, ...up]) for (const k of kids[m].filter((x) => !up.includes(x) && !nodes[x].dust && !nodes[x].virtual).slice(0, 2)) stubs.push([k, m]);
    return { up: up.reverse(), node: i, down, stubs, toRoot: reachesRoot(i) };
  }

  const LAYOUT_MS = Math.round(performance.now() - LAYOUT_T0);
  await yieldFrame();
  const nbLinks = [];
  for (const l of links) if (l.rel !== 'belongs_to') nbLinks.push({ source: l.source.i, target: l.target.i });
  let sim, tick = 0;
  const TICKS = 300;
  function anchorForce(alpha) { for (const n of nodes) { n.vx += (n.ax - n.x) * 0.3 * alpha; n.vy += (n.ay - n.y) * 0.3 * alpha; } }
  function newSim() { // the layout is analytic (themes, molecules, time): computed once at boot, no force ticks on the main thread
    nodes.forEach((n) => { n.x = n.ax; n.y = n.ay; n.vx = n.vy = 0; });
    tick = TICKS;
  }
  function stepSim() {}

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
  const state = { sessHi: null, hoverProj: null, screen: 'memory', prev: null, morph0: 0, sel: null, hover: null, selT: 0, chain: null, loader: null, replay: { s: 1, playing: false }, filterType: null, fin: { hover: -1, a: 0, b: AU.fin.months.length - 1, kind: -1, cat: -1, q: '', voided: false }, task: 'b7a7d9af' };

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
    $('#m-stats').innerHTML = `<b>${nf0.format(GA.full.nodes)}</b> узлов · <b>${nf0.format(GA.full.edges)}</b> связей<br>в прототипе ${nf0.format(NN)} узлов, ${nf0.format(links.length)} связей<br>облако = проект · ядро = свежее · цвет = смысл<br><span id="lightbuf"></span>` + (params.has('fps') ? '<br><span id="fps"></span>' : '');
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
    const maxX = VW - off - (state.sel != null ? 470 : 20) - 140; // never under the detail panel
    for (const [el, n, k] of ch.labels) { const x = Math.min(n.sx - off - 70, maxX), y = n.sy; el.style.transform = `translate(${x.toFixed(1)}px, ${(y + (k % 2 ? -78 : 18)).toFixed(1)}px)`; el.classList.toggle('up', k % 2 === 1); }
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
  const built = {};
  function ensureBuilt(name) { // screens build their DOM on first visit, not at boot
    if (built[name]) return; built[name] = true;
    if (name === 'tasks') { buildLanes(); showTask(); }
    if (name === 'finance') buildFinance();
    if (name === 'spec') buildSpec();
  }
  function setScreen(name, instant) {
    if (!SCREENS[name]) name = 'memory';
    ensureBuilt(name);
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
    if (n.session) { out[0] = x; out[1] = y; out[2] = 0; return; }
    if (state.sessHi != null) { out[0] = x; out[1] = y; out[2] = sessSet.has(n.i) ? 1.3 : 0.12 * (CONF_LOOK[n.c][0] > 0.9 ? 1 : 0.85); return; }
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
    state.loader = { t0: preOn ? PRE_T0 : performance.now(), phase: 'fetch', fetchP: 0, parseP: 0, layoutP: 0, done: 0 };
    newSim();
    $('#rail').style.opacity = 0; $('#cmd').style.opacity = 0;
    $('#rail').style.transition = $('#cmd').style.transition = 'none';
    for (const s of Object.values(SCREENS)) $(s).classList.remove('on', 'enter');
    $('#loader').style.display = ''; $('#stages').style.opacity = 1;
    $$('.stage').forEach((s) => s.classList.remove('on', 'ok'));
    $('#ld-note').textContent = 'прототип: байты и разбор идут по замеренному темпу; раскладка аналитическая и считается один раз при старте';
    nodes.forEach((n, i) => { const u = hash(i * 3.1) * 0.999; n.mu = u; n.a = 0; [n.mpx, n.mpy] = markPoint(u); }); // once, not per frame (was O(nodes x stroke points) every frame)
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
    if (t >= FETCH + PARSE) { L.layoutP = REDUCE ? 1 : clamp((t - FETCH - PARSE) / 1500, 0, 1); }
    const set = (id, on, ok, p, txt) => { const s = $('#st-' + id); s.classList.toggle('on', on); s.classList.toggle('ok', ok); s.querySelector('.pb i').style.transform = `scaleX(${p})`; $('#st-' + id + '-n').textContent = txt; };
    const mb = (FULL.bytes / 1e6);
    set('fetch', true, L.fetchP >= 1, L.fetchP, `${(mb * L.fetchP).toFixed(1)} / ${mb.toFixed(1)} МБ`);
    set('parse', L.fetchP >= 1, L.parseP >= 1, L.parseP, `${nf0.format(Math.round(FULL.nodes * L.parseP))} узлов · ${nf0.format(Math.round(FULL.edges * L.parseP))} связей`);
    set('layout', L.parseP >= 1, L.layoutP >= 1, L.layoutP, `рассчитана за ${LAYOUT_MS} мс · посадка`);
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
    ticks += nodes.filter((n) => n.session && n.age <= AGE_MAX).map((n) => `<span class="ss" data-i="${n.i}" style="left:${(sAt(-n.age) * 100).toFixed(2)}%"></span>`).join('');
    ticks += BIG_DAYS.map((b) => `<span class="bd" style="left:${(sAt(-b.age) * 100).toFixed(2)}%" title="${esc(b.label)} · ${b.n} записей"></span>`).join('');
    T2.innerHTML = `<button class="play" aria-label="Проиграть память">${ico('fin-income', 'pl')}</button><div class="tr"><div class="fill"></div>${ticks}<div class="hd" role="slider" tabindex="0" aria-label="Момент памяти"></div></div><span class="when num"></span><span class="say"></span>`;
    $('.play', T2).onclick = togglePlay;
    const tr = $('.tr', T2);
    let drag = false;
    const setX = (x) => { const r = tr.getBoundingClientRect(); state.replay.s = clamp((x - r.left) / r.width, 0, 1); state.replay.playing = false; };
    tr.addEventListener('pointerdown', (e) => { drag = true; tr.setPointerCapture(e.pointerId); setX(e.clientX); });
    tr.addEventListener('pointermove', (e) => {
      if (drag) { setX(e.clientX); return; }
      // sessions are time: hovering a session tick lights up everything it produced, wherever it sits
      let best = null, bd = 6;
      for (const el of tr.querySelectorAll('.ss')) { const d = Math.abs(el.getBoundingClientRect().left - e.clientX); if (d < bd) { bd = d; best = el; } }
      const si = best ? +best.dataset.i : null;
      if (si !== state.sessHi) {
        tr.querySelectorAll('.ss.on').forEach((x) => x.classList.remove('on')); if (best) best.classList.add('on');
        state.sessHi = si; sessSet = new Set(si == null ? [] : adj[si].filter(([j, r, dir]) => dir === 'out' && (r === 'contains' || r === 'related_to')).map(([j]) => j));
        for (const n of nodes) { n.bx = n.sx; n.by = n.sy; n.ba = n.a; } state.morph0 = performance.now() - 600;
      }
      if (si != null) tip(e, 'сессия · ' + sessSet.size + ' записей', stripProj(nodes[si].label)); else tip(null);
    });
    tr.addEventListener('pointerleave', () => { if (state.sessHi != null) { state.sessHi = null; sessSet = new Set(); tr.querySelectorAll('.ss.on').forEach((x) => x.classList.remove('on')); } tip(null); });
    tr.addEventListener('pointerup', () => { drag = false; });
    $('.hd', T2).addEventListener('keydown', (e) => { const d = e.key === 'ArrowLeft' ? -0.02 : e.key === 'ArrowRight' ? 0.02 : 0; if (d) { state.replay.s = clamp(state.replay.s + d, 0, 1); e.preventDefault(); } });
  }
  function togglePlay() {
    const R = state.replay;
    if (REDUCE) { R.s = R.s >= 0.999 ? 0 : 1; return; }       // reduced motion: the timeline jumps instead of playing
    if (!R.playing && R.s >= 0.999) R.s = 0;
    R.playing = !R.playing;
  }
  let sayT = 0, sessSet = new Set();
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
  let ggl = null;
  const EB = new Float32Array(16000 * 20), NB = new Float32Array(16000 * 16);
  let fitK = 1, lightGain = 1;
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
        if (e.kind !== 'mol') continue; // at rest: short links inside molecules only; no spokes, no attachment fans
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
      // theme spines: each theme is one branch from the project out to its tip
      if (!chain) for (const tn of themeNodes.values()) {
        const H = nodes[tn.hub]; if (!H || tn.a < 0.03 || !tn.tip) continue;
        const [tx, ty] = project({ x: tn.tip.ax, y: tn.tip.ay }); tn.tip.sx = tx; tn.tip.sy = ty;
        const al = (hp != null && hp !== tn.hub ? 0.05 : 0.28) * Math.min(tn.a, 1) * (focus != null ? 0.35 : 1);
        const w = 1 + Math.log2(1 + tn.count) * 0.3, c0 = GC[tn.g];
        putE(H, tn, 0.02, w, 0, 1, c0, al * 0.6, c0, al, tn.birth);
        putE(tn, tn.tip, 0.015, w * 0.8, 0, 1, c0, al, c0, al * 0.08, tn.birth);
      }
      // projects feed the root: one luminous axon per ganglion
      if (!chain) for (const G of ganglia.values()) {
        if (G.key < 0) continue;
        const H = nodes[G.key]; if (H.a < 0.05) continue;
        const al = (hp != null && hp !== G.key ? 0.06 : 0.35) * (loaderOn ? sstep(0.6, 1, L.layoutP) : 1) * (focus != null ? 0.3 : 1);
        putE(H, rootN, 0.06, 1.2 + Math.log2(1 + G.members.length) * 0.35, 0, 1, GC[G.dom], al * 0.5, [0.91, 0.9, 0.88], al);
      }
      // focus: the node's other links as thin dashed arcs (cross-molecule solves/refines/supersedes, context); never session lines
      if (focus != null && !chain) for (const [j, rel] of adj[focus]) {
        if (rel === 'belongs_to') continue;
        const A = nodes[focus], B = nodes[j];
        if (B.session || A.session || parent[j] === focus || parent[focus] === j) continue;
        const logical = rel === 'solves' || rel === 'refines' || rel === 'supersedes';
        const ca = colOf(A, 1).slice(), cb = colOf(B, 1).slice(), al = logical ? 0.75 : 0.35;
        for (let q = 0; q < 10; q++) putE(A, B, 0.22, 1.0, q / 10, q / 10 + 0.06, ca, al, cb, al); // dashes
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
      if ((n.dust || n.pool) && !isF && !near) { I *= 0.35; size = Math.min(size, 1.3); glyph = 0; halo = 0.6; }
      if (n.virtual) { size = 3.2 + Math.log2(1 + n.count) * 0.5; I = Math.max(I, 1.1) * Math.min(n.a, 1); glyph = 1; }
      let gl2 = glyph;
      if (iconZ > 0 && n.icon >= 0) { gl2 = glyph + iconZ; size = lerp(size, Math.max(size, 9), iconZ); ext = Math.max(ext, 2.4); } // far dot -> mid silhouette -> close icon
      putN(n.sx, n.sy, size, ext, colOf(n, sat), I, halo, n.shape, gl2, n.icon, n.birth, n.supAt);
    }
    // u8 light buffer: accumulate pre-scaled light (density-based exposure), expanded again in the tone map
    lightGain = ggl.light === 'u8' ? clamp(0.3 * Math.sqrt(700 / Math.max(nn, 1)), 0.1, 0.3) : 1;
    return ggl.draw({ gain: lightGain, vw: VW, vh: VH, ratio: DPR, edges: EB, nEdges: ne, nodes: NB, nNodes: nn, tNow: replayOn ? state.replay.t : 0, tFade: replayOn ? state.replay.fade : 0.001 });
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
  const themeEls = new Map();
  function buildThemeLabels() {
    const box = $('#m-themes'); box.innerHTML = '';
    for (const tn of themeNodes.values()) { const el = document.createElement('span'); el.className = 'tlabel'; el.innerHTML = `${esc(tn.label)} <span class="num">${nf0.format(tn.count)}</span>`; el.style.color = GHEX[tn.g]; box.append(el); themeEls.set(tn.i, el); }
  }
  function updateThemeLabels(scr, L) {
    const on = scr === 'memory' && (!L || L.done) && !state.chain && state.sessHi == null;
    const off = $('#s-memory').getBoundingClientRect().left;
    for (const tn of themeNodes.values()) {
      const el = themeEls.get(tn.i); if (!el || !tn.tip) continue;
      const H = nodes[tn.hub];
      const len = Math.hypot(tn.tip.sx - H.sx, tn.tip.sy - H.sy);
      const vis = on && len > 90 && tn.a > 0.2 && (state.hoverProj == null || state.hoverProj === tn.hub);
      el.classList.toggle('on', vis);
      if (!vis) continue;
      let ang = Math.atan2(tn.tip.sy - H.sy, tn.tip.sx - H.sx);
      const mx = H.sx + (tn.tip.sx - H.sx) * 0.55, my = H.sy + (tn.tip.sy - H.sy) * 0.55;
      let deg = ang * 180 / Math.PI; if (deg > 90 || deg < -90) deg += 180; // never upside down
      if (legendRect && mx > legendRect[0] - 40 && mx < legendRect[0] + legendRect[2] + 40 && my > legendRect[1] - 12 && my < legendRect[1] + legendRect[3] + 12) { el.classList.remove('on'); continue; }
      el.style.transform = `translate(${(mx - off).toFixed(1)}px, ${my.toFixed(1)}px) translate(-50%, -50%) rotate(${deg.toFixed(1)}deg) translateY(-9px)`;
    }
  }
  function updateLabels(scr, L) {
    const box = $('#m-labels');
    const on = scr === 'memory' && (!L || L.done);
    box.style.opacity = on ? (L ? sstep(0, 900, performance.now() - L.done) : 1) : 0;
    if (!on) return;
    gangliaGeom();
    updateLegend(cam.k / fitK);
    const placed = legendRect ? [legendRect] : []; // the legend is an obstacle: no project label lands on it
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
      const chainHide = !!state.chain; // in chain mode the project is already a step on the path
      const vis = !chainHide && !hit(x, y) && zr < 7 && (sector || rpx > 6) && x > 140 && x + w < VW - 10 && y > 6 && y + h < VH - 84 && !(state.sel != null && Math.abs(y + h - (nodes[state.sel].sy - 16)) < 30 && Math.abs(cx - nodes[state.sel].sx) < 240);
      el.style.transform = `translate(${(x - railOff).toFixed(1)}px, ${y.toFixed(1)}px)`;
      el.classList.toggle('off', !vis);
      el.classList.toggle('hl', state.hoverProj === G.key);
      el.classList.toggle('dim', state.hoverProj != null && state.hoverProj !== G.key);
      if (vis) placed.push([x, y, w, h]);
      const pmEl = el.querySelector('.pm');
      if (state.replay.s < 0.999) { const born = G.members.filter((m) => m.birth <= state.replay.t).length; el.classList.toggle('off', !vis || born === 0); pmEl.dataset.r = '1'; pmEl.innerHTML = `<span class="num">${nf0.format(born)}</span> в выборке к этой дате`; }
      else if (pmEl.dataset.r) { delete pmEl.dataset.r; const info = G.info; pmEl.innerHTML = info ? `<span class="num">${nf0.format(info.n)}</span> узлов <i class="act ${info.last < 7 ? 'hot' : ''}"></i>${agoTxt(info.last)}` : `<span class="num">${G.members.length}</span> в выборке`; }
    }
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
      <dt>без связей</dt><dd class="num">${info && info.lonely != null ? Math.round(100 * info.lonely / info.n) + '% проекта' : '—'}</dd>
      <dt>тем</dt><dd class="num">${[...themeNodes.values()].filter((t) => t.hub === G.key).length}</dd></dl>
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
  let legendMode = '', legendRect = null;
  function measureLegend() { const r = $('#m-legend').getBoundingClientRect(); legendRect = r.width ? [r.left - 8, r.top - 8, r.width + 16, r.height + 16] : null; }
  function updateLegend(zr) {
    const mode = zr < 2.6 ? 'over' : 'near';
    if (mode === legendMode) return; legendMode = mode;
    requestAnimationFrame(measureLegend);
    const L = $('#m-legend');
    const dot = (hex, s = 8) => `<i class="sw" style="width:${s}px;height:${s}px;background:${hex};box-shadow:0 0 8px ${hex}"></i>`;
    if (mode === 'over') {
      L.innerHTML = `<p class="lg"><span class="axon"></span>ветка — тема проекта, у основания старое, к концу новое</p>
        <p class="lg"><span class="neb"></span>сгусток на ветке — молекула: проблема и решения, линия версий, задача с журналом</p>
        <p class="lg"><span class="core"></span>яркое ядро — записано за 7 дней</p>
        <p class="lg">${ico('task', '', 'color:' + GHEX.w)}активная задача</p>
        <p class="lg">${ico('problem', '', 'color:' + GHEX.p)}открытая проблема</p>
        <p class="lg"><span class="core"></span>пыль — записи без связей; сессии — засечки на шкале времени</p>
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
  let glShown = false; // until the GL field is up, the 2D path draws the graph
  let last = performance.now(), fpsAcc = [], mouse = { x: -1e4, y: -1e4 }, vignette = null;
  const FPROF = params.has('prof') ? (window.__fprof = []) : null;
  function frame(now) {
    const fT0 = FPROF ? performance.now() : 0;
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
      if (!L.fitDone && !camFly) { Object.assign(cam, fitTarget()); fitK = cam.k; L.fitDone = true; } // the layout is final: fit once (bounds() sorts all nodes)
      const mx = markXf();
      for (const n of nodes) {
        const mx0 = n.mpx, my0 = n.mpy;
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
    if (!glShown && edgeA > 0) {
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

    if (!glShown) {
    c.globalCompositeOperation = 'lighter';
    const zk = clamp(Math.pow(cam.k / (fitK || 1), 0.35), 0.8, 2.4);
    for (const n of nodes) {
      if (n.a <= 0.01) continue;
      if (L && !L.done) { c.globalAlpha = Math.min(n.a, 1) * 0.8; c.fillStyle = GHEX[n.g] || '#e8e6e1'; c.fillRect(n.sx - 0.9, n.sy - 0.9, 1.8, 1.8); continue; } // cheap particles while GL compiles
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
    updateThemeLabels(scr, L);
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
    if (!glass) { fbCtx.setTransform(1, 0, 0, 1, 0, 0); fbCtx.drawImage(scene, 0, 0); }
    if (glass) {
      let obj = { x: 0, y: 0, z: 0, size: 1, extent: 1.1, morph: 1, flat: 1, presence: 0, thick: 0.05, inv: I3 };
      if (L && !REDUCE) {
        const mx = markXf();
        const pres = sstep(0.7, 1, L.fetchP) * (1 - sstep(0.82, 1, L.layoutP));
        const morph = quintic(sstep(0.42, 0.72, L.layoutP));
        const flat = 1 - 0.96 * sstep(0.7, 0.95, L.layoutP);
        obj = { x: mx.cx, y: mx.cy, z: 0.3 * mx.s * MK.R, size: mx.s * MK.R, extent: 1.12, morph, flat, presence: pres, thick: 0.045, inv: I3, aa: false };
      }
      if (!ggl && glass.ready()) { ggl = window.GraphGL.init(glass.gl); buildAtlas(ggl); if ($('#lightbuf')) $('#lightbuf').textContent = 'light buffer: ' + ggl.light; }
      const graphTex = ggl ? buildGraph(now, scr, L) : null;
      const shown = glass.frame({ scene, slabs: collectSlabs(), obj, vw: VW, vh: VH, ratio: DPR, graphTex, graphGain: lightGain });
      if (!shown) { fbCtx.setTransform(1, 0, 0, 1, 0, 0); fbCtx.drawImage(scene, 0, 0); } // shaders still compiling: show the 2D scene
      document.body.classList.toggle('nogl', !shown);
      glShown = !!shown && !!ggl && ggl.ready();
    }
    if (params.has('fps') && $('#fps')) { const avg = fpsAcc.reduce((x, y) => x + y, 0) / fpsAcc.length; $('#fps').textContent = `${(1 / avg).toFixed(0)} fps · ${(avg * 1000).toFixed(1)} мс/кадр`; }
    window.__frames = (window.__frames || 0) + 1;
    if (FPROF) FPROF.push([performance.now() - fT0, state.loader ? (state.loader.parseP < 1 ? (state.loader.fetchP < 1 ? 'fetch' : 'parse') : 'layout') : 'run']);
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
  await yieldFrame();
  buildLegend(); statsLine();
  await yieldFrame();
  buildLabels(); buildThemeLabels(); buildTimeline();
  if ($('#lightbuf')) $('#lightbuf').textContent = 'light buffer: ' + (glass ? '…' : '2d');
  await yieldFrame();
  resize();
  await yieldFrame();
  document.fonts.ready.then(() => { if (state.screen === 'finance') renderFin(); });
  if (params.has('skip')) {
    newSim(); stepSim(TICKS); fitCamera(true);
    nodes.forEach((n) => { const [x, y] = project(n); n.sx = n.bx = x; n.sy = n.by = y; n.a = n.ba = 0.8; });
    $('#loader').style.display = 'none'; $('#rail').style.opacity = 1; $('#cmd').style.opacity = 1;
    setScreen(initialScreen(), true);
    if (params.get('focus') === 'expand') select(GA.expand, true);
  } else startLoader();
  preOn = false;
  if (params.has('task')) { state.task = params.get('task'); ensureBuilt('tasks'); showTask(); }
  requestAnimationFrame(frame);
  window.__au = { state, setScreen, select, nodes, cam, startLoader, enterChain, exitChain, togglePlay, setReplay: (v) => { state.replay.s = v; state.replay.playing = false; }, chainOf, ganglia, gangliaGeom, flyToGanglion, fitK: () => fitK, project, pulses, adj };
})();
