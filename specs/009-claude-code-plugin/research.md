# Research: Плагин Claude Code для aurelius

**Date**: 2026-09-05 | **Spec**: [spec.md](spec.md)

Источники: документация Claude Code по плагинам (выжимка — узел au `0afc5d2e`), справка
локального `claude` 2.1.261 (`claude plugin --help`, `plugin install --help`, `plugin marketplace
--help`), живой `~/.claude/settings.json` и `~/.claude.json` владельца, `install.sh` и
`contrib/claude-code/*.sh` этого репозитория (узел `227dc31b`), `au --help` и справка подкоманд.

## R1. Команда MCP-сервера

- **Decision**: `au mcp`.
- **Rationale**: `crates/au/src/commands.rs::mcp()` и `crates/aurelius/src/main.rs` зовут одну
  функцию `aurelius::mcp::serve()`; сервер один. `au` обязан быть в PATH ради хуков, значит для
  плагина достаточно одного бинарника. Совпадает с тем, что `install.sh` писал в `settings.json`.
- **Alternatives considered**: бинарник `aurelius` (так зарегистрировано сегодня в
  `~/.claude.json` через `~/.local/share/mcp/aurelius`) — второй бинарник в PATH без выигрыша;
  оставить оба — два места, две версии.

## R2. Форма команд хуков

- **Decision**: exec-форма `{"type":"command","command":"au","args":[...]}`, без оболочки.
  Три обёртки заменяются флагом `--hook` у `touch`, `reindex`, `db backup`; четыре хука уже
  нативны (`skills`, `snapshot`, `trace`, `judge`).
- **Rationale**: документация: exec-форма с `args` сама экранирует пути и не зависит от оболочки;
  на Windows `au` разрешается в `au.exe` через PATH. Обёртки требуют bash и python3.
- **Alternatives considered**: shell-форма с `"shell":"bash"` — требует Git Bash на Windows;
  переписать обёртки на PowerShell + bash парами — два набора скриптов на одно поведение.

## R3. Что читает хук из stdin

- **Decision**: `au touch --hook` берёт `tool_input.file_path`, при отсутствии — `tool_input.path`;
  `au reindex --hook` берёт корень проекта из поля `cwd` полезной нагрузки хука, при отсутствии —
  из текущего каталога процесса, затем поднимается до корня git.
- **Rationale**: обёртка `aurelius-track-edit.sh` извлекала ровно эти два поля; поле `cwd` в
  полезной нагрузке хуков Claude Code документировано и точнее, чем `pwd` процесса.
- **Alternatives considered**: `au touch --hook` создаёт узлы для новых файлов — обёртка этого не
  делала намеренно («creates NO new nodes»), поведение сохраняется.

## R4. Раскладка плагина в репозитории

- **Decision**: манифест в `.claude-plugin/plugin.json` (обязательное место), содержимое под
  `plugin/` с путями в манифесте (`hooks`, `skills`, `commands`), `mcpServers` — прямо в
  `plugin.json`.
- **Rationale**: корень Rust-репозитория не обрастает каталогами `hooks/ skills/ commands/`;
  корневой `.mcp.json` Claude Code прочитал бы и как проектную MCP-конфигурацию при работе в
  самом репозитории — второе объявление того же сервера с запросом подтверждения.
- **Alternatives considered**: всё в корне по умолчаниям плагина — мусор в корне и двойной
  `.mcp.json`; отдельный каталог `claude-plugin/` с манифестом внутри — Claude Code ищет манифест
  в `.claude-plugin/` корня плагина, а корень плагина в маркетплейсе можно указать как
  подкаталог, но тогда версия репозитория и плагина живут в разных корнях.

## R5. Установка: маркетплейс внутри репозитория

- **Decision**: `.claude-plugin/marketplace.json` рядом с манифестом: маркетплейс `blysspeak`, один
  плагин `aurelius` с `source: "./"`. Локальный клон: `claude plugin marketplace add <путь к
  клону>`; чужая машина: `claude plugin marketplace add Blysspeak/aurelius`. Затем
  `claude plugin install aurelius@blysspeak -s user`.
- **Rationale**: `claude plugin install` в 2.1.261 ставит только из маркетплейсов; `marketplace
  add` принимает URL, путь или GitHub-репо. Имя `blysspeak` — пространство владельца, куда позже
  ляжет и ulika.
- **Alternatives considered**: `claude --plugin-dir` — только для отладки, не переживает
  перезапуск; отдельный репозиторий-маркетплейс — второе место с версией и ещё один клон.
- **Open until implementation**: точная схема `marketplace.json` проверяется командой
  `claude plugin marketplace add ./` на локальном клоне — она валидирует манифест; поля `name`,
  `owner`, `plugins[].name/source/description` по документации.

## R6. Версия плагина

- **Decision**: явное поле `version` в `plugin.json`, равное `[workspace.package].version`;
  интеграционный тест в `crates/au/tests/plugin_manifest.rs` читает манифест по
  `CARGO_MANIFEST_DIR/../../.claude-plugin/plugin.json` и сравнивает с `CARGO_PKG_VERSION`.
  Релизный флоу бампает оба места; расхождение — красный `cargo test`, то есть красный гейт релиза.
- **Rationale**: документация: `version` пинует обновления у пользователей; фолбэк на
  `Cargo.toml` при отсутствии поля есть, но неявное правило хуже явного числа с проверкой.
- **Alternatives considered**: без поля, фолбэк — никто не увидит, если фолбэк перестанет
  работать; скрипт в CI — у репозитория нет CI, гейты локальные.

## R7. Миграция старых записей

- **Decision**: python3-блок в `install.sh` снимает: хуки, чья команда содержит
  `aurelius-(reindex|track-edit|skills|backup|capture)\.sh` или начинается с `au ` и содержит
  `--hook`; `mcpServers.aurelius` в `~/.claude/settings.json` и `~/.claude.json`. Перед правкой
  — копия файла `<файл>.bak-<UTC-метка>`; каждая снятая запись печатается строкой с причиной.
  Повторный запуск ничего не находит и говорит об этом.
- **Rationale**: хуки плагина сливаются с `settings.json` без дедупликации (документация), MCP из
  пользовательской области перекрывает плагинный целиком — без снятия плагин либо дублирует,
  либо не работает. python3 уже требовался прежним `install.sh`.
- **Alternatives considered**: нативная миграция в `au` — кроссплатформенно, но новая подкоманда
  ради разового шага (см. Complexity Tracking в плане); `jq` — не гарантирован.
- **Не снимается**: `aurelius-save-reminder.mjs` (не из этого репозитория), хуки ulika, файлы
  `~/.claude/hooks/aurelius-*.sh` (печатаются как более не нужные, удаление — решение человека).

## R8. Подсказка при отсутствующем `au`

- **Decision**: не реализуется в хуке. Claude Code сам показывает ошибку хука с командой `au`;
  README и `description` плагина называют шаг установки. FR-006 в спеке ослаблен соответственно.
- **Rationale**: строку контекста может отдать только исполняемая команда; если `au` нет,
  исполнять нечего, а обёртка-оболочка — ровно то, от чего уходим.

## R9. Хук бэкапа: параметры

- **Decision**: `au db backup --hook [--keep N] [--min-hours H]`; умолчания 7 и 24, переопределение
  переменными `AURELIUS_BACKUP_KEEP`, `AURELIUS_BACKUP_MIN_HOURS` (как у обёртки); каталог
  `<data_dir>/backups`, имя `aurelius-<UTC %Y%m%dT%H%M%SZ>.db`; троттлинг по mtime новейшего
  `aurelius-*.db`; после снимка — `db check`; провал → переименование в `.FAILED-CHECK` и удаление
  `-wal/-shm` соседей; ротация — оставить N новейших.
- **Rationale**: поведение обёртки переносится один в один; каталог бэкапов уже существует и
  используется (`~/.local/share/aurelius/backups/`, шесть снимков на 05.09).
- **Alternatives considered**: троттлинг по календарному дню — сессия в 23:59 и в 00:01 дала бы
  два снимка; по числу правок — нет источника счётчика.

## R10. Диагностика в режиме хука

- **Decision**: код возврата 0 всегда; stderr пуст, кроме случая `AURELIUS_HOOK_DEBUG=1`, когда
  печатается причина отказа одной строкой.
- **Rationale**: Claude Code показывает stderr хука пользователю при ненулевом коде; при нулевом
  молчит. Отладка нужна редко, шум на каждом событии — всегда.
