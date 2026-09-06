# Contract: раскладка плагина

Файлы, которые появляются в репозитории, и их содержимое. Всё — данные, кода нет.

## `plugin/.claude-plugin/plugin.json`

```json
{
  "name": "aurelius",
  "version": "3.4.0",
  "description": "Long-term memory for Claude Code: session hooks, skill cards and the /pickup command. The MCP server is not bundled here: install.sh registers it user-scope (claude mcp add -s user aurelius au mcp) so its tools keep the mcp__aurelius__ prefix. Requires the au binary in PATH.",
  "author": { "name": "Vladislav Rahmanov" },
  "homepage": "https://github.com/Blysspeak/aurelius",
  "repository": "https://github.com/Blysspeak/aurelius",
  "license": "MIT",
  "keywords": ["memory", "knowledge-graph", "mcp", "hooks"],
  "hooks": "./hooks.json",
  "skills": "./skills",
  "commands": "./commands"
}
```

Блока `mcpServers` в манифесте нет намеренно: серверу, объявленному внутри плагина, Claude Code
даёт инструментам имена `mcp__plugin_aurelius_aurelius__*`, и все записанные ссылки на
`mcp__aurelius__*` переставали бы совпадать. Сервер регистрирует `install.sh` в пользовательской
области командой `claude mcp add -s user aurelius au mcp` (решение 05.09.2026, вариант B).
Отсутствие блока сторожит тест `plugin_json_bundles_no_mcp_server`, наличие регистрации в
`install.sh` — тест `install_sh_registers_mcp_server_user_scope`.

`version` совпадает с `[workspace.package].version` в `Cargo.toml` — проверяет тест
`crates/au/tests/plugin_manifest.rs`. `license` — как в `Cargo.toml` workspace (проверить при
реализации, не выдумывать).

## `.claude-plugin/marketplace.json`

```json
{
  "name": "blysspeak",
  "owner": { "name": "Vladislav Rahmanov" },
  "description": "Blysspeak plugins for Claude Code: aurelius, long-term memory with session hooks and skill cards.",
  "plugins": [
    {
      "name": "aurelius",
      "source": "./plugin",
      "description": "Long-term memory for Claude Code: session hooks, skill cards and /pickup; the MCP server is registered by install.sh."
    }
  ]
}
```

Корень плагина — каталог `plugin/`, а не корень репозитория: `claude plugin install` копирует
корень плагина целиком в `~/.claude/plugins/cache`, и корень репозитория утащил бы за собой
`target/` на десятки гигабайт (установка 3.4.1 записала 17 ГБ и не завершилась). Манифест
маркетплейса остаётся в корне — там его ищет `claude plugin marketplace add <клон>`. Размер
каталога `plugin/` сторожит тест `plugin_root_stays_small`.

Схема валидируется командой `claude plugin marketplace add ./` из корня клона — её вывод и есть
приёмка этого файла. Если Claude Code требует поля, не перечисленные здесь, они добавляются, а
контракт обновляется.

## `plugin/hooks.json`

```json
{
  "description": "aurelius: memory snapshot and skill index at session start, database backup, file-touch and action trace after tool use, reindex and judge on stop. Every command is the au binary from PATH.",
  "hooks": {
    "SessionStart": [
      { "matcher": "", "hooks": [
        { "type": "command", "command": "au", "args": ["skills", "--hook"], "timeout": 10 },
        { "type": "command", "command": "au", "args": ["snapshot", "--hook"], "timeout": 10 },
        { "type": "command", "command": "au", "args": ["db", "backup", "--hook"], "timeout": 30 }
      ] }
    ],
    "PostToolUse": [
      { "matcher": "Edit|Write", "hooks": [
        { "type": "command", "command": "au", "args": ["touch", "--hook"], "timeout": 5 }
      ] },
      { "matcher": "Bash|PowerShell|Edit|Write|NotebookEdit", "hooks": [
        { "type": "command", "command": "au", "args": ["trace", "--hook"], "timeout": 5 }
      ] }
    ],
    "Stop": [
      { "matcher": "", "hooks": [
        { "type": "command", "command": "au", "args": ["reindex", "--hook"], "timeout": 15 },
        { "type": "command", "command": "au", "args": ["judge", "--hook"], "timeout": 20 }
      ] }
    ]
  }
}
```

Инварианты: семь команд, все `command: "au"`, ни одного пути к скрипту, ни одного `bash`/`python`.
Если exec-форма с `args` не принимается локальной версией Claude Code (проверяется первым же
`claude plugin install`), допускается shell-форма `"command": "au skills --hook"` без поля
`shell` — `au` резолвится через PATH на обеих ОС; контракт тогда обновляется с указанием версии.

## `plugin/skills/aurelius-cards/SKILL.md`

```markdown
---
name: aurelius-cards
description: Reusable how-to cards (au CLI reference, agent checkpoints, workflow orders) live in aurelius memory, not in this plugin. Load when a task needs one of them - the index arrives at SessionStart, the body comes from skill_get.
---

The cards are stored in the aurelius knowledge graph and served by the MCP server this plugin
registers. Do not look for their text here.

1. The SessionStart hook already printed the index: one line per card, name plus trigger.
2. Fetch a body by name: `mcp__aurelius__skill_get(name: "<card-name>")`.
3. No index in context (hook failed or was disabled): `mcp__aurelius__skill_list()`.
4. Working out a repeatable procedure worth keeping: `mcp__aurelius__skill_save(...)` - it lands
   next to the others and shows up in the next session's index.
```

Инвариант: файл не содержит текста ни одной карточки — только маршрут к ним (решение `d70969e2`).

## `plugin/commands/pickup.md`

Команда подъёма состояния после `/clear` или холодного старта: то, что делает личная `/pickup`
владельца сегодня — снимок памяти проекта (`au snapshot --project <cwd name>` или
`memory_status`), открытые задачи (`task_list` по проекту), состояние git (ветка, последний
коммит, незакоммиченные файлы), и указание продолжать с хвоста последней сессии, не переспрашивая
то, что уже написано. Текст берётся из `~/.claude/commands/pickup.md` владельца с двумя правками:
без ссылок на личные пути и без проверки «цикла правки самого pickup».

Инвариант: команда не пишет в память, только читает.
