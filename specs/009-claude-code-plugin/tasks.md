# Tasks: Плагин Claude Code для aurelius

**Input**: Design documents from `/specs/009-claude-code-plugin/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: конституция (принцип V) требует тест на каждое поведение — тесты включены как часть
задач реализации, не отдельной фазой.

**Organization**: по историям спеки. Волны исполнения (ulika, один Workflow = один наряд) идут
последовательно: W-A манифесты → W-B флаги `--hook` → W-C `install.sh` и документы → живая
проверка. Причина последовательности: тест `plugin_manifest.rs` из W-A попадает в
`cargo test --workspace` любой следующей волны, а CHANGELOG правят W-B и W-C.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

- [ ] T001 Проверить метаданные для манифеста в `Cargo.toml` (`[workspace.package]`: version 3.3.1, license MIT, authors, repository) — значения копируются в `.claude-plugin/plugin.json`, не выдумываются

## Phase 2: Foundational — манифесты плагина (волна W-A)

**Purpose**: без манифестов нет ни установки (US1), ни миграции (US2), ни проверки версии (US4).

- [ ] T002 [P] Создать `.claude-plugin/plugin.json` по `contracts/plugin-layout.md`: name aurelius, version = версия workspace, mcpServers.aurelius = {command au, args [mcp]}, пути hooks/skills/commands под `./plugin/`
- [ ] T003 [P] Создать `.claude-plugin/marketplace.json` по контракту: маркетплейс blysspeak, один плагин aurelius, source "./"
- [ ] T004 [P] Создать `plugin/hooks.json` по контракту: семь хуков в exec-форме, команда au, матчеры и таймауты из data-model.md
- [ ] T005 [P] [US5] Создать `plugin/skills/aurelius-cards/SKILL.md` по контракту: маршрут к карточкам через skill_get, без текста карточек
- [ ] T006 [P] [US5] Создать `plugin/commands/pickup.md` из `~/.claude/commands/pickup.md` владельца (читать, не менять): без личных путей и без проверки «цикла правки pickup»
- [ ] T007 [US4] Написать `crates/au/tests/plugin_manifest.rs`: версия plugin.json == CARGO_PKG_VERSION; mcpServers.aurelius = au mcp; hooks.json по пути из манифеста содержит ровно 7 команд, каждая command == "au" и без поля shell; marketplace.json: plugins[0].name == aurelius, source == "./"
- [ ] T008 Проверить манифесты локальным `claude`: `claude plugin details` с путём к клону или, если путь не принимается, синтаксическая проверка JSON всех трёх файлов; в профиль пользователя плагин на этом шаге НЕ ставится

**Checkpoint W-A**: `cargo test -p au --test plugin_manifest` зелёный; манифесты валидны.

---

## Phase 3: User Story 1 — установка на чистом профиле (P1) 🎯 MVP (волна W-B)

**Goal**: семь хуков плагина работают без bash и python3 — три обёртки заменены флагом `--hook`.

**Independent Test**: `echo '{"tool_input":{"file_path":"README.md"}}' | au touch --hook; echo $?` → 0;
`AURELIUS_HOME=$(mktemp -d)` → `au init`, дважды `au db backup --hook` → ровно один снимок.

- [ ] T009 [US1] Новый модуль `crates/au/src/hooks.rs`: разбор полезной нагрузки хука из stdin (serde_json::from_reader, не-JSON → None), извлечение `tool_input.file_path` / `tool_input.path` / `cwd`; чистые функции `throttled(newest_mtime, now, min_hours)` и `to_delete(snapshots, keep)`; debug-строка в stderr при `AURELIUS_HOOK_DEBUG=1`; юнит-тесты на разбор (4 образца), троттлинг (3 случая), ротацию (2 случая) в `#[cfg(test)]`
- [ ] T010 [US1] `au touch --hook` в `crates/au/src/main.rs` (флаг `--hook`, конфликтует с позиционным PATH) и `crates/au/src/commands.rs` (диспетчер → `hooks::touch_hook`): путь из нагрузки, не файл → выход 0, иначе существующая логика touch; узлы не создаются
- [ ] T011 [US1] `au reindex --hook` в `main.rs` (флаг, конфликтует с `--path`) и `commands.rs` (→ `hooks::reindex_hook`): корень из `cwd` нагрузки или текущего каталога, подъём до корня git, существующий reindex, затем существующий share push для всех sync-проектов; отказ первого не отменяет второго
- [ ] T012 [US1] `au db backup --hook [--keep N] [--min-hours H]` в `main.rs` (флаги, `--hook` конфликтует с `--out`) и `commands.rs` (→ `hooks::db_backup_hook`): каталог `<dir(db_path())>/backups`, троттлинг по mtime новейшего `aurelius-*.db`, имя `aurelius-<UTC %Y%m%dT%H%M%SZ>.db`, существующие `db_backup_cli` и `db_check_cli`, провал проверки → `.FAILED-CHECK` + удаление `-wal/-shm`, ротация; переменные `AURELIUS_BACKUP_KEEP`, `AURELIUS_BACKUP_MIN_HOURS`, флаг сильнее переменной
- [ ] T013 [US1] Интеграционный тест `crates/au/tests/hook_flags.rs` на собранном бинарнике с временным `AURELIUS_HOME`: touch --hook с не-JSON → 0; db backup --hook дважды → один файл; `--min-hours 0` с паузой больше секунды → второй файл; `--keep 2` после трёх → два новейших
- [ ] T014 [US1] CHANGELOG.md Unreleased / Added: один буллит про флаги `--hook` (обёртки заменены, Windows без Git Bash и python3)

**Checkpoint W-B**: fmt, clippy, test зелёные через verify-run; ручной прогон трёх команд из
`contracts/au-cli-hooks.md`, раздел «Реальный прогон».

---

## Phase 4: User Story 2 — миграция машины с ручной установки (P1) (волна W-C)

**Goal**: `install.sh` ставит плагин и снимает старые записи; ничего не пишет в JSON кроме снятия.

**Independent Test**: прогон `migrate_legacy()` на копиях живых `settings.json` и `~/.claude.json`
в `$TMP`: семь строк «снято» для хуков, одна для сервера, чужие хуки на месте, `.bak-*` создан,
повтор → «миграция не требуется».

- [ ] T015 [US2] `install.sh`: удалить раздел 6 (копирование обёрток в ~/.claude/hooks) и раздел 7 (python-правка settings.json с добавлением mcpServers и хуков)
- [ ] T016 [US2] `install.sh`: шаг установки плагина по `contracts/install-migration.md` — `claude plugin marketplace add "$SCRIPT_DIR"` (или `update blysspeak`, если уже добавлен), `claude plugin install aurelius@blysspeak -s user -y` (или update), `claude plugin list`; `claude` не найден → предупреждение с командами, код возврата не меняется
- [ ] T017 [US2] `install.sh`: python3-блок `migrate_legacy()` по контракту — признаки записей, бэкап `<файл>.bak-<UTC>`, строки «снято: …», удаление пустых групп/событий/mcpServers, «миграция не требуется» при отсутствии, напоминание про `~/.claude/hooks/aurelius-*.sh` и `~/.local/share/mcp/aurelius`; каталог Claude берётся из `CLAUDE_HOME` (по умолчанию `~/.claude`), файл `~/.claude.json` — из `CLAUDE_JSON` (по умолчанию `~/.claude.json`), чтобы блок можно было прогнать на копиях
- [ ] T018 [US2] Установка бинарников в `install.sh` через временный файл и `mv` (ETXTBSY при живых MCP-серверах), с копией `.bak-<UTC>` прежнего бинарника
- [ ] T019 [US2] Проверка: `bash -n install.sh`; прогон `migrate_legacy()` на копиях живых файлов владельца в `$TMP` через `CLAUDE_HOME`/`CLAUDE_JSON` — ожидаемый вывод из Independent Test; `grep -n "settings.json\|\.claude\.json" install.sh` находит только миграционный блок

**Checkpoint W-C**: `install.sh` проходит `bash -n` и прогон миграции на копиях.

---

## Phase 5: User Story 3 — Windows (P2)

**Goal**: та же интеграция без Git Bash и python3.

**Independent Test**: на Windows-машине владельца по quickstart: сессия открыта и завершена, семь
хуков без ошибок «команда не найдена».

- [ ] T020 [US3] Убедиться тестом T007, что ни одна команда hooks.json не зависит от оболочки (command == "au", нет поля shell, нет путей к скриптам) — покрыто T007, отдельного кода нет
- [ ] T021 [US3] README.md: раздел Windows по quickstart (сборка, PATH, две команды плагина, ручное снятие старых записей таблицей) — часть T024
- [ ] T022 [US3] Живая проверка на Windows-машине — владелец, вне сессии; результат записать в au (`task_log a4b486b3`)

---

## Phase 6: User Story 4 — версия в ногу с релизом (P2)

- [ ] T023 [US4] Покрыто T007 (тест равенства версий); в `/release` при бампе `Cargo.toml` бампается и `.claude-plugin/plugin.json` — иначе `cargo test` красный до релиза; зафиксировать это одной строкой в README, раздел «Release»

---

## Phase 7: User Story 5 — подъём и карточки из плагина (P3)

- [ ] Покрыто T005 и T006 (волна W-A); проверка — после живой установки команда `/aurelius:pickup` в списке команд и выдаёт снимок

---

## Phase 8: Polish & Cross-Cutting (волна W-C, продолжение)

- [ ] T024 README.md: раздел «Установка» переписан (чистая машина, существующая машина, Windows), раздел «Claude Code Integration» ссылается на `plugin/hooks.json` как источник истины, `contrib/claude-code/*.sh` помечены устаревшими, строка про бамп версии плагина при релизе
- [ ] T025 [P] `contrib/claude-code/*.sh` и `contrib/claude-code/install.sh`: шапка-комментарий «устарело с 3.4.0, заменено плагином aurelius; удаляется в следующем мажоре»
- [ ] T026 CHANGELOG.md Unreleased: Added — плагин Claude Code (маркетплейс blysspeak, семь хуков, au mcp); Changed — install.sh ставит плагин и снимает старые записи, больше не правит settings.json/~/.claude.json
- [ ] T027 Гейты через verify-run: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`; `bash -n install.sh`

---

## Phase 9: Живая проверка (после мержа W-A..W-C, действия оркестратора и владельца)

- [ ] T028 Релиз 3.4.0 по `/release` (feat), пересборка, установка бинарников
- [ ] T029 [US2] На Linux-машине владельца: `./install.sh` → вывод миграции (7 хуков + 1 сервер снято, бэкапы), `claude plugin list` показывает aurelius@blysspeak; рестарт Claude Code — владелец
- [ ] T030 [US1] В новой сессии: снимок и индекс карточек при старте, `memory_status` с server.version 3.4.0 и restart_needed false, за сессию каждый хук по одному разу (без дублей); результаты в `task_log a4b486b3`, критерии задачи отмечены

## Dependencies

- W-A (T001–T008) → W-B (T009–T014) → W-C (T015–T019, T024–T027) → T028 → T029 → T030.
- US3 и US4 кода не добавляют: закрываются тестом T007 и документами.
- US5 закрывается в W-A.

## Parallel Execution

- Внутри W-A: T002–T006 — разные файлы, один исполнитель пишет их подряд (объём мал); T007 после них.
- Внутри W-C: T025 параллелен T015–T018 (разные файлы), но один исполнитель — ради одного CHANGELOG.
- Между волнами параллелизма нет: общий `cargo test --workspace` и общий CHANGELOG.

## Implementation Strategy

MVP = W-A + W-B: плагин можно поставить на чистую машину руками по quickstart, хуки работают.
W-C добавляет миграцию для существующих машин и документы. Живая проверка — только после релиза,
потому что плагин зовёт `au` из PATH, и там должен стоять бинарник с флагами `--hook`.
