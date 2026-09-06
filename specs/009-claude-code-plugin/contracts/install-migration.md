# Contract: `install.sh` — установка плагина и миграция

## Что остаётся

1. Сборка `cargo build --release` и установка `au` и `aurelius` в `~/.local/bin` (замена через
   временный файл и `mv`: прямой `cp` поверх бинарника, который держат запущенные MCP-серверы,
   падает с ETXTBSY).
2. Опциональная сборка UI, `au init`, ключ Brave, git-хук `post-commit` для текущего репозитория,
   `au reindex` — без изменений.

## Что уходит

- Раздел 6 «Install Claude Code hooks» — копирование `contrib/claude-code/*.sh` в `~/.claude/hooks`.
- Раздел 7 «Auto-configure Claude Code settings» — python-блок, добавляющий `mcpServers` и хуки в
  `~/.claude/settings.json`.

Проверка (SC-004): `grep -n "settings.json\|\.claude\.json" install.sh` находит только строки
миграционного блока ниже.

## Что появляется: установка плагина

```
claude plugin marketplace add "$SCRIPT_DIR"            # маркетплейс blysspeak из локального клона
claude plugin install aurelius@blysspeak -s user -y      # пользовательская область
claude plugin list                                        # печатается для человека
```

- `claude` не найден → предупреждение с двумя командами выше для ручного запуска; код возврата
  `install.sh` не меняется (бинарники уже стоят).
- Маркетплейс уже добавлен → `claude plugin marketplace update blysspeak` вместо `add`.
- Плагин уже установлен → `claude plugin update aurelius` (или повторный `install`, если `update`
  в этой версии не принимает имя — проверить `claude plugin --help` при реализации).

## Что появляется: миграция старых записей

python3-блок `migrate_legacy()`:

| Файл | Что снимается | Признак |
|---|---|---|
| `~/.claude/settings.json` | элементы `hooks.<event>[].hooks[]` | `command` содержит `aurelius-(reindex\|track-edit\|skills\|backup\|capture)\.sh`, или начинается с `au ` и содержит `--hook` |
| `~/.claude/settings.json` | `mcpServers.aurelius` | ключ — снимается всегда |
| `~/.claude.json` | `mcpServers.aurelius` | ключ — снимается, только если запись не каноническая (`command` не `au mcp`) |

Правила:

- Перед первой правкой файла — копия `<файл>.bak-<UTC %Y%m%dT%H%M%SZ>` рядом.
- Группа матчера, оставшаяся без хуков, удаляется; событие без групп удаляется; пустой
  `mcpServers` удаляется.
- Каждая снятая запись — строка в stdout:
  `снято: settings.json hooks.Stop "" → bash ~/.claude/hooks/aurelius-reindex.sh — переехало в плагин aurelius`
  `снято: ~/.claude.json mcpServers.aurelius → /home/blyss/.local/share/mcp/aurelius — сервер теперь регистрирует плагин`
- Нечего снимать → одна строка `миграция не требуется: старых записей aurelius нет`.
- Не трогаются: `aurelius-save-reminder.mjs`, любые команды без признака, хуки ulika.
- После снятия печатается напоминание: файлы `~/.claude/hooks/aurelius-*.sh` и симлинк
  `~/.local/share/mcp/aurelius` больше не используются — удалять или нет, решает человек.
- `python3` не найден → блок пропускается с предупреждением и ссылкой на quickstart (ручное
  снятие).

Идемпотентность: второй запуск на тех же файлах не создаёт новых `.bak-*` и печатает «миграция не
требуется».

## Регистрация MCP-сервера

Функция `register_mcp_server` выполняется после `migrate_legacy`. Если `claude mcp get aurelius`
уже показывает команду `au` с аргументом `mcp` — ничего не делает и говорит об этом. Иначе снимает
старую запись (`claude mcp remove aurelius -s user`, ошибка игнорируется) и выполняет
`claude mcp add -s user aurelius au mcp`; запись ложится в `~/.claude.json` как
`mcpServers.aurelius`. Отказ команды — предупреждение с ручной командой, установка продолжается.
Режим `--migrate-only` сервер не регистрирует. Миграция не удаляет такую каноническую запись:
удаляются только записи с другой командой (старая обёртка) и любая `mcpServers.aurelius` в
`settings.json`.

## Проверка (принцип V)

- `bash -n install.sh`.
- Прогон `migrate_legacy()` на копиях живых файлов владельца в `$TMP` (переменные `CLAUDE_HOME`
  или аналог, чтобы блок читал не `~/.claude`): в выводе семь строк «снято» для хуков и одна для
  сервера; в результирующем `settings.json` остаются `aurelius-save-reminder.mjs` и хуки ulika;
  `.bak-*` создан; повторный прогон — «миграция не требуется».
- Живая миграция на Linux-машине владельца — отдельная волна с рестартом Claude Code.

## README

Раздел «Установка» переписывается: (1) чистая машина — сборка, PATH, две команды плагина,
рестарт; (2) существующая машина — `install.sh` и что он снимет; (3) Windows — сборка,
`%USERPROFILE%\.local\bin` в PATH, те же две команды плагина, ручное снятие старых записей по
таблице выше. Раздел «Claude Code Integration» ссылается на `plugin/hooks.json` как на источник
истины и помечает `contrib/claude-code/*.sh` устаревшими.
