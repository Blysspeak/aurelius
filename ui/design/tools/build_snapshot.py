"""Build ui/design/snapshot.js from real au exports in /tmp/aud (taken 2026-10-15)."""
import json, random, collections, sys, datetime

random.seed(7)
OUT = sys.argv[1]
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
    pick |= set(random.sample(m, min(k, len(m))))
tl0 = json.load(open('/tmp/aud/tasks.json'))
pick |= {t['id'] for t in tl0}
pick = [k for k in pick if k in nodes]
idx = {k: i for i, k in enumerate(pick)}

def short(s, n=90):
    s = (s or '').replace('\n', ' ')
    return s if len(s) <= n else s[:n - 1] + '…'

N = []
for k in pick:
    x = nodes[k]
    dd = x.get('data') or {}
    age = (datetime.datetime(2026, 10, 15, tzinfo=datetime.timezone.utc) - datetime.datetime.fromisoformat(x['created_at'][:19] + '+00:00')).days
    st = dd.get('status') if ntype(x) == 'task' else None
    N.append([TI.get(ntype(x), TI['doc']), CONF.index(dd.get('confidence')) if dd.get('confidence') in CONF else 0,
              short(x['label'], 80), max(age, 0), st or 0])
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

graph = {
    'types': TYPES, 'conf': CONF, 'rels': RELS, 'nodes': N, 'edges': E, 'focus': idx[FOCUS],
    'expand': idx[EXP], 'details': details,
    'projects': [[idx[p], nodes[p]['label'], len(members[p])] for p in top],
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
