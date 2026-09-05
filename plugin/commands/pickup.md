---
description: Подъём состояния после /clear — знания, задачи и git по проекту
argument-hint: [project]
allowed-tools: Bash
---

# Подъём

Контекст только что очищен `/clear`. Это не новая задача — восстанови картину по данным ниже и продолжай с того места, где остановились. Не переспрашивай то, что уже написано здесь.

**Проект:** !`P="$ARGUMENTS"; [ -z "$P" ] && P=$(basename "$PWD"); echo "$P"`

## На чём остановились — хвост прошлой сессии
Это главное в подъёме. Ответ начинай с этого, а не с беклога.
!`P="$ARGUMENTS"; [ -z "$P" ] && P=$(basename "$PWD"); R=$(timeout 15 au export 2>/dev/null | jq -r --arg p "$P" '[.nodes[] | select(.node_type=="session") | select(.data.project==$p) | select(((.data.next_steps//[])|length)>0)] | sort_by(.created_at) | reverse | .[0] | if . == null then empty else ("последняя сессия с хвостом: " + (.created_at[0:16] | sub("T";" ")) + " UTC"), ((.data.next_steps//[])[] | "- → " + .), (if ((.data.key_files//[])|length)>0 then "- файлы: " + ((.data.key_files)|join(", ")) else empty end) end' 2>/dev/null); [ -z "$R" ] && R="хвоста нет — последняя сессия по «$P» писалась без --next (au session -n «…»)"; echo "$R"`

## Знания и незакрытая работа
!`P="$ARGUMENTS"; [ -z "$P" ] && P=$(basename "$PWD"); R=$(timeout 10 au snapshot -p "$P" --json 2>/dev/null | jq -r '[.facts[] | select(.kind!="userfact") | select(.kind!="active_task" and .kind!="task") | select(.kind!="digest" or (.text|length)>60) | select(.text | test("тир .*прогон wf_|кодовых правок за ход|чекпоинт [0-9]+k|ходов [0-9]+\\. тиры|компакций [0-9]+|улик зелёных [0-9]+, красных [0-9]+|Перенос рабочего окружения") | not)] | sort_by(.at) | reverse | .[0:16][] | "- [" + .kind + "] " + (.text[0:220] | gsub("\n";" "))' 2>/dev/null); [ -z "$R" ] && R="нет сохранённых знаний по «$P» в au — либо проект новый, либо au недоступен"; echo "$R"`

## Открытые задачи
!`P="$ARGUMENTS"; [ -z "$P" ] && P=$(basename "$PWD"); R=$(timeout 8 au task list -p "$P" -s active,backlog 2>/dev/null | grep -v '^[[:space:]]*by:' | head -30); [ -z "$R" ] && R="au task list недоступен для «$P»"; echo "$R"`

## Git
!`R=$(timeout 5 bash -c 'git rev-parse --is-inside-work-tree >/dev/null 2>&1 && { echo "ветка: $(git branch --show-current 2>/dev/null), последний коммит: $(git log -1 --format="%h %s" 2>/dev/null)"; N=$(git status --short | wc -l); echo "незакоммичено файлов: $N"; git status --short | head -12; }' 2>/dev/null); [ -z "$R" ] && R="не git-репозиторий или git недоступен в текущем каталоге"; echo "$R"`

Дальше — по делу, без пересказа того, что уже написано выше.
