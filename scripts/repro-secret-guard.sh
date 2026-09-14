#!/usr/bin/env bash
# Репро рубежа секретов на запись (`add_node_full`, subject
# `aurelius:write:secret-guard`): полезная нагрузка заказа от 14.09.2026
# прогоняется через НАСТОЯЩИЙ путь записи (`au note`), а не через юнит-тест,
# потому что код возврата и то, что реально легло в узлы, существуют только у
# процесса и его базы.
#
# Что проверяется:
#   1. точная полезная нагрузка заказа (note + claim + subject + evidence)
#      проходит с кодом 0 и ложится ровно одним узлом без маркера обхода;
#   2. camelCase-имя кода, на котором рубеж отказал 14.09.2026 (смещение 55
#      байт в `evidence` заметки про dsh-russian-lang), больше не отказывает;
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
    eyJub3RlIjogIkFTS0VEOiDQu9C+0LrQsNC70LjQt9C+0LLQsNGC0Ywg0LjQvdGC0LXRgNGE0LXQ
    udGBIERTSCDQvdCwINGA0YPRgdGB0LrQuNC5LiBXSFk6INCy0LvQsNC00LXQu9C10YYg0LLQuNC0
    0LXQuyDQutC40YLQsNC50YHQutC40LUg0L/QvtC00L/QuNGB0Lgg0LIg0LTQvtC60LUg0LrQvtC8
    0L/QvtC30LXRgNCwINC4INCyINC90LDRgdGC0YDQvtC50LrQsNGFLiBGT1VORDog0LrQuNGC0LDQ
    udGB0LrQuNC5INC70YzRjtGCINGB0YLQvtGA0L7QvdC90LjQtSBjbGllbnQt0L/Qu9Cw0LPQuNC9
    0Ysg0L/RgNC+0YTQuNC70Y8gd2ViLCDRj9C00YDQviDRg9C20LUg0LDQvdCz0LvQuNC50YHQutC+
    0LU7INGI0YLQsNGC0L3Ri9C5IGxvY2FsZS5yZWdpc3RlciDQv9C+0LfQstC+0LvRj9C10YIg0LTQ
    vtC/0L7Qu9C90LjRgtGMINGH0YPQttC+0LkgbmFtZXNwYWNlINGB0LvQvtCy0LDRgNGR0LwgcnUs
    INCwINC/0LvQsNCz0LjQvdC5INCx0LXQtyDQu9C+0LrQsNC70LjQt9Cw0YbQuNC4IChhcGktYmFs
    YW5jZSkg0L/QtdGA0LXQstC+0LTQuNGC0YHRjyDRgtC+0LvRjNC60L4g0L/RgNCw0LLQutC+0Lkg
    0LrQvtC0IOKAlCDRhNC+0YDQuiDRgSDRgNGD0YHRgdC60LjQvCDRgdC70L7QstCw0YDRkdC8LiBO
    RVhUOiDQtNCy0LAg0YDQtdGI0LXQvdC40Y8g0LLQu9Cw0LTQtdC70YzRhtCwIOKAlCDQv9GD0LHQ
    u9C40LrQvtCy0LDRgtGMINC70Lgg0YTQvtGA0Log0Lgg0L/RgNC10LTQu9Cw0LPQsNGC0Ywg0LvQ
    uCDQu9C+0LrQsNC70LjQt9Cw0YbQuNGOINCyINCw0L/RgdGC0YDQuNC8IDAyTXVsbGVyMjUvZHNo
    LWFwaS1iYWxhbmNlLlxuXG7Qk9GA0LDQsdC70Lg6IHBucG0gMTEg0LTQtdGA0LbQuNGCINGB0YPR
    gtC+0YfQvdGL0Lkg0LrQsNGA0LDQvdGC0LjQvSDQvdCwINGB0LLQtdC20LjQtSDQstC10YDRgdC4
    0LggKNC+0LHRhdC+0LQg0YfQtdGA0LXQtyDQvNC40L3QuNC80LDQu9GM0L3Ri9C5INCy0L7Qt9GA
    0LDRgdGCINGA0LXQu9C40LfQsCDQvdC1INC00LXQu9Cw0LvQuCwgMC4yLjE2INC/0L7QtNGC0Y/Q
    vdC10YLRgdGPINGB0LDQvCkuINCW0LjQstC+0Lkgd2F0Y2hlciDQv9GA0L7RhNC40LvRjyDRgdC7
    0LXQtNC40YIg0YLQvtC70YzQutC+INC30LAg0YTQsNC50LvQvtC8INC/0LDRgtGH0LAg0L/RgNC+
    0YTQuNC70Y8g0Lgg0L/QsNGC0YfQtdC8INCyINC00L7QvNCw0YjQvdC10Lwg0LrQsNGC0LDQu9C+
    0LPQtSBEU0gg4oCUINC40LfQvNC10L3QtdC90LjRjyBwYWNrYWdlLmpzb24gKNGB0L/QuNGB0L7Q
    uiDQsdCw0L3QtNC70L7Qsiwg0LjRgdGC0L7Rh9C90LjQuiDQt9Cw0LLQuNGB0LjQvNC+0YHRgtC4
    KSDQstC40LTQvdGLINGC0L7Qu9GM0LrQviDQvdCwINGB0YLQsNGA0YLQtSwg0L/QvtGN0YLQvtC8
    0YMg0YHQvNC10L3QsCDQuNGB0YLQvtGH0L3QuNC60LAgYXBpLWJhbGFuY2Ug0Lgg0L3QvtCy0YvQ
    uSDQv9Cw0LrQtdGCINC/0L7RgtGA0LXQsdC+0LLQsNC70Lgg0YDQtdGB0YLQsNGA0YLQsCBzeXN0
    ZW1kLdGO0L3QuNGC0LAgZHNoLXdlYi5zZXJ2aWNlLiDQotC40L/QvtCz0YDQsNGE0LjQutGDINC/
    0LDQutC10YLQsCDQstGL0LrQu9GO0YfQuNC70Lg6INC/0L7Qu9C1IGxpdmVJbnB1dCDQv9C+INGD
    0LzQvtC70YfQsNC90LjRjiDQstC60LvRjtGH0LXQvdC+INC4INC80LDRgdGC0LXRgC3QutC70Y7R
    h9C+0Lwg0L3QtSDQs9C10LnRgtC40YLRgdGPLCDQvtC90LAg0L/RgNCw0LLQuNC70LAg0LHRiyDR
    gtC10LrRgdGCINC/0YDRj9C80L4g0LIg0L/QvtC70LUg0LLQstC+0LTQsC4g0KHQtdGC0Ywg0LIg
    0L/QsNC60LXRgtC1INC20LjQstGR0YIg0YLQvtC70YzQutC+INC/0L7QtCDQutC90L7Qv9C60L7Q
    uSDQv9C10YDQtdCy0L7QtNCwINC+0YLQstC10YLQsCDQsNGB0YHQuNGB0YLQtdC90YLQsC4iLCAi
    ZGF0YSI6IHsiZm9ya19jb21taXRzIjogWyIyMjJlNGY2IiwgIjc0NTA2ODIiXSwgImZvcmtfbG9j
    YXRpb24iOiAid29ya1NwYWNlL3Byb2plY3QvZHNoLWFwaS1iYWxhbmNlIiwgImluc3RhbGxlZCI6
    IHsiYXBpLWJhbGFuY2UiOiAibGluayAwLjMuMCIsICJsYW5nX3BhY2siOiAiMC4yLjE0In0sICJy
    ZWplY3RlZCI6IFsi0L/QsNGC0Ycgbm9kZV9tb2R1bGVzIiwgItC/0LDRgtGHINC40YHRhdC+0LTQ
    vdC40LrQvtCyINGH0LXQutCw0YPRgtCwIiwgItGB0LLQvtC5INC/0LXRgNC10YXQstCw0YLRh9C4
    0Log0YLQtdC60YHRgtCwINCyIERPTSIsICJpbWRlbmlpbC9kc2gtbG9jYWxlLXJ1Il19LCAiY2xh
    aW0iOiAi0KDRg9GB0YHQutC40Lkg0LTQu9GPIENTSCDQtNC10LvQsNC10YLRgdGPINCx0LXQtyDQ
    v9GA0LDQstC60Lgg0Y/QtNC90YA6INGP0LfRi9C60L7QstC+0Lkg0L/QsNC60LXRgiDQtNC+0L/Q
    vtC70L3Rj9C10YIg0YfRg9C20LjQtSBuYW1lc3BhY2Ug0YHQu9C+0LLQsNGA0ZHQvCBydSwg0LAg
    0L/Qu9Cw0LPQuNC9INCx0LXQtyDQu9C+0LrQsNC70LjQt9Cw0YbQuNC4INGE0L7RgNC60LDQtdGC
    0YHRjyDQuCDQv9C+0LvRg9GH0LDQtdGCIGxvY2FsZS5yZWdpc3RlciIsICJzdWJqZWN0IjogImRl
    ZXBzZWVrLWhhcm5lc3M6d2ViLWd1aTpsb2NhbGl6YXRpb24tcnUiLCAiZXZpZGVuY2UiOiAiZ2l0
    IC1DIHdvcmtTcGFjZS9wcm9qZWN0L2RzaC1hcGktYmFsYW5jZSBsb2cgLS1vbmVsaW5lOyBucG0t
    dmVyc2lvbiDQvtCx0L7QuNGFINGD0YHRgtCw0L3QvtCy0LvQtdC90L3Ri9GFINC/0LDQutC10YLQ
    vtCyOyBncmVwIC1jINC/0L4gQ0pLINCyINGD0YHRgtCw0L3QvtCy0LvQtdC90L3QvtC8INC60LvQ
    uNC10L3RgtGB0LrQvtC8INCx0LDQvdC00LvQtSBhcGktYmFsYW5jZSDQtNCw0ZHRgiAwIn0=
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

# 1. Точная полезная нагрузка заказа целиком.
H="$(fresh_home payload)"
check "payload accepted end-to-end" "$H" 0 1 \
    note "$NOTE" --claim "$CLAIM" --subject "$SUBJECT" --evidence "$EVIDENCE"

# 2. Живой виновник: camelCase-имя в evidence.
CAMEL_EVIDENCE='python3 scan of lib/client.js: the single call site of translateTurnContent sits inside the onClick of slot entry dsh-russian-lang-translate-action; the two fetch calls are the only network calls in the bundle'
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
