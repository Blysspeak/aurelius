# Implementation Plan: Плагин Claude Code для aurelius

**Branch**: `009-claude-code-plugin` | **Date**: 2026-09-05 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/009-claude-code-plugin/spec.md`

## Summary

Интеграция aurelius с Claude Code (MCP-сервер, семь хуков, указатель на карточки, команда
подъёма) переезжает из ручных правок `~/.claude/settings.json` и `~/.claude.json` в плагин Claude
Code, который живёт в этом же репозитории и ставится через встроенный в репозиторий маркетплейс.
Три bash-обёртки хуков заменяются флагом `--hook` у существующих подкоманд `au` (`touch`,
`reindex`, `db backup`), чтобы хуки работали на Windows без Git Bash и python3. `install.sh`
собирает бинарники, ставит плагин и снимает старые записи с бэкапом; версия манифеста плагина
проверяется тестом на равенство версии workspace.

## Technical Context

**Language/Version**: Rust (edition workspace, toolchain текущий stable), bash для `install.sh`;
Claude Code 2.1.261 (измерено на машине владельца) — плагины с манифестом, хуками, MCP, скиллами,
командами и маркетплейсами.
**Primary Dependencies**: без новых крейтов — `clap`, `serde_json`, `rusqlite`, `chrono`, `uuid`
уже в `crates/au`. Формат плагина — по документации Claude Code (факт `0afc5d2e`).
**Storage**: SQLite, без изменений схемы. Бэкап — существующий `au db backup` (VACUUM INTO).
**Testing**: `cargo test --workspace` (юнит-тесты на разбор полезной нагрузки хука, троттлинг и
ротацию бэкапов, равенство версий манифеста и workspace); проверка реальным бинарником на
временном `AURELIUS_HOME`; живая миграция на Linux-машине владельца.
**Target Platform**: Linux и Windows (Claude Code на обеих; PATH содержит каталог бинарников).
**Project Type**: CLI + MCP-сервер + декларативные манифесты плагина.
**Performance Goals**: хуки укладываются в сегодняшние таймауты (5/10/15/20/30 с); бэкап ~50 мс
на 8 МБ базы (измерено обёрткой ранее).
**Constraints**: команды хуков — только `au` в exec-форме, без интерпретаторов; MCP-поверхность
не меняется; никаких новых зависимостей; `install.sh` пишет в чужие JSON только при снятии
старых записей, с бэкапом.
**Scale/Scope**: 7 хуков, 3 новых флага `--hook`, 2 манифеста (плагин, маркетплейс), 1 скилл,
1 команда, 1 тест версии, правка `install.sh`, README, CHANGELOG.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Принцип | Статус | Как соблюдается |
|---|---|---|
| I. Data Durability First | ✅ | `au db backup --hook` зовёт тот же VACUUM INTO и ту же проверку `db check`, что и обёртка; снимок, не прошедший проверку, переименовывается в `.FAILED-CHECK`, не удаляется; на путях записи нет `.ok()`-глушения — ошибки собираются и решают код возврата **процесса-хука**, см. Complexity Tracking |
| II. One Local File, Many Processes | ✅ | Путь к базе и открытие соединения — через существующие `db_path()` / `open_and_ensure`; новых точек открытия нет |
| III. Rust Clean Code | ✅ | `unwrap_used`/`expect_used` = deny уже в workspace; разбор JSON хука через `serde_json::from_reader` с `Result`; поля извлекаются `Option`-цепочкой |
| IV. Surgical Simplicity | ✅ | Ни одной новой подкоманды: три флага на существующих; манифесты — данные; обёртки не удаляются, а помечаются устаревшими |
| V. Verify Before Done | ✅ | Каждый флаг — тест на образце JSON хука; равенство версий — тест; живой прогон: `echo '{…}' \| au touch --hook` и `AURELIUS_HOME=<tmp> au db backup --hook` дважды подряд (второй — троттлинг); миграция на живой машине с `settings.json` владельца |
| VI. MCP Surface Stability | ✅ | Инструменты, параметры и результаты не меняются; меняется только место регистрации сервера |
| Quality Gates | ✅ | fmt, clippy `-D warnings`, test через `verify-run`; `install.sh` — `bash -n` плюс прогон миграции на копии `settings.json` |

**Post-design re-check (после Phase 1)**: без изменений — контракты не добавили ни новых зависимостей, ни изменений схемы, ни новых MCP-инструментов.

## Project Structure

### Documentation (this feature)

```text
specs/009-claude-code-plugin/
├── spec.md
├── plan.md              # этот файл
├── research.md          # Phase 0
├── data-model.md        # Phase 1
├── quickstart.md        # Phase 1
├── contracts/
│   ├── plugin-layout.md       # манифесты плагина и маркетплейса, hooks.json, скилл, команда
│   ├── au-cli-hooks.md        # au touch --hook, au reindex --hook, au db backup --hook
│   └── install-migration.md   # что install.sh ставит, что снимает, как печатает
├── checklists/requirements.md
└── tasks.md             # Phase 2 (/speckit-tasks)
```

### Source Code (repository root)

```text
.claude-plugin/
├── plugin.json              # манифест плагина: name aurelius, version = workspace, mcpServers inline, пути к hooks/skills/commands
└── marketplace.json         # маркетплейс blysspeak с одним плагином, source "./"
plugin/
├── hooks.json               # семь хуков, exec-форма, команда au
├── skills/aurelius-cards/SKILL.md   # указатель: карточки — через skill_get
└── commands/pickup.md       # команда подъёма состояния

crates/au/src/
├── main.rs                  # clap: --hook у touch, reindex, db backup (+ --keep, --min-hours)
├── commands.rs              # реализация трёх режимов --hook (или новый модуль hooks.rs, если commands.rs не вмещает)
└── ...
crates/au/tests/
└── plugin_manifest.rs       # версия .claude-plugin/plugin.json == CARGO_PKG_VERSION; marketplace.json ссылается на "./"

install.sh                   # разделы 6-7 (копия обёрток, python-правка settings.json) → установка плагина + миграция
contrib/claude-code/*.sh     # шапка «устарело, заменено плагином 3.4.0»
README.md                    # раздел установки: плагин (чистая машина, миграция, Windows)
CHANGELOG.md                 # Unreleased
```

**Structure Decision**: манифест плагина лежит там, где его ищет Claude Code (`.claude-plugin/` в
корне), а всё содержимое плагина — под `plugin/`, чтобы корень Rust-репозитория не обрастал
каталогами `hooks/`, `skills/`, `commands/`; пути указываются в манифесте. MCP-сервер объявлен
внутри `plugin.json`, а не отдельным `.mcp.json` в корне: корневой `.mcp.json` Claude Code читал бы
ещё и как проектную конфигурацию при работе в самом репозитории aurelius — второе объявление
того же сервера. Rust-часть — только `crates/au`: три флага на существующих подкомандах плюс
интеграционный тест манифеста, который живёт в `au`, потому что именно `au` — бинарник плагина.

## Phase 0 — Research

Все неизвестные закрыты в [research.md](research.md): команда сервера, форма команд хуков,
раскладка плагина, маркетплейс внутри репозитория, проверка версии, миграция, ограничение
подсказки при отсутствующем бинарнике, наблюдение контекста хука (`cwd`).

## Phase 1 — Design

- [data-model.md](data-model.md) — манифест плагина, набор хуков, запись MCP-сервера, наследные
  записи, полезная нагрузка хука, снимок базы.
- [contracts/](contracts/) — три контракта, см. дерево выше.
- [quickstart.md](quickstart.md) — чистая установка, миграция, Windows, проверка.
- Agent context: в корне репозитория нет `CLAUDE.md` с маркерами SPECKIT — шаг пропущен; ссылка
  на план живёт в `.specify/feature.json`.

## Waves (подсказка для tasks.md)

1. **au --hook** (Rust, `crates/au`): `touch --hook`, `reindex --hook`, `db backup --hook`; тесты;
   гейты. Это единственная волна с кодом, идёт первой — манифест хуков ссылается на эти флаги.
2. **Манифесты** (данные): `.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`,
   `plugin/hooks.json`, скилл, команда; тест равенства версий; локальная проверка
   `claude plugin marketplace add ./` и `claude plugin install aurelius@blysspeak`.
3. **install.sh + документы**: миграция, установка плагина, шапки в `contrib/`, README, CHANGELOG.
4. **Живая проверка**: миграция на Linux-машине владельца (рестарт — действие человека), журнал
   одной сессии без дублей; Windows — владелец на второй машине по quickstart.

Волны 1 и 2 не пересекаются по файлам и могут идти параллельно; 3 зависит от 2 (имя маркетплейса,
пути). Релиз после волны 3 — минорный (`feat`), 3.4.0.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Режим `--hook` глушит ошибки на границе процесса (принцип I: «write failures MUST propagate») | Хук, вернувший код ≠ 0, показывает пользователю ошибку на каждом событии, а Stop-хук с кодом 2 блокирует завершение хода; сессия не должна зависеть от состояния памяти. Так уже устроены `snapshot`, `trace`, `judge --hook` | Пропускать код возврата наружу — сессия Claude Code становится заложником базы; писать ошибки в stderr — Claude Code показывает stderr хука пользователю при ненулевом коде, при нулевом — нет. Компромисс: код 0 всегда, диагностика в stderr только при `AURELIUS_HOOK_DEBUG=1` |
| python3 остаётся в `install.sh` для снятия старых записей | Правка двух JSON-файлов с бэкапом — разовая миграция; `jq` не гарантирован, bash JSON не разбирает | Нативная миграция в `au` (`au install claude-code --migrate`) кроссплатформенна, но это новая подкоманда ради одноразового шага; Windows-машина владельца ставилась руками, снять две записи там — тоже руками, по quickstart |
