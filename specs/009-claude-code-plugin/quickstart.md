# Quickstart: плагин Claude Code для aurelius

## Чистая машина (Linux, macOS)

```
git clone https://github.com/Blysspeak/aurelius && cd aurelius
cargo build --release
install -m 755 target/release/au target/release/aurelius ~/.local/bin/   # каталог должен быть в PATH
au init
claude plugin marketplace add Blysspeak/aurelius      # или путь к клону: claude plugin marketplace add .
claude plugin install aurelius@blysspeak -s user
```

Перезапустить Claude Code. В новой сессии:

- в контексте есть снимок памяти и индекс карточек (SessionStart);
- `memory_status` отвечает; блок `server.version` равен `au --version`;
- `claude plugin list` показывает `aurelius@blysspeak`.

## Существующая машина (стояло руками или старым `install.sh`)

```
cd aurelius && git pull && ./install.sh
```

`install.sh` соберёт бинарники, поставит плагин и снимет старые записи из
`~/.claude/settings.json` и `~/.claude.json`, напечатав каждую с причиной; рядом с файлами лягут
копии `.bak-<дата>`. Перезапустить Claude Code. Проверка: за одну сессию каждый хук aurelius
срабатывает один раз (нет дублей снимка при старте, нет двух переиндексаций на Stop).

## Windows

```
git clone https://github.com/Blysspeak/aurelius; cd aurelius
cargo build --release
copy target\release\au.exe %USERPROFILE%\.local\bin\      # каталог должен быть в PATH
au init
claude plugin marketplace add Blysspeak/aurelius
claude plugin install aurelius@blysspeak -s user
```

Если раньше хуки и сервер были прописаны руками: удалить из `%USERPROFILE%\.claude\settings.json`
хуки с командами `aurelius-*.sh` или `au … --hook`, и ключ `mcpServers.aurelius` там и в
`%USERPROFILE%\.claude.json` (сначала скопировать оба файла). Git Bash и python3 не нужны: все
команды хуков — `au`.

## Проверка хуков вручную

```
echo '{"tool_input":{"file_path":"README.md"}}' | au touch --hook; echo $?     # 0
au db backup --hook; ls ~/.local/share/aurelius/backups/ | tail -1               # свежий снимок или ничего (троттлинг 24 ч)
AURELIUS_HOOK_DEBUG=1 au reindex --hook < /dev/null                              # причина, если что-то не так
```

## Разработка плагина

```
claude plugin marketplace add .        # из корня клона, один раз
claude plugin install aurelius@blysspeak
# после правок plugin/hooks.json или манифеста:
claude plugin marketplace update blysspeak && claude plugin update aurelius
cargo test -p au --test plugin_manifest   # версия манифеста == версия workspace
```
