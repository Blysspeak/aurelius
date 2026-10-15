"""Build the aurelius icon system: ui/design/icons/*.svg, icons.svg (symbol sprite), icons.js.

Grid 24, live area 4..20, stroke 1.5, round caps and joins, open line icons, currentColor.
Small solid marks (dots) are allowed where the metaphor needs them.
Run: python3 -I ui/design/tools/build_icons.py
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.dirname(HERE)

P = lambda d: f'<path d="{d}"/>'
C = lambda cx, cy, r: f'<circle cx="{cx}" cy="{cy}" r="{r}"/>'
D = lambda cx, cy, r=1.1: f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="currentColor" stroke="none"/>'
R = lambda x, y, w, h, rx=1.5: f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}"/>'
PAGE = P('M7 4h7l4 4v12H7z') + P('M14 4v4h4')

ICONS = {
    # ---- node types (silhouette first, meaning inside)
    'decision': P('M12 4l8 8-8 8-8-8z') + P('M12 16v-3.5M12 12.5 9.8 10.3M12 12.5l2.2-2.2'),
    'problem': P('M12 4.5 20 19H4z') + P('M11 10.2l1.8 1.8-1.6 1.6 1.8 1.9'),
    'solution': P('M6 5h12l-2.6 14H8.6z') + P('M9.4 11.4l1.9 1.9 3.4-3.6'),
    'concept': C(11, 13, 5.5) + D(17.4, 6.6, 1.5),
    'session': C(12, 12, 7) + P('M12 8v4l2.6 1.6'),
    'task': R(5, 5, 14, 14, 2) + P('M9 12.2l2 2 4-4.2'),
    'work_log': P('M6 8h12M6 12h12M6 16h8'),
    'user_fact': C(12, 9, 3.2) + P('M6 19.5c0-3.4 2.7-5.7 6-5.7s6 2.3 6 5.7'),
    'person': C(9.5, 9.5, 2.6) + C(15.6, 10, 2.2) + P('M4.5 18.5c.3-3.1 2.3-4.7 5-4.7s4.7 1.6 5 4.7') + P('M14.6 13.9c2.6 0 4.6 1.4 4.9 4.1'),
    'doc': PAGE + P('M9.5 12.5h6M9.5 15.5h6'),
    'skill': R(5, 6, 14, 12, 2) + P('M12 9l.9 2.1 2.1.9-2.1.9L12 15l-.9-2.1L9 12l2.1-.9z'),
    'file': PAGE + P('M10.6 11.6 9 13.6l1.6 2M14.4 11.6l1.6 2-1.6 2'),
    'dependency': P('M10 14l-2 2a2.83 2.83 0 0 1-4-4l2-2a2.83 2.83 0 0 1 4 0') + P('M14 10l2-2a2.83 2.83 0 0 1 4 4l-2 2a2.83 2.83 0 0 1-4 0') + P('M10 14l4-4'),
    'server': R(5, 4.5, 14, 4, 1) + R(5, 10, 14, 4, 1) + R(5, 15.5, 14, 4, 1) + D(8, 6.5, .8) + D(8, 12, .8) + D(8, 17.5, .8),
    'project': P('M12 4l6.9 4v8L12 20l-6.9-4V8z') + D(12, 9.6, 1.2) + D(9.7, 13.8, 1.2) + D(14.3, 13.8, 1.2),
    'reminder': P('M8 16v-4.5a4 4 0 0 1 8 0V16l1.5 1.5h-11z') + P('M10.5 19.8h3M12 5.5V7.5'),
    'module': P('M12 4l7 4v8l-7 4-7-4V8z') + P('M5 8l7 4 7-4M12 12v8'),
    'digest': P('M5 6h14l-5.5 6.5V18l-3 1.5v-7z'),
    'config': P('M6 8h12M6 16h12') + C(10, 8, 1.9) + C(15, 16, 1.9),
    # ---- task statuses
    'st-backlog': C(12, 12, 6),
    'st-active': C(12, 12, 7.5).replace('/>', ' opacity=".45"/>') + D(12, 12, 3.6),
    'st-blocked': C(12, 12, 7) + P('M7.5 12h9'),
    'st-done': C(12, 12, 7) + P('M8.8 12.2l2.2 2.2 4.2-4.4'),
    'st-cancelled': C(12, 12, 7) + P('M7.6 16.4l8.8-8.8'),
    # ---- confidence marks
    'cf-measured': D(12, 12, 5),
    'cf-inferred': '<circle cx="12" cy="12" r="6" stroke-dasharray="2.6 2.1"/>',
    'cf-reported': '<circle cx="12" cy="12" r="6" stroke-dasharray=".01 2.9"/>',
    'cf-stale': '<circle cx="12" cy="12" r="6" opacity=".5"/>',
    # ---- finance kinds
    'fin-expense': P('M12 5v10M8 11l4 4 4-4M6 19h12'),
    'fin-income': P('M12 15V5M8 9l4-4 4 4M6 19h12'),
    'fin-exchange': P('M6 9h11l-3-3M18 15H7l3 3'),
    # ---- finance categories
    'cat-apartment': P('M5 11l7-6 7 6v8H5z') + P('M10 19v-5h4v5'),
    'cat-groceries': P('M5 10h14l-1.5 8h-11z') + P('M8.5 10 11 5.5M15.5 10 13 5.5'),
    'cat-transport': P('M5 15v-3l2-4h10l2 4v3z') + P('M5 12h14') + C(8, 16.5, 1.5) + C(16, 16.5, 1.5),
    'cat-car-loan': P('M4 15v-2.5l2-3h8l2 3V15z') + C(7, 16.3, 1.3) + C(13, 16.3, 1.3) + C(18.5, 6.5, 2.5),
    'cat-claude': C(12, 12, 7) + P('M12 8v8M8.5 10l7 4M15.5 10l-7 4'),
    'cat-transfers': C(9, 8.5, 2.5) + P('M4.5 18c.3-3 2.2-4.5 4.5-4.5') + P('M13 14h6M16.5 11.5 19 14l-2.5 2.5'),
    'cat-erip': P('M7 4h10v16l-2-1.5-2 1.5-2-1.5-2 1.5-2-1.5z') + P('M9.5 9h5M9.5 12.5h5'),
    'cat-cafe': P('M6 9h10v5a4 4 0 0 1-4 4h-2a4 4 0 0 1-4-4z') + P('M16 10.5h1.5a2 2 0 0 1 0 4H16') + P('M9 4.5v2M12 4.5v2'),
    'cat-marketplaces': P('M6 8h12l-1 12H7z') + P('M9.5 8V7a2.5 2.5 0 0 1 5 0v1'),
    'cat-cash': R(4, 7, 16, 10) + C(12, 12, 2.2),
    'cat-digital': R(5, 5, 14, 10) + P('M9 19h6M12 15v4') + P('M12 7.5v4.5M10 10l2 2 2-2'),
    'cat-services': P('M17 9a6 6 0 0 0-10.5.5M7 15a6 6 0 0 0 10.5-.5') + P('M17 5v4h-4M7 19v-4h4'),
    'cat-home': P('M8 4h8l2 6H6z') + P('M12 10v8M8.5 20h7'),
    'cat-clothes': P('M9 4.5 5 7.5l1.5 3.5L8 10v9.5h8V10l1.5 1 1.5-3.5-4-3c-.7 1.5-1.7 2.3-3 2.3S9.7 6 9 4.5z'),
    'cat-entertainment': P('M5 8h14v2.5a1.5 1.5 0 0 0 0 3V16H5v-2.5a1.5 1.5 0 0 0 0-3z') + '<path d="M12.5 8.5v7" stroke-dasharray="1.2 1.8"/>',
    'cat-hosting': P('M12 4l6.5 2.5v5c0 4-2.7 7.1-6.5 8.5-3.8-1.4-6.5-4.5-6.5-8.5v-5z') + C(12, 11, 1.6) + P('M12 12.6V15'),
    'cat-pets': C(12, 15.5, 2.8) + C(7.5, 10.8, 1.3) + C(10, 7.2, 1.3) + C(14, 7.2, 1.3) + C(16.5, 10.8, 1.3),
    'cat-telecom': D(12, 17.5, 1.2) + P('M8.5 14a5 5 0 0 1 7 0M5.5 11a9 9 0 0 1 13 0'),
    'cat-profi': P('M14.5 5.5a4 4 0 0 0-4.6 5.6L5 16l3 3 4.9-4.9a4 4 0 0 0 5.6-4.6L16 12l-4-4z'),
    'cat-other': D(7, 12) + D(12, 12) + D(17, 12),
    'cat-health': P('M10 5h4v5h5v4h-5v5h-4v-5H5v-4h5z'),
    'cat-credit': R(4, 6.5, 16, 11) + P('M4 10h16M7 14.5h3'),
    'cat-ip': R(4.5, 8, 15, 10) + P('M9.5 8V6.5a1 1 0 0 1 1-1h3a1 1 0 0 1 1 1V8M4.5 12.5h15'),
    'cat-taxes': PAGE + P('M10 16l4-5') + C(10.2, 11.4, .8) + C(13.8, 15.6, .8),
    'cat-bank-fees': P('M4.5 9 12 5l7.5 4z') + P('M6.5 9v8M10 9v8M14 9v8M17.5 9v8M4.5 19h15'),
    'cat-documents': P('M9 4.5h6V9l1.5 4h-9L9 9z') + P('M6 16h12v3H6z'),
    'cat-tests': P('M10 4.5h4M10.5 4.5V10L6 18a1.3 1.3 0 0 0 1.1 2h9.8a1.3 1.3 0 0 0 1.1-2l-4.5-8V4.5') + P('M8 15h8'),
    'cat-travel': P('M4 12l16-7-5 14-3.5-5.5z') + P('M11.5 13.5 20 5'),
    'cat-cash-deposit': R(4, 11, 16, 8) + P('M12 4v5M9.5 6.5 12 9l2.5-2.5'),
    'cat-refunds': P('M5 10h10a4.5 4.5 0 0 1 0 9h-5') + P('M8.5 6.5 5 10l3.5 3.5'),
    'cat-incoming': C(15, 8.5, 2.5) + P('M10.5 18c.3-3 2.2-4.5 4.5-4.5s4.2 1.5 4.5 4.5') + P('M3.5 12H9M6.5 9.5 9 12l-2.5 2.5'),
    'cat-cashback': C(11, 13, 5.5) + P('M11 10.5v5') + P('M15 4a8 8 0 0 1 4.5 5.5M19.5 6v3.5H16'),
    'cat-offcard': R(4, 7, 12, 9) + C(10, 11.5, 1.8) + P('M15 18h5M18 16l2 2-2 2'),
    'cat-contract': P('M5 19h14') + P('M7 15.5 15.5 7 18 9.5 9.5 18H7z'),
    'cat-topup': P('M4.5 9 11 5.5 17.5 9z') + P('M6.5 9v6M11 9v6M15.5 9v6M4.5 17h13') + P('M18.5 15.5v5M16 18h5'),
}

# au category name -> icon (both directions of money)
CATEGORY = {
    'квартира': 'cat-apartment', 'продукты': 'cat-groceries', 'транспорт и авто': 'cat-transport',
    'кредит за машину': 'cat-car-loan', 'подписка claude': 'cat-claude', 'переводы людям': 'cat-transfers',
    'платежи ерип': 'cat-erip', 'кафе и еда вне дома': 'cat-cafe', 'маркетплейсы': 'cat-marketplaces',
    'наличные': 'cat-cash', 'цифровые покупки': 'cat-digital', 'подписки и сервисы': 'cat-services',
    'покупки для дома': 'cat-home', 'одежда и уход': 'cat-clothes', 'развлечения': 'cat-entertainment',
    'хостинг и vpn': 'cat-hosting', 'питомцы': 'cat-pets', 'связь и интернет': 'cat-telecom',
    'profi.ru': 'cat-profi', 'прочее': 'cat-other', 'здоровье и аптека': 'cat-health', 'кредит': 'cat-credit',
    'услуги ип': 'cat-ip', 'налоги': 'cat-taxes', 'комиссии банков': 'cat-bank-fees',
    'документы и госпошлины': 'cat-documents', 'тесты xhub': 'cat-tests', 'путешествия': 'cat-travel',
    'внесение наличных': 'cat-cash-deposit', 'возвраты': 'cat-refunds', 'входящие переводы': 'cat-incoming',
    'кэшбэк и бонусы': 'cat-cashback', 'наличные мимо карт': 'cat-offcard', 'подряд': 'cat-contract',
    'пополнение из других банков': 'cat-topup',
}

ATTR = 'fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"'


def main():
    os.makedirs(os.path.join(OUT, 'icons'), exist_ok=True)
    for name, body in ICONS.items():
        with open(os.path.join(OUT, 'icons', f'{name}.svg'), 'w') as f:
            f.write(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" {ATTR}>{body}</svg>\n')
    sprite = ''.join(f'<symbol id="i-{n}" viewBox="0 0 24 24" {ATTR}>{b}</symbol>' for n, b in ICONS.items())
    with open(os.path.join(OUT, 'icons.svg'), 'w') as f:
        f.write(f'<svg xmlns="http://www.w3.org/2000/svg" style="display:none">{sprite}</svg>\n')
    with open(os.path.join(OUT, 'icons.js'), 'w') as f:
        f.write('// Generated by tools/build_icons.py: the icon system as data, so file:// pages can inline it.\n')
        f.write('window.AU_ICONS = ' + json.dumps({'attr': ATTR, 'icons': ICONS, 'category': CATEGORY}, ensure_ascii=False) + ';\n')
    print(len(ICONS), 'icons')


if __name__ == '__main__':
    sys.exit(main())
