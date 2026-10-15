"""Build ui/design/snapshot.js from real au exports in /tmp/aud (taken 2026-10-15)."""
import json, random, collections, sys, datetime

random.seed(7)
OUT = sys.argv[1]
FULL = '--full' in sys.argv[2:]          # every node and edge of the graph, not the ~3.5k sample
d = json.load(open('/tmp/aud/graph.json'))
nodes = {x['id']: x for x in d['nodes']}
edges = d['edges']

def ntype(x):
    t = x['node_type']
    return t if isinstance(t, str) else 'doc'

TYPES = ['decision', 'solution', 'problem', 'concept', 'work_log', 'session', 'task', 'user_fact',
         'doc', 'project', 'file', 'dependency', 'server', 'skill', 'config', 'digest', 'person', 'module']
TI = {t: i for i, t in enumerate(TYPES)}
CONF = [None, 'measured', 'inferred', 'reported', 'unverified']
RELS = sorted({e['relation'] for e in edges})
RI = {r: i for i, r in enumerate(RELS)}

# project membership
proj_of = {}
for e in edges:
    if e['relation'] == 'belongs_to' and e['to_id'] in nodes and ntype(nodes[e['to_id']]) == 'project':
        proj_of[e['from_id']] = e['to_id']
members = collections.defaultdict(list)
for k, p in proj_of.items():
    members[p].append(k)
top = sorted(members, key=lambda p: -len(members[p]))[:14]

# ---- themes (owner order 15.10): theme = subject prefix (project:segment), numeric segments are spec/task numbers (#036).
# Inheritance for nodes without a subject, in order, to a fixpoint over the FULL graph:
#   1) from the node they solve / refine / supersede, 2) from their containing task or work_log,
#   3) from the task their session relates to. au exposes no vectors, so the rest stays "без темы".
import re
def seg_theme(x, plabel):
    sj = (x.get('data') or {}).get('subject')
    if not sj: return None
    seg = sj.split(':')
    if len(seg) < 2 or not seg[1]: return None
    if seg[0].lower() != plabel.lower() and not plabel.lower().startswith(seg[0].lower()): return None
    t = seg[1].split('/')[0]
    return ('#' + t) if t.isdigit() else t[:28]
by_rel = collections.defaultdict(list)
for e in edges:
    if e['from_id'] in nodes and e['to_id'] in nodes:
        by_rel[e['relation']].append((e['from_id'], e['to_id']))
sess_task = collections.defaultdict(list)        # session -> tasks it relates to
for a, b in by_rel['related_to']:
    if ntype(nodes[a]) == 'session' and ntype(nodes[b]) == 'task': sess_task[a].append(b)
    if ntype(nodes[b]) == 'session' and ntype(nodes[a]) == 'task': sess_task[b].append(a)
contained_in = collections.defaultdict(list)
for a, b in by_rel['contains']:
    contained_in[b].append(a)
theme_full, coverage = {}, {}
for p in top:
    plabel = nodes[p]['label']
    mem = [k for k in members[p] if k in nodes and k != p]
    th = {k: seg_theme(nodes[k], plabel) for k in mem}
    before = sum(1 for v in th.values() if v)
    mset = set(mem)
    for _ in range(6):
        changed = 0
        for rel in ('solves', 'refines', 'supersedes'):
            for a, b in by_rel[rel]:
                if a in mset and b in mset and th.get(a) is None and th.get(b): th[a] = th[b]; changed += 1
        for k in mem:
            if th[k] is None:
                for c in contained_in.get(k, []):
                    if c in mset and ntype(nodes[c]) in ('task', 'work_log') and th.get(c): th[k] = th[c]; changed += 1; break
        for k in mem:
            if th[k] is None:
                for c in contained_in.get(k, []):
                    if ntype(nodes[c]) == 'session':
                        t = next((th.get(t2) for t2 in sess_task.get(c, []) if th.get(t2)), None)
                        if t: th[k] = t; changed += 1; break
        if not changed: break
    after = sum(1 for v in th.values() if v)
    coverage[plabel] = [len(mem), before, after]
    theme_full.update({k: v for k, v in th.items() if v})
# ---- harder inference for linked nodes without a theme (orchestrator, 15.10). A molecule = component over the
# logical relations the UI builds its chains from, inside one project, sessions and projects excluded.
MOLREL = {'subtask_of', 'contains', 'solves', 'refines', 'supersedes', 'implements', 'blocks', 'caused_by', 'depends_on', 'prerequisite', 'next_step', 'verified_by'}
uf = {}
def find(a):
    while uf.get(a, a) != a:
        uf[a] = uf.get(uf[a], uf[a]); a = uf[a]
    return a
for e in edges:
    a, b, r = e['from_id'], e['to_id'], e['relation']
    if r not in MOLREL or a not in nodes or b not in nodes: continue
    if r == 'contains' and ntype(nodes[a]) not in ('task', 'work_log'): continue
    if 'session' in (ntype(nodes[a]), ntype(nodes[b])) or 'project' in (ntype(nodes[a]), ntype(nodes[b])): continue
    if proj_of.get(a) is None or proj_of.get(a) != proj_of.get(b): continue
    ra, rb = find(a), find(b)
    if ra != rb: uf[ra] = rb
mols = collections.defaultdict(list)
for k in proj_of:
    if k in nodes and ntype(nodes[k]) not in ('session', 'project'): mols[find(k)].append(k)
KEEP = {}
for p in top:
    cnt0 = collections.Counter(theme_full[k] for k in members[p] if k in theme_full)
    KEEP[p] = [t for t, _ in cnt0.most_common(12)]
RU = {'antifraud': ['антифрод'], 'payment': ['платеж', 'платёж'], 'window': ['окно'], 'bank': ['банк'], 'workspace': ['воркспейс'],
      'cabinet': ['кабинет'], 'release': ['релиз'], 'payouts': ['выплат'], 'prod': ['прод'], 'stand': ['стенд'], 'overpay': ['оверпей'],
      'scutum': ['скутум'], 'design': ['дизайн'], 'backend': ['бэкенд'], 'frontend': ['фронтенд'], 'mask': ['маск'], 'audit': ['аудит'],
      'hooks': ['хук'], 'gate': ['гейт'], 'dispatch': ['диспетч'], 'snapshot': ['снапшот', 'снимок'], 'finance': ['финанс'], 'reminders': ['напоминан'],
      'covers': ['обложк'], 'publish': ['публикац'], 'spec': ['спек'], 'task': ['задач'], 'client': ['клиент'], 'ui': ['интерфейс']}
def aliases(name):
    n = name.lower(); out = {n}
    m = re.fullmatch(r'#(\d+)', n)
    if m: out |= {m.group(1), 't' + m.group(1)}
    seg = [x for x in re.split(r'[-_/\s]+', n) if x]
    if len(seg) > 1: out |= {' '.join(seg), ''.join(seg)}
    m = re.fullmatch(r'([a-z]+)(\d+)', n)
    if m: out.add(m.group(1) + ' ' + m.group(2)); out |= {r + m.group(2) for r in RU.get(m.group(1), [])}
    if len(seg) == 1 and seg[0] in RU: out |= set(RU[seg[0]])
    if len(seg) > 1 and all(x in RU for x in seg): out.add(' '.join(RU[x][0] for x in seg))
    return {a for a in out if len(a) >= 3}
APAT = {p: [(t, re.compile(r'(?<![\w#])(' + '|'.join(re.escape(a) for a in sorted(aliases(t), key=len, reverse=True)) + r')', re.I)) for t in KEEP[p] if aliases(t)] for p in top}
def text_of(k):
    x = nodes[k]; dd = x.get('data') or {}
    return ((x.get('label') or '') + ' ' + (dd.get('claim') or '')).lower()
def kw(p, k):
    hits = {t for t, rx in APAT.get(p, []) if rx.search(text_of(k))}
    return hits.pop() if len(hits) == 1 else None
linked = {k for r, ms in mols.items() if len(ms) > 1 for k in ms}
def themeless():
    c = collections.Counter(proj_of[k] for k in linked if k not in theme_full and proj_of.get(k) in members)
    return {nodes[p]['label']: c.get(p, 0) for p in top}
INFER = {'before': themeless()}
for root, ms in mols.items():                                  # a. molecule majority
    if len(ms) < 2: continue
    c = collections.Counter(theme_full[k] for k in ms if k in theme_full)
    if c:
        t = c.most_common(1)[0][0]
        for k in ms: theme_full.setdefault(k, t)
INFER['a_majority'] = themeless()
for root, ms in mols.items():                                  # b. keyword match, unique only; a molecule takes it whole
    if len(ms) < 2 or all(k in theme_full for k in ms): continue
    p = proj_of.get(ms[0]); found = {kw(p, k) for k in ms if k not in theme_full} - {None}
    if len(found) == 1:
        t = found.pop()
        for k in ms: theme_full.setdefault(k, t)
INFER['b_keyword'] = themeless()
for root, ms in mols.items():                                  # c. the molecule's task label decides
    if len(ms) < 2 or all(k in theme_full for k in ms): continue
    p = proj_of.get(ms[0]); found = {kw(p, k) for k in ms if ntype(nodes[k]) == 'task'} - {None}
    if len(found) == 1:
        t = found.pop()
        for k in ms: theme_full.setdefault(k, t)
INFER['c_task'] = themeless()
# the remainder: "без темы" branches by month of the molecule's newest member; more than 6 months -> older merged
MN_RU = ['янв', 'фев', 'мар', 'апр', 'май', 'июн', 'июл', 'авг', 'сен', 'окт', 'ноя', 'дек']
month_of = {}
for root, ms in mols.items():
    if len(ms) < 2 or all(k in theme_full for k in ms): continue
    newest = max(nodes[k]['created_at'][:7] for k in ms)
    for k in ms:
        if k not in theme_full: month_of[k] = newest
NOTHEME = {}
for p in top:
    mon = sorted({m for k, m in month_of.items() if proj_of.get(k) == p}, reverse=True)
    recent = mon[:6]
    for k, m in month_of.items():
        if proj_of.get(k) == p: NOTHEME[k] = ('без темы · ' + MN_RU[int(m[5:7]) - 1]) if m in recent else 'без темы · раньше'
INFER['d_month_branches'] = {nodes[p]['label']: sum(1 for k in NOTHEME if proj_of.get(k) == p) for p in top}
print('inference', json.dumps(INFER, ensure_ascii=False))
THEMES = []
TINDEX = {}
for p in top:
    plabel = nodes[p]['label']
    cnt = collections.Counter(theme_full[k] for k in members[p] if k in theme_full)
    keep = [t for t in KEEP[p] if cnt.get(t)]
    for t in keep:
        TINDEX[(p, t)] = len(THEMES); THEMES.append([plabel, t, cnt[t]])
    rest = sum(c for t, c in cnt.items() if t not in keep)
    if rest:
        TINDEX[(p, '…')] = len(THEMES); THEMES.append([plabel, 'другие темы', rest])
    nt = collections.Counter(NOTHEME[k] for k in members[p] if k in NOTHEME)
    for t, c in sorted(nt.items(), key=lambda kv: (kv[0] == 'без темы · раньше', kv[0])):
        TINDEX[(p, t)] = len(THEMES); THEMES.append([plabel, t, c])
def theme_idx(k):
    p = proj_of.get(k)
    t = theme_full.get(k) or NOTHEME.get(k)
    if p is None or t is None: return -1
    return TINDEX.get((p, t), TINDEX.get((p, '…'), -1))
print('coverage', json.dumps(coverage, ensure_ascii=False))

adj_nb = collections.defaultdict(list)
for e in edges:
    if e['relation'] != 'belongs_to':
        adj_nb[e['from_id']].append(e['to_id']); adj_nb[e['to_id']].append(e['from_id'])
logic_nb = collections.defaultdict(list)
for e in edges:
    if e['relation'] in ('solves', 'refines', 'supersedes', 'subtask_of', 'implements', 'caused_by', 'depends_on', 'blocks') or (e['relation'] == 'contains' and e['from_id'] in nodes and ntype(nodes[e['from_id']]) in ('task', 'work_log')):
        logic_nb[e['from_id']].append(e['to_id']); logic_nb[e['to_id']].append(e['from_id'])
FOCUS = next(k for k in nodes if k.startswith('85f5fa07'))
adj = collections.defaultdict(set)
for e in edges:
    adj[e['from_id']].add(e['to_id']); adj[e['to_id']].add(e['from_id'])

pick = set(top)
pick.add(FOCUS)
hop1 = adj[FOCUS]
pick |= hop1
EXP = next(k for k in nodes if k.startswith('d310c206'))
pick.add(EXP)
pick |= adj[EXP]
for h in list(hop1):
    if ntype(nodes[h]) != 'project':
        pick |= set(list(adj[h])[:6])
total = sum(len(members[p]) for p in top)
BUDGET = 3400
for p in top:
    m = members[p]
    k = max(25, int(BUDGET * len(m) / total))
    # per theme quota, then grow molecules over logical edges only (no session edges: sessions are time, not structure)
    mset = set(m); got = set()
    LOGIC = {'solves', 'refines', 'supersedes', 'subtask_of', 'contains', 'implements', 'caused_by', 'depends_on', 'blocks'}
    def grow(seed, cap, same_theme):
        q = [seed]
        while q and len(got) < cap:
            u = q.pop(0)
            if u in got: continue
            got.add(u)
            for v in logic_nb[u]:
                if v in mset and v not in got and ntype(nodes[v]) != 'session' and (not same_theme or theme_full.get(v) in (None, same_theme)): q.append(v)
    themed = collections.defaultdict(list)
    for x in m:
        if theme_idx(x) >= 0 and THEMES[theme_idx(x)][1] != 'другие темы': themed[theme_full.get(x) or NOTHEME.get(x)].append(x)
    tot_t = sum(len(v) for v in themed.values()) or 1
    for t, xs in themed.items():
        cap = len(got) + max(12, int(k * 0.8 * len(xs) / tot_t))
        random.shuffle(xs)
        xs.sort(key=lambda z: -len(logic_nb[z]))           # molecule cores first
        for sd in xs:
            if len(got) >= cap: break
            if sd not in got: grow(sd, cap, t)
    seeds = random.sample(m, len(m))
    for sd in seeds:
        if len(got) >= k: break
        if sd not in got and ntype(nodes[sd]) != 'session': grow(sd, k, None)
    pick |= got
tl0 = json.load(open('/tmp/aud/tasks.json'))
pick |= {t['id'] for t in tl0}
if FULL:
    pick = list(nodes)
pick = [k for k in pick if k in nodes]
idx = {k: i for i, k in enumerate(pick)}
solved = {e['to_id'] for e in edges if e['relation'] == 'solves'} | {e['from_id'] for e in edges if e['relation'] == 'solves'}
NOW = datetime.datetime(2026, 10, 15, 19, 0, tzinfo=datetime.timezone.utc)
def days(ts): return (NOW - datetime.datetime.fromisoformat(ts[:19] + '+00:00')).total_seconds() / 86400
def pstats(p):
    ms = [nodes[k] for k in members[p] if k in nodes]
    last = min(days(x.get('updated_at') or x['created_at']) for x in ms)
    act = sum(1 for x in ms if ntype(x) == 'task' and (x.get('data') or {}).get('status') == 'active')
    blk = sum(1 for x in ms if ntype(x) == 'task' and (x.get('data') or {}).get('status') == 'blocked')
    opn = sum(1 for x in ms if ntype(x) == 'problem' and x['id'] not in solved)
    lonely = sum(1 for x in ms if not adj_nb.get(x['id']))   # nothing but project membership
    return [idx[p], nodes[p]['label'], len(members[p]), round(last, 2), act, opn, blk, lonely]

def short(s, n=90):
    s = (s or '').replace('\n', ' ')
    return s if len(s) <= n else s[:n - 1] + '…'

N = []
for k in pick:
    x = nodes[k]
    dd = x.get('data') or {}
    age = round((datetime.datetime(2026, 10, 15, 16, 0, tzinfo=datetime.timezone.utc) - datetime.datetime.fromisoformat(x['created_at'][:19] + '+00:00')).total_seconds() / 86400, 3)
    st = dd.get('status') if ntype(x) == 'task' else None
    N.append([TI.get(ntype(x), TI['doc']), CONF.index(dd.get('confidence')) if dd.get('confidence') in CONF else 0,
              short(x['label'], 80), max(age, 0), st or 0, 1 if ntype(x) == 'problem' and k not in solved else 0,
              theme_idx(k)])
E = []
seen = set()
for e in edges:
    a, b = idx.get(e['from_id']), idx.get(e['to_id'])
    if a is None or b is None or a == b:
        continue
    key = (min(a, b), max(a, b))
    if key in seen:
        continue
    seen.add(key)
    E += [a, b, RI[e['relation']]]

def full(k):
    x = nodes[k]
    dd = x.get('data') or {}
    ev = dd.get('evidence')
    if isinstance(ev, list):
        ev = '; '.join(f"{v.get('command')} -> exit {v.get('exit_code')}" for v in ev[-3:])
    links = []
    for e in edges:
        if e['from_id'] == k and e['to_id'] in idx:
            links.append([e['relation'], idx[e['to_id']], 'out'])
        elif e['to_id'] == k and e['from_id'] in idx:
            links.append([e['relation'], idx[e['from_id']], 'in'])
    return {
        'i': idx[k], 'id': k, 'type': ntype(x), 'label': x['label'], 'note': x.get('note'),
        'claim': dd.get('claim'), 'subject': dd.get('subject'), 'confidence': dd.get('confidence'),
        'evidence': ev, 'volatility': dd.get('volatility'), 'measured_at': dd.get('measured_at'),
        'created_at': x['created_at'], 'source': x.get('source'), 'access_count': x.get('access_count'),
        'links': links[:40],
    }

# expanded node: the richest decision in the focus branch with evidence
cands = [h for h in hop1 if ntype(nodes[h]) in ('decision', 'solution') and (nodes[h].get('data') or {}).get('evidence')]
cands.sort(key=lambda h: -len(nodes[h].get('note') or ''))
details = {}
for k in [FOCUS, EXP] + cands[:5]:
    details[idx[k]] = full(k)

type_counts = collections.Counter(ntype(x) for x in d['nodes'])
rel_counts = collections.Counter(e['relation'] for e in edges)
conf_counts = collections.Counter((x.get('data') or {}).get('confidence') or 'none' for x in d['nodes'])

import hashlib, os, struct
STATUS = [None, 'backlog', 'active', 'blocked', 'done', 'cancelled']
def big_days():
    # the days with the most writing, named by a release or a task of that day (was computed in app.js)
    by = collections.defaultdict(list)
    for i, r in enumerate(N): by[int(r[3] // 1)].append(i)
    top7 = sorted(by.items(), key=lambda kv: -len(kv[1]))[:7]
    out = []
    for d, arr in top7:
        lab = lambda i: N[i][2]
        rel = next((i for i in arr if re.search(r'релиз|release|v\d+\.\d+', lab(i), re.I)), None)
        if rel is None: rel = next((i for i in arr if TYPES[N[i][0]] == 'task'), None)
        if rel is None: rel = next((i for i in arr if TYPES[N[i][0]] == 'decision'), arr[0])
        t = re.sub(r'^T\d+\s*', '', re.sub(r'^\[[^\]]+\]\s*', '', lab(rel)))
        out.append({'age': d + 0.5, 'n': len(arr), 'label': t if len(t) <= 34 else t[:33] + '…'})
    return sorted(out, key=lambda b: -b['age'])
BIN = None
if FULL:
    # compact binary graph: typed arrays only; labels go to a separate text file loaded on demand
    n, ne = len(N), len(E) // 3
    def pad(b): return b + b'\0' * (-len(b) % 4)
    body = b'AUG2' + struct.pack('<III', 1, n, ne)
    body += pad(bytes(r[0] for r in N) + bytes(r[1] for r in N) + bytes(STATUS.index(r[4]) if r[4] in STATUS else 0 for r in N) + bytes(r[5] for r in N))
    body += pad(struct.pack('<%dh' % n, *[r[6] for r in N]))
    body += struct.pack('<%df' % n, *[r[3] for r in N])
    body += struct.pack('<%dI' % ne, *E[0::3]) + struct.pack('<%dI' % ne, *E[1::3])
    body += pad(bytes(E[2::3]))
    txt = '\n'.join(r[2].replace('\n', ' ') for r in N) + '\n'
    base = os.path.dirname(os.path.abspath(OUT))
    open(os.path.join(base, 'graph-full.bin'), 'wb').write(body)
    open(os.path.join(base, 'graph-full.txt'), 'w').write(txt)
    BIN = {'url': 'graph-full.bin', 'txt': 'graph-full.txt', 'bytes': len(body), 'txtBytes': len(txt.encode()), 'n': n, 'e': ne}
    GHASH = hashlib.sha1(body).hexdigest()[:16]
else:
    GHASH = hashlib.sha1(json.dumps([N, E], ensure_ascii=False).encode()).hexdigest()[:16]
graph = {
    'types': TYPES, 'conf': CONF, 'rels': RELS, 'nodes': [] if FULL else N, 'edges': [] if FULL else E, 'focus': idx[FOCUS],
    'bin': BIN, 'hash': GHASH, 'inference': INFER, 'status': STATUS, 'bigDays': big_days(),
    'expand': idx[EXP], 'details': details,
    'projects': [pstats(p) for p in top],
    'themes': THEMES, 'coverage': coverage,
    'full': {'nodes': len(d['nodes']), 'edges': len(edges), 'bytes': 81809155,
             'types': type_counts.most_common(), 'rels': rel_counts.most_common(), 'conf': conf_counts.most_common()},
}

# ---- tasks
tl = json.load(open('/tmp/aud/tasks.json'))
def tdetail(prefix):
    k = next(i for i in nodes if i.startswith(prefix))
    x = nodes[k]; dd = x.get('data') or {}
    logs, decisions, problems = [], [], []
    for e in edges:
        if e['from_id'] == k and e['relation'] == 'contains' and e['to_id'] in nodes:
            c = nodes[e['to_id']]
            t = ntype(c)
            item = {'t': c['created_at'], 'text': short(c.get('note') or c['label'], 260)}
            (logs if t == 'work_log' else decisions if t == 'decision' else problems if t == 'problem' else []).append(item)
    for L in (logs, decisions, problems):
        L.sort(key=lambda z: z['t'])
    subs = [{'id': s, 'label': nodes[s]['label'], 'status': (nodes[s].get('data') or {}).get('status')}
            for s in nodes if ntype(nodes[s]) == 'task' and any(
                e['from_id'] == s and e['to_id'] == k and e['relation'] == 'subtask_of' for e in edges)]
    return {'id': k, 'label': x['label'], 'note': x.get('note'), 'data': {kk: dd.get(kk) for kk in (
        'acceptance_criteria', 'criteria_met', 'status', 'priority', 'git', 'resolution', 'activated_at', 'closed_at',
        'confidence', 'started_at', 'due')}, 'created_at': x['created_at'], 'logs': logs, 'decisions': decisions,
        'problems': problems, 'subtasks': subs}

tasks = {'prs': json.load(open('/tmp/aud/prs.json')), 'list': tl, 'gidx': {t['id']: idx[t['id']] for t in tl if t['id'] in idx}, 'detail': {p: tdetail(p) for p in ['85f5fa07', '7a721a0f', 'b7a7d9af', '6b03e425', 'f0713c63']}}

# ---- finance (au fin ... --json, period all)
sc = json.load(open('/tmp/aud/stats_cat.json'))
sm = json.load(open('/tmp/aud/stats_month.json'))
ls = json.load(open('/tmp/aud/listall.json'))['entries']
CATS = sorted({e['category'] for e in ls if e.get('category')})
CI = {c: i for i, c in enumerate(CATS)}
cats_n = collections.Counter(e['category'] for e in ls if not e.get('voided_at'))
keep = ('id', 'kind', 'amount', 'currency', 'counter_amount', 'counter_currency', 'category', 'note', 'occurred_at',
        'void_reason', 'voided_at', 'source')
def ent(e):
    r = {k: e.get(k) for k in keep}
    u = e.get('usd')
    r['usd'] = u and {'usd': u['usd'], 'units_per_usd': u.get('units_per_usd'), 'source': u['source_label'],
                      'rate_date': u['rate_date'], 'stale': u['stale']}
    r['note'] = short(r['note'], 110)
    return r
recent = [ent(e) for e in ls[:90]]
voided = [ent(e) for e in ls if e.get('voided_at')][:8]
exch = [ent(e) for e in ls if e['kind'] == 'exchange' and not e.get('voided_at')]
fin = {
    'taken_at': '2026-10-15T19:02', 'command': 'au fin stats --period all --json',
    'usd': {k: sc['usd'][k] for k in ('expense', 'income', 'converted_entries', 'sources', 'stale_entries', 'unconverted')},
    'categories': [dict(g, n=cats_n.get(g['key'], 0)) for g in sc['usd']['groups']],
    'months': sm['usd']['groups'], 'native': sc['totals'], 'voided_excluded': sc['voided_excluded'],
    'entries_total': len(ls),
    'cats': CATS,
    'entries': [[e['id'], (datetime.datetime.fromisoformat(e['occurred_at'].replace('Z','+00:00')).astimezone().strftime('%Y-%m-%d')),
                 ['expense', 'income', 'exchange'].index(e['kind']), e['amount'], e['currency'], e.get('counter_amount'), e.get('counter_currency'),
                 CI.get(e.get('category'), -1), short(e.get('note'), 110), (e.get('usd') or {}).get('usd_minor') or 0,
                 (e.get('usd') or {}).get('source_label'), (e.get('usd') or {}).get('units_per_usd'), 1 if e.get('voided_at') else 0,
                 e.get('void_reason')] for e in ls], 'recent': recent, 'voided': voided, 'exchanges': exch,
}

js = ('// Real snapshot of the owner\'s aurelius database, 2026-10-15 19:02 local.\n'
      '// Built by /tmp/aud/build_snapshot.py from /api/graph, au task list --json, au fin stats/list --json.\n'
      'window.AU = ' + json.dumps({'graph': graph, 'tasks': tasks, 'fin': fin}, ensure_ascii=False, separators=(',', ':')) + ';\n')
open(OUT, 'w').write(js)
print('nodes', len(N), 'edges', len(E) // 3, 'bytes', len(js.encode()), 'details', list(details))
print('fin', fin['usd']['expense'], fin['usd']['income'], 'entries', len(ls), 'exch', len(exch), 'voided', fin['voided_excluded'])
