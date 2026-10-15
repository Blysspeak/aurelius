import re, sys

def paths(fn):
    s = open(fn).read()
    return re.findall(r'<path d="([^"]+)"', s, re.S)

out = sys.argv[1]
sil = ' '.join(p.replace('\n', ' ') for p in paths('/tmp/aud/sil.svg'))
head = ' '.join(p.replace('\n', ' ') for p in paths('/tmp/aud/head_solid.svg'))
word = ' '.join(p.replace('\n', ' ') for p in paths('/tmp/aud/word.svg'))

C = (512, 511.5)
N = {'A': (591.1, 319.1), 'B': (528.2, 393.1), 'C': (658.2, 393.1),
     'D': (513.8, 497.6), 'E': (626.8, 497.7), 'F': (695.1, 497.5)}
E = [('A', 'B'), ('A', 'C'), ('B', 'D'), ('B', 'E'), ('C', 'D'), ('C', 'E'), ('C', 'F')]

def net(r=16, sw=7):
    lines = ''.join(f'<line x1="{N[a][0]}" y1="{N[a][1]}" x2="{N[b][0]}" y2="{N[b][1]}"/>' for a, b in E)
    dots = ''.join(f'<circle cx="{x}" cy="{y}" r="{r}"/>' for x, y in N.values())
    return (f'<g class="au-net" stroke="currentColor" stroke-width="{sw}" stroke-linecap="round">{lines}</g>'
            f'<g class="au-nodes" fill="currentColor">{dots}</g>')

def rings():
    return (f'<circle class="au-ring" cx="{C[0]}" cy="{C[1]}" r="338" fill="none" stroke="currentColor" stroke-width="10"/>'
            f'<circle class="au-ring-inner" cx="{C[0]}" cy="{C[1]}" r="308" fill="none" stroke="currentColor" stroke-width="3"/>')

T = 'transform="translate(0 1024) scale(0.1 -0.1)"'

def line_mark():
    return (rings() +
            f'<path class="au-head" {T} d="{sil}" fill="none" stroke="currentColor" stroke-width="90" stroke-linejoin="round"/>' +
            net())

def solid_mark():
    return (rings() +
            f'<path class="au-head" {T} d="{head}" fill="currentColor"/>' +
            ''.join(f'<circle cx="{x}" cy="{y}" r="12" fill="currentColor"/>' for x, y in N.values()))

hdr = '<svg xmlns="http://www.w3.org/2000/svg" role="img" aria-label="aurelius" '

mark = hdr + 'viewBox="150 150 724 724" width="724" height="724">' + line_mark() + '</svg>\n'
solid = hdr + 'viewBox="150 150 724 724" width="724" height="724">' + solid_mark() + '</svg>\n'

# lockup: mark 724 high, word scaled to 0.9, placed right of mark
s = 0.82
ww, wh = 1717 * s, 329 * s
x0 = 874 + 110
y0 = 150 + (724 - wh) / 2
lock = (hdr + f'viewBox="150 150 {x0 - 150 + ww:.0f} 724" width="{x0 - 150 + ww:.0f}" height="724">' + line_mark() +
        f'<g transform="translate({x0:.1f} {y0:.1f}) scale({s})"><path class="au-word" transform="translate(0 329) scale(0.1 -0.1)" d="{word}" fill="currentColor"/></g></svg>\n')

open(out + '/logo-mark.svg', 'w').write(mark)
open(out + '/logo-mark-solid.svg', 'w').write(solid)
open(out + '/logo.svg', 'w').write(lock)
# pieces for inlining in the prototype
open('/tmp/aud/sil_d.txt', 'w').write(sil)
open('/tmp/aud/word_d.txt', 'w').write(word)
print(len(mark), len(solid), len(lock))
