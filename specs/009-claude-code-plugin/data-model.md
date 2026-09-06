# Data Model: Плагин Claude Code для aurelius

Схема базы не меняется. Ниже — сущности конфигурации и полезных нагрузок, с которыми работает
фича.

## Манифест плагина (`.claude-plugin/plugin.json`)

| Поле | Значение | Правило |
|---|---|---|
| `name` | `aurelius` | пространство имён скиллов и команд (`/aurelius:pickup`) |
| `version` | `X.Y.Z` | равно `[workspace.package].version`; проверяется тестом |
| `description` | одна строка | называет шаг установки бинарников (`cargo build --release`, PATH) |
| `author`, `homepage`, `repository`, `license`, `keywords` | из `Cargo.toml` | информационные |
| `mcpServers` | отсутствует | в манифесте плагина сервер не объявлен: местом регистрации служит `~/.claude.json` → `mcpServers.aurelius` (пользовательская область, пишет `install.sh` командой `claude mcp add`) |
| `hooks` | `./plugin/hooks.json` | путь относительно корня плагина |
| `skills` | `./plugin/skills` | каталог скиллов |
| `commands` | `./plugin/commands` | каталог команд |

## Манифест маркетплейса (`.claude-plugin/marketplace.json`)

| Поле | Значение |
|---|---|
| `name` | `blysspeak` |
| `owner.name` | владелец |
| `plugins[0].name` | `aurelius` |
| `plugins[0].source` | `./` — плагин и есть репозиторий |
| `plugins[0].description` | как в манифесте плагина |

Инвариант: ровно один плагин; его `name` совпадает с `plugin.json`.

## Набор хуков (`plugin/hooks.json`)

Единственный источник истины о том, что aurelius делает в сессии Claude Code. Семь записей:

| # | Событие | Матчер | Команда `au` | Таймаут, с | Заменяет |
|---|---|---|---|---|---|
| 1 | SessionStart | `""` | `skills --hook` | 10 | `aurelius-skills.sh` |
| 2 | SessionStart | `""` | `snapshot --hook` | 10 | ручная запись |
| 3 | SessionStart | `""` | `db backup --hook` | 30 | `aurelius-backup.sh` |
| 4 | PostToolUse | `Edit\|Write` | `touch --hook` | 5 | `aurelius-track-edit.sh` |
| 5 | PostToolUse | `Bash\|PowerShell\|Edit\|Write\|NotebookEdit` | `trace --hook` | 5 | ручная запись |
| 6 | Stop | `""` | `reindex --hook` | 15 | `aurelius-reindex.sh` |
| 7 | Stop | `""` | `judge --hook` | 20 | ручная запись |

Правила: каждая команда — exec-форма (`command: "au"`, `args: [...]`); никаких путей к скриптам;
таймауты равны живым значениям `settings.json` владельца на 05.09.2026.

## Полезная нагрузка хука (stdin, JSON от Claude Code)

| Поле | Кто читает | Правило |
|---|---|---|
| `tool_input.file_path` | `touch --hook` | основной источник пути |
| `tool_input.path` | `touch --hook` | запасной источник пути |
| `cwd` | `reindex --hook` | корень поиска проекта; при отсутствии — текущий каталог процесса |
| остальное | — | игнорируется |

Не-JSON или пустой stdin → выход 0 без действий.

## Снимок базы (`<data_dir>/backups/aurelius-<UTC %Y%m%dT%H%M%SZ>.db`)

| Свойство | Правило |
|---|---|
| Создание | `VACUUM INTO` (существующий `au db backup`) |
| Проверка | `au db check <файл>`; провал → `<файл>.FAILED-CHECK`, соседи `-wal`/`-shm` удаляются |
| Троттлинг | новый снимок только если новейшему `aurelius-*.db` не меньше `min_hours` (24) |
| Ротация | остаются `keep` (7) новейших `aurelius-*.db`; `.FAILED-CHECK` не считаются и не удаляются |
| Переопределение | `--keep`, `--min-hours`; переменные `AURELIUS_BACKUP_KEEP`, `AURELIUS_BACKUP_MIN_HOURS`; флаг сильнее переменной |

## Наследные записи (объект миграции `install.sh`)

| Где | Что | Признак |
|---|---|---|
| `~/.claude/settings.json` → `hooks.*[].hooks[]` | хук aurelius | `command` содержит `aurelius-(reindex\|track-edit\|skills\|backup\|capture)\.sh`, или начинается с `au ` и содержит `--hook` |
| `~/.claude/settings.json` → `mcpServers.aurelius` | сервер | ключ `aurelius` — наследная всегда |
| `~/.claude.json` → `mcpServers.aurelius` | сервер | ключ `aurelius` — наследная, только если `command` не `au mcp` |

Не признак (не трогать): `aurelius-save-reminder.mjs`, любые команды без `au`/`aurelius-*.sh`,
хуки ulika. Пустая группа хуков после снятия удаляется целиком.

## Состояния установки

```
ручная (сегодня) ──install.sh──▶ мигрировано (плагин, старых записей нет) ──рестарт──▶ работает
                                     ▲
чистый профиль ──marketplace add + plugin install──┘
```

Повторный `install.sh` в состоянии «мигрировано» ничего не меняет и сообщает об этом.
