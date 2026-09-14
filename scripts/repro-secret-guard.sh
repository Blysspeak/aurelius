#!/usr/bin/env bash
# Репро рубежа секретов на запись (`add_node_full`, subject
# `aurelius:write:secret-guard`): полезная нагрузка заказа от 14.09.2026
# прогоняется через НАСТОЯЩИЙ путь записи (`au note`), а не через юнит-тест,
# потому что код возврата и то, что реально легло в узлы, существуют только у
# процесса и его базы.
#
# Что проверяется:
#   1. полезная нагрузка заказа той же формы (note + claim + subject + evidence)
#      проходит с кодом 0 и ложится ровно одним узлом без маркера обхода;
#   2. camelCase-имя кода той же формы, на которой рубеж отказал 14.09.2026
#      (смещение 55 байт в `evidence` рабочей заметки), больше не отказывает;
#   3. настоящий ключ с известным префиксом по-прежнему отказывает кодом 13;
#   4. случайная строка без camelCase-отрезков тоже по-прежнему отказывает.
#
# Базовая линия (как выглядел отказ ДО правки) снимается тем же скриптом на
# бинаре до правки:
#   AU_BIN=/home/blyss/.cargo/bin/au scripts/repro-secret-guard.sh --pre-fix
# В этом режиме проверка 2 ждёт код 13 и ноль узлов — ровно то, что измерено
# на 29bb64d.
set -euo pipefail

PRE_FIX=0
for arg in "$@"; do
    case "$arg" in
        --pre-fix) PRE_FIX=1 ;;
        -h|--help)
            sed -n '2,20p' "$0"
            exit 0
            ;;
        *)
            echo "unknown argument: $arg" >&2
            exit 2
            ;;
    esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AU_BIN="${AU_BIN:-$ROOT/target/debug/au}"
if [ ! -x "$AU_BIN" ]; then
    echo "au binary not found at $AU_BIN" >&2
    echo "build it first: cargo build -p au --bin au" >&2
    exit 2
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
PAYLOAD_JSON="$WORK/probe-payload.json"

# Полезная нагрузка заказа, зашитая в base64: скрипт самодостаточен и не
# зависит от того, лежит ли декодированный файл в $HOME. Внешний файл
# (PROBE_PAYLOAD или $HOME/probe-payload.json) имеет приоритет, чтобы можно
# было прогнать другую нагрузку тем же скриптом.
if [ -f "${PROBE_PAYLOAD:-$HOME/probe-payload.json}" ]; then
    cp "${PROBE_PAYLOAD:-$HOME/probe-payload.json}" "$PAYLOAD_JSON"
else
    base64 -d > "$PAYLOAD_JSON" <<'PAYLOAD_B64'
    eyJub3RlIjogIkFTS0VEOiDQstGL0Y/RgdC90LjRgtGMLCDQv9C+0YfQtdC80YMg0L3QvtGH0L3R
    i9C1INGB0L3QuNC80LrQuCDRgtC10L/Qu9C40YbRiyDQv9C+0L/QsNC00LDRjtGCINCyINGB0LXQ
    vNC10LnQvdGL0Lkg0LDQu9GM0LHQvtC8INCx0LXQtyDQv9C+0LTQv9C40YHQuC4gV0hZOiDQv9C+
    INC/0L7QtNC/0LjRgdC4INGBINC00LDRgtC+0Lkg0Lgg0LLQu9Cw0LbQvdC+0YHRgtGM0Y4g0YHQ
    stC10YDRj9GO0YIg0YDQvtGB0YIg0YDQsNGB0YHQsNC00YssINGB0LXRgNC40Y8g0LHQtdC3INC9
    0LXRkSDQsdC10YHQv9C+0LvQtdC30L3QsC4gRk9VTkQ6INC/0L7QtNC/0LjRgdGMINGB0YLQsNCy
    0LjRgiDRgdCx0L7RgNGJ0LjQuiDQutCw0LTRgNC+0LIsINCwINC80LjQvdC40LDRgtGO0YDRiyDR
    gNC10LbQtdGCINC+0YLQtNC10LvRjNC90YvQuSDQvNC+0LTRg9C70Ywg0L/RgNC10LLRjNGOLCDQ
    utC+0YLQvtGA0YvQuSDQviDQv9C+0LTQv9C40YHQuCDQvdC1INC30L3QsNC10YI7INGF0YPQuiB0
    aGVtZS5vdmVycmlkZSDQsiDRiNCw0LHQu9C+0L3QtSDQsNC70YzQsdC+0LzQsCDRgdGA0LDQsdCw
    0YLRi9Cy0LDQtdGCINGA0LDQvdGM0YjQtSDQvNC+0LTRg9C70Y8sINC4INC/0YDQuCDQutCw0LbQ
    tNC+0Lwg0L/QtdGA0LXRgdGH0ZHRgtC1INC80LjQvdC40LDRgtGO0YDQsCDRgdC+0LHQuNGA0LDQ
    tdGC0YHRjyDQuNC3INGH0LjRgdGC0L7Qs9C+INC60LDQtNGA0LAuINCc0L7QtNGD0LvRjCDQv9GA
    0LXQstGM0Y4g0LLQt9GP0YIg0LjQtyDRhNC+0YDQutCwIDA0SGFyYm9yMTcvd2ViLWltZy1nYWxs
    ZXJ5LiBORVhUOiDQvdC10LTQtdC70Y4g0LPQvtC90Y/RgtGMINC90L7Rh9C90YvQtSDRgdC10YDQ
    uNC4INC90LAg0YTQvtGA0LrQtSDRgSDQuNGB0L/RgNCw0LLQu9C10L3QvdGL0Lwg0L/QvtGA0Y/Q
    tNC60L7QvCDRhdGD0LrQvtCyINC4INGB0YfQuNGC0LDRgtGMINC60LDQtNGA0Ysg0LHQtdC3INC/
    0L7QtNC/0LjRgdC4LlxuXG7QodGC0LXQvdC0OiDQutCw0LzQtdGA0LAg0L3QsNC0INCz0YDRj9C0
    0LrQsNC80Lgg0YHQvdC40LzQsNC10YIg0YDQsNC3INCyINC00LXRgdGP0YLRjCDQvNC40L3Rg9GC
    LiDQodCx0L7RgNGJ0LjQuiDRgdCw0Lwg0LrQu9Cw0LTRkdGCINGE0LDQudC70Ysg0LIg0LrQsNGC
    0LDQu9C+0LMg0LDQu9GM0LHQvtC80LAsINCwIHBpeC13ZWIuc2VydmljZSDRgNCw0Lcg0LIg0YfQ
    sNGBINC/0LXRgNC10YHQvtCx0LjRgNCw0LXRgiDRgdGC0YDQsNC90LjRhtGLLiDQnNC+0LTRg9C7
    0Ywg0L/RgNC10LLRjNGOINCy0LXRgNGB0LjQuCAxLjQuMiwg0YLQtdC80LAg0LfQsNC60YDQtdC/
    0LvRj9C10YIgMS40LjAg0YfQtdGA0LXQtyBsaW5rIDIuMS4wLCDRgtCw0Log0YfRgtC+INC+0LHQ
    vdC+0LLQu9GP0YLRjCDQv9GA0LjRhdC+0LTQuNGC0YHRjyDQsiDQtNCy0YPRhSDQvNC10YHRgtCw
    0YUuINCf0L7Qu9C1IGxhenlUaHVtYiDQsiBjb25maWcudG9tbCDQstC60LvRjtGH0LXQvdC+INC/
    0L4g0YPQvNC+0LvRh9Cw0L3QuNGOINC4INC+0YLQutC70LDQtNGL0LLQsNC10YIg0L/QtdGA0LXR
    gdGH0ZHRgiDQtNC+INC/0LXRgNCy0L7Qs9C+INC/0YDQvtGB0LzQvtGC0YDQsCwg0L/QvtGN0YLQ
    vtC80YMg0L/RgNC+0L/QsNC20LAg0L/QvtC00L/QuNGB0Lgg0LLQuNC00L3QsCDRgtC+0LvRjNC6
    0L4g0YPRgtGA0L7QvC4g0J7RgtCy0LXRgNCz0L3Rg9GC0L46INC/0YDQsNCy0LjRgtGMINGI0LDQ
    sdC70L7QvSDRj9C00YDQsCwg0YHRgtCw0LLQuNGC0Ywg0L/QvtC00L/QuNGB0Ywg0LLRgtC+0YDR
    i9C8INC/0YDQvtGF0L7QtNC+0Lwg0L/QviDQs9C+0YLQvtCy0YvQvCDQvNC40L3QuNCw0YLRjtGA
    0LDQvCwg0LLRi9C60LvRjtGH0LjRgtGMINC/0YDQtdCy0YzRjiDRhtC10LvQuNC60L7QvC4iLCAi
    ZGF0YSI6IHsiZm9ya19jb21taXRzIjogWyI1ZDJlOGExIiwgImIzOTA3YzQiXSwgImZvcmtfbG9j
    YXRpb24iOiAid29ya1NwYWNlL3Byb2plY3Qvd2ViLWltZy1nYWxsZXJ5IiwgImluc3RhbGxlZCI6
    IHsiaW1nLWdhbGxlcnkiOiAibGluayAyLjEuMCIsICJ0aGVtZV9wYWNrIjogIjEuNC4wIn0sICJy
    ZWplY3RlZCI6IFsi0L/RgNCw0LLQutCwINGI0LDQsdC70L7QvdCwINGP0LTRgNCwIiwgItC/0L7Q
    tNC/0LjRgdGMINCy0YLQvtGA0YvQvCDQv9GA0L7RhdC+0LTQvtC8INC/0L4g0LPQvtGC0L7QstGL
    0Lwg0LzQuNC90LjQsNGC0Y7RgNCw0LwiLCAi0LLRi9C60LvRjtGH0LjRgtGMINC/0YDQtdCy0YzR
    jiDRhtC10LvQuNC60L7QvCIsICJleGFtcGxlLXVzZXIvd2ViLWltZy13YXRlcm1hcmsiXX0sICJj
    bGFpbSI6ICLQn9C+0LTQv9C40YHRjCDQvdCwINC80LjQvdC40LDRgtGO0YDQsNGFINCw0LvRjNCx
    0L7QvNCwINGC0LXRgNGP0LXRgtGB0Y8g0L3QtSDQsiDRj9C00YDQtTog0YXRg9C6IHRoZW1lLm92
    ZXJyaWRlINGB0YDQsNCx0LDRgtGL0LLQsNC10YIg0YDQsNC90YzRiNC1INC80L7QtNGD0LvRjyDQ
    v9GA0LXQstGM0Y4sINC4INC80LjQvdC40LDRgtGO0YDQsCDRgdC+0LHQuNGA0LDQtdGC0YHRjyDQ
    uNC3INGH0LjRgdGC0L7Qs9C+INC60LDQtNGA0LAg0LTQviDQvdCw0LvQvtC20LXQvdC40Y8g0L/Q
    vtC00L/QuNGB0LgiLCAic3ViamVjdCI6ICJiYWNreWFyZC1nYWxsZXJ5OndlYi1ndWk6dGh1bWJu
    YWlsaW5nLXJ1IiwgImV2aWRlbmNlIjogImdpdCAtQyB3b3JrU3BhY2UvcHJvamVjdC93ZWItaW1n
    LWdhbGxlcnkgbG9nIC0tb25lbGluZTsgeWFybi12ZXJzaW9uINC+0LHQvtC40YUg0L/QsNC60LXR
    gtC+0LIg0YLQtdC80Ys7IGdyZXAgLWMg0L/QviBFWElGINCyINGB0L7QsdGA0LDQvdC90L7QvCDQ
    sdCw0L3QtNC70LUg0L/RgNC10LLRjNGOIGltZy1nYWxsZXJ5INC00LDRkdGCIDAifQ==
PAYLOAD_B64
fi

# Поля вытаскиваются питоном и уходят в шелл уже проэкранированными: заметка
# содержит кириллицу и переводы строк.
eval "$(python3 - "$PAYLOAD_JSON" <<'PYEXTRACT'
import json, shlex, sys
d = json.load(open(sys.argv[1], encoding="utf-8"))
for key in ("note", "claim", "subject", "evidence"):
    print(f"{key.upper()}={shlex.quote(d[key])}")
PYEXTRACT
)"

PASS=0
FAIL=0

# Свежий изолированный дом на каждый случай: настоящая база владельца не
# должна быть задета, а число узлов — считаться по этому дому.
fresh_home() {
    local dir="$WORK/home-$1"
    rm -rf "$dir"
    mkdir -p "$dir"
    printf '%s' "$dir"
}

count_nodes() {
    python3 - "$1/aurelius.db" <<'PYCOUNT'
import os, sqlite3, sys
path = sys.argv[1]
if not os.path.exists(path):
    print(0)
    raise SystemExit
con = sqlite3.connect(path)
print(con.execute("SELECT COUNT(*) FROM nodes WHERE deleted_at IS NULL").fetchone()[0])
PYCOUNT
}

# check <tag> <home> <expected_exit> <expected_nodes> <args...>
check() {
    local tag="$1" home="$2" want_exit="$3" want_nodes="$4"
    shift 4
    local out err code nodes
    # Из дома запускается и сам `au`: без этого авто-индексатор увидит
    # Cargo.toml текущего каталога и набьёт изолированный дом своими узлами,
    # так что «сколько узлов легло» перестанет что-либо измерять. Тот же
    # приём, что у интеграционных тестов (`current_dir(&home)`).
    out="$(cd "$home" && AURELIUS_HOME="$home" "$AU_BIN" "$@" 2>"$WORK/stderr")" && code=0 || code=$?
    err="$(cat "$WORK/stderr")"
    nodes="$(count_nodes "$home")"
    if [ "$code" = "$want_exit" ] && [ "$nodes" = "$want_nodes" ]; then
        printf 'ok   %-46s exit=%s nodes=%s\n' "$tag" "$code" "$nodes"
        PASS=$((PASS + 1))
    else
        printf 'FAIL %-46s exit=%s (want %s) nodes=%s (want %s)\n' \
            "$tag" "$code" "$want_exit" "$nodes" "$want_nodes"
        printf '     stderr: %s\n' "$err"
        FAIL=$((FAIL + 1))
    fi
}

echo "au binary : $AU_BIN"
echo "payload   : $PAYLOAD_JSON ($(wc -c < "$PAYLOAD_JSON") bytes)"
if [ "$PRE_FIX" = 1 ]; then
    echo "mode      : pre-fix baseline (check 2 expects refusal)"
else
    echo "mode      : post-fix (check 2 expects acceptance)"
fi
echo

# 1. Полезная нагрузка заказа целиком (обезличенная, той же формы).
H="$(fresh_home payload)"
check "payload accepted end-to-end" "$H" 0 1 \
    note "$NOTE" --claim "$CLAIM" --subject "$SUBJECT" --evidence "$EVIDENCE"

# 2. Живой виновник: camelCase-имя в evidence.
CAMEL_EVIDENCE='python3 scan of src/widget.js: the single call site of calculateCartSummary sits inside the onClick of menu entry web-invoice-cart-calculate-action; the two render calls are the only layout calls in the widget'
H="$(fresh_home camel)"
if [ "$PRE_FIX" = 1 ]; then
    check "camelCase evidence refused (baseline)" "$H" 13 0 \
        note "status update" --evidence "$CAMEL_EVIDENCE"
else
    check "camelCase evidence accepted (fix)" "$H" 0 1 \
        note "status update" --evidence "$CAMEL_EVIDENCE"
fi

# 3. Настоящий ключ с известным префиксом — отказ в обе стороны правки.
H="$(fresh_home credential)"
REAL_KEY='sk-proj-abc123def456ghi789jkl012mno345'
check "known-prefix credential still refused" "$H" 13 0 \
    note "leaked token here" --evidence "curl -H 'Authorization: Bearer $REAL_KEY'"

# 4. Случайная строка без camelCase-отрезков — тоже отказ.
H="$(fresh_home random)"
RANDOM_WORD='aZbQmKxRvNpLwTsHdGfQr'
check "random mixed-case string still refused" "$H" 13 0 \
    note "leaked: $RANDOM_WORD rotate it"

echo
echo "passed: $PASS  failed: $FAIL"
[ "$FAIL" = 0 ]
