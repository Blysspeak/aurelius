# Changelog

## [Unreleased]

### Added
- **Режим `--hook` у `au touch`, `au reindex` и `au db backup`.** Три хука Claude Code были bash-обёртками из `contrib/claude-code`, которым нужны bash и python3; на Windows без Git Bash они не работали. Теперь au сам читает JSON события из stdin: touch `--hook` отмечает правку файла из tool_input.file_path, reindex `--hook` переиндексирует корень проекта из cwd события и публикует sync-проекты, db backup `--hook` снимает бэкап базы не чаще раза в сутки и хранит семь снимков, проверяя каждый. В режиме хука код возврата всегда 0; причина отказа — в stderr при `AURELIUS_HOOK_DEBUG`=1. Обёртки остаются до следующего мажора
- **Плагин Claude Code.** Интеграция aurelius — MCP-сервер `au mcp`, семь хуков сессии, указатель на карточки и команда подъёма — теперь плагин в этом же репозитории: маркетплейс blysspeak, установка двумя командами `claude plugin marketplace add` и `claude plugin install aurelius@blysspeak`. Хуки зовут `au` напрямую, без bash и `python3`, и работают на Windows. Единственный источник истины о хуках — `plugin/hooks.json`; версия плагина равна версии workspace и проверяется тестом

### Changed
- **`install.sh` ставит плагин и снимает старые записи.** Скрипт больше не копирует обёртки в `~/.claude/hooks` и не правит `settings.json`: он собирает бинарники, ставит плагин и снимает прежние хуки и запись MCP-сервера из `settings.json` и `~/.claude.json`, печатая каждую с причиной и оставляя копии файлов с датой. Повторный запуск ничего не трогает. Флаг `--migrate-only` делает только миграцию. Обёртки в `contrib/claude-code` помечены устаревшими и уйдут в следующем мажоре

## [v3.3.1] — 2026-09-05

### Fixed
- **`restart_needed` видит замену бинарника через `mv`.** Проверка брала путь к своему исполняемому файлу из `/proc/self/exe`; после замены файла на диске через `mv` (единственный способ заменить бинарник, который держат запущенные MCP-серверы — прямой `cp` падает с «текстовый файл занят») ссылка читается как «… (deleted)», метаданные по ней не достаются, и блок `server` в `memory_status` молча терял ключ `restart_needed` — ровно в том случае, ради которого проверка писалась. Удалённый образ теперь сам считается доказательством устаревания: `restart_needed`: true без сравнения времён

## [v3.3.0] — 2026-09-05

### Added
- **Задачу можно назвать префиксом id.** `task_view`, `task_update`, `task_log` и команды `au task` принимали только полный `UUID` или точный лейбл; восемь символов из `au task list` или из хэндоффа прошлой сессии отвечали «task not found», и каждая такая ссылка стоила лишнего поиска. Теперь уникальный префикс id от восьми символов находит задачу; неоднозначный префикс — ошибка с перечислением кандидатов, а не первый попавшийся узел. Правило одно, в ядре: CLI и MCP зовут его оба

### Changed
- **Отсев исполнимости ловит объём, названный числом.** Гейт `FR-005b` узнавал многоэтапную задачу по слову (эпик, `Phase N`, `MVP-N`) либо по числу критериев приёмки выше шести. Задача «разрезать 86 крупных боевых файлов» не попадала ни под то, ни под другое и уходила в машинный пул при стене наряда в 25 минут. Теперь количественный оборот в лейбле или критерии — от 10 файлов, вхождений, роутов, таблиц, миграций, модулей, компонентов, страниц — тоже маркер многоэтапности; годы и номера вида #296 под него не попадают

### Fixed
- **`task_update` не спорит сам с собой о предмете.** Проверка противоречий по `subject` искала любой узел с тем же предметом, включая тот, который сейчас правится: повторный `task_update` с тем же `subject` на той же задаче отклонялся как противоречие с самим собой. Теперь правимый узел исключён из поиска; чужой узел с тем же предметом по-прежнему требует `resolution`. `memory_add`, `task_create`, `task_log`, `memory_session` и `au note` создают новый узел, исключать им нечего

## [v3.2.0] — 2026-09-05

### Added
- **Поля происхождения принимают все ручки записи MCP.** `task_log`, `task_create`, `task_update` и `memory_session` берут те же `confidence`, `evidence`, `subject`, volatility, claim, measured_at и verify_with, что `memory_add`, тем же разбором. Раньше агент, только что прогнавший команду, не мог записать `measured` через них: канала не было, и каждый журнал работы, задача и сессия ложились непроверенными — отсюда снимок памяти сплошь unverified. Узлы, которые ручка создаёт попутно (решения, проблемы), наследуют `confidence` и `evidence`, но не `subject`: ключ предмета уникален для одного факта
- **`memory_status` говорит, что сервер устарел.** Блок `server` несёт версию запущенного MCP-сервера и `restart_needed`, если исполняемый файл на диске новее момента старта процесса. Дважды за день после установки нового бинарника сессия продолжала работать на старом образе, и агент узнавал об этом только по «unknown tool». Теперь это видно первым же вызовом

### Changed
- **Список задач перестал пересказывать журнал прогонов.** `task_list` по 16 задачам весил около 20 тысяч символов, из них большая часть — массив улик одной задачи из 35 записей с путями артефактов. Теперь в списке сводка: сколько улик, сколько зелёных, последняя зелёная с командой и временем; полный массив остался в `task_view`. Параметр `full_notes=true` отдаёт заметки целиком прямо в списке: раньше, чтобы прочитать заметки 16 задач, нужно было 17 вызовов, и небольшая модель-аудитор на этом умирала на лимите ходов. `au task list` показывает ту же сводку строкой под задачей, считает её та же функция ядра
- **Запись в журнал работы больше не берёт задачу в работу сама.** `task_log` в MCP и `au task log` при первой записи переводили задачу из очереди в active, минуя общее правило взятия в работу: не ставили `activated_at` и не вытесняли уже активную задачу проекта, так что в проекте оказывались две активные при правиле «не более одной». Запись наблюдения и решение взять задачу в работу — разные действия; второе теперь только явное, через `task_update status=active` или `au task activate`. Ответ и вывод команды говорят текущий статус и как активировать. Сам узел журнала создаётся одной функцией ядра вместо двух копий

### Fixed
- **Закрытие задачи из очереди больше не приписывает ей всю историю правок проекта.** Окно сбора файлов для способа решения считалось от `activated_at`, а при его отсутствии — от `created_at`: задача, заведённая три недели назад и закрытая через `task_update` без взятия в работу, получала в способ решения 61 файл, включая чужие спеки и временные репро-тесты, и отметку `confirmed`. Без взятия в работу нет и окна работы: файлы не собираются, коммит не читается из HEAD, подтверждённым способ решения считается только с явным коммитом или запросом слияния

## [v3.1.0] — 2026-09-05

### Added
- **Perplexity Search как второй провайдер `search_web`.** Параметр `provider` (`brave` по умолчанию, `perplexity`) выбирает, куда идёт запрос; ответ говорит, кто его дал. Кэш стал различать провайдеров: раньше он искал только по строке запроса, и запрос к Perplexity получил бы ответ Brave. Ключ читается из `PERPLEXITY_API_KEY` или `~/.config/aurelius/perplexity.key`, той же схемой, что у Brave; резолвер ключа один на обоих

## [v3.0.0] — 2026-09-05

Задача перестала зависеть от того, вспомнит ли человек сказать «она сделана». Раньше круг рвался в одном месте: работа шла, проверка проходила, а закрытие требовало отдельного усилия памяти — и задача висела открытой месяцами. Теперь улика прогона привязывается к задаче сама, а созревшую к закрытию машина предъявляет без вопроса «что там по задачам».

### Added
- **Задачу можно править после заведения.** `au task update` меняет приоритет, заметку и список критериев приёмки, `au task criterion --met/--unmet` отмечает отдельный критерий. Раньше задача была неизменяемой во всём, что имеет значение: после измерения, сдвинувшего её приоритет, приходилось заводить новую. Справка `au task` переведена на английский
- **Граф документации, первая волна.** `au graph import` загружает внешнюю документацию как узлы типа `doc` с типизированными рёбрами `prerequisite`, `next_step`, `defines`, `references`; `au path A B` и MCP `memory_path` показывают лесенку шагов между двумя узлами, `au graph export --format mermaid` рисует её. Слой vendor-docs живёт вне снимка и не съедает его бюджет
- **Три времени вместо одного статуса.** У задачи видно, когда заведена, когда взята в работу и когда закрыта. Статусы были и раньше, но времена переходов никуда не попадали: «done» не отличалось от «done месяц назад», и по списку нельзя было понять, что тянется, а что закрылось за час
- **Способ решения собирается из следов, а не из памяти человека.** При закрытии записывается, чем именно задача решена: коммит, ссылка на запрос слияния, изменённые файлы. Коммит определяется из состояния репозитория, файлы — из правок, привязанных к задаче по ходу работы. Флаги `--commit` и `--pr` остались, но уточняют, а не заменяют: система не спрашивает того, что уже знает. Закрытие без сведений всё равно проходит — задача честно помечается закрытой без подтверждения, а не выглядит полноценно закрытой при пустом основании
- **Улика прогона привязывается к задаче.** `au task evidence` зовётся обвязкой ulika после каждой проверки и записывает команду, код возврата, время и путь к артефакту. Человек эту команду не вызывает: прогон и так состоялся, незачем просить пересказать его словами. Артефакт, который потом пропал, помечается утраченным — сведения о прогоне остаются, а ссылка не притворяется живой
- **Созревание к закрытию.** Задача считается созревшей, когда по ней была правка и есть зелёная улика свежее этой правки. Состояние не хранится, а вычисляется: хранимое «созрела» устаревало бы молча. `au task ripe` показывает такие задачи с основанием, `au task list` их помечает, а `au judge --hook` предъявляет в конце хода — закрытие остаётся решением человека, предъявление стало обязанностью машины
- **Отказ закрыть не повторяется до новой работы.** Отклонённое предложение молчит, пока по задаче не появится новая правка. Иначе предъявление превратилось бы в шум, который перестают читать
- **Координаты секретов.** `au secret add/list/rm` хранят не значение секрета, а место, где оно лежит: переменная окружения, путь к файлу, запись в менеджере паролей. Хранение значений отвергнуто целиком, включая шифрованные: расшифровывать всё равно пришлось бы перед вставкой в контекст, а всё попавшее в дамп попадает в транскрипт на диске, в бэкапы и уходит в API — оттуда секрет задним числом не вычистить. Строка, похожая на само значение, отклоняется с кодом 1 и объяснением, какой признак сработал

### Changed
- **Активная задача больше не выпадает из дампа.** Открытых задач 228, в дамп помещалось три-четыре самые свежие — при этом статус `active` в модели был, а снимок им не пользовался. Теперь активные выбираются отдельно, идут без бюджетного среза, и объём при нехватке отбирается у слоёв ниже приоритетом, а не у них
- **Записи в дампе не рвутся посреди слова.** Обрезка шла по числу символов, и 9 записей из 17 обрывались на середине слова. Теперь срез идёт по границе слова — правка общая для всех слоёв, не только для задач
- **Взятие задачи в работу вытесняет прежнюю активную.** В проекте не более одной активной задачи; вытесненная возвращается в очередь со всей своей историей и временами, и о вытеснении говорится вслух
- **Снимок перед сжатием контекста возвращается после него.** Обвязка писала «на чём остановились» узлом по умолчанию, из-за чего снимок ложился в слой решений с лимитом в пять узлов, а слой последних сессий читает только сессионные узлы. Круг записи был замкнут, круг чтения — нет

### Fixed
- **`memory_context` больше не возвращает весь граф.** Та же утечка через узел проекта, что у `task_view`, только хуже: посев идёт с пяти якорей, и каждый тянет свой хаб. Измерено на живой базе: глубина 2 по теме одной задачи давала 2809 узлов и 2,4 мегабайта, включая чужие проекты; глубина 1 — 12 узлов и 9 килобайт. Умолчание снижено до 1, явный `depth=2` остался как сознательный выбор и задокументирован тестом как протекающий
- **Граница проекта отсекала все файлы, а не чужие.** Каталог проекта записан после `canonicalize`, а пути правок приходят как их прислал Claude Code; на Windows это `\\?\A:\...` против `A:\...`, и сравнение по префиксу не совпадало никогда — закрытая задача оставалась без списка файлов. Обе стороны приводятся к одной форме перед сравнением
- **Задача больше не показывает чужие решения как свои.** `task_view` обходил граф на глубину 2 и на втором шаге уходил через ребро «задача — проект», а оттуда возвращался на все остальные задачи проекта вместе с их ветками. В выдаче по задаче, заведённой 30 августа, лежали 66 решений и 28 проблем от 15-17 августа — заведённых раньше неё самой. Ответ при этом весил 107 тысяч символов и не помещался в окно вызывающей модели, то есть ручка не работала по прямому назначению. Глубина снижена до 1; поверх добавлен предел на свою ветку с честным отчётом, сколько узлов каждого типа осталось за кадром и как их достать. Поля самой задачи не режутся никогда
- **Закрытие задачи через MCP не оставляло следа.** `task_update` писал старые `started_at` и `completed_at`, тогда как CLI пишет `activated_at`, `closed_at` и собирает способ решения. Задача, закрытая ассистентом, оставалась без времени закрытия и без основания — при том что `task_view` эти поля уже читал. Теперь оба пути пишут одно и то же, а логика вынесена в одно место: две копии правила расходятся молча
- **Способ решения больше не приписывает задаче чужой репозиторий.** Коммит определялся командой `git rev-parse HEAD` в рабочем каталоге процесса. Но aurelius — один демон на все репозитории сразу: сервер, запущенный в каталоге одного проекта, при закрытии задачи другого записывал ей коммит первого — и помечал этот способ решения подтверждённым. Ложь, выглядящая достовернее правды: пустое основание видно, а чужой коммит читается как настоящий. Теперь коммит берётся из каталога того проекта, которому принадлежит задача, а если каталог проекта неизвестен — не подставляется вовсе. Пустой способ решения честнее подставленного наугад
- **Список изменённых файлов не собирает правки соседних проектов.** Правки отбирались только по времени — с момента взятия задачи в работу, — а таблица следов вообще не хранит, к какому проекту относится правка. Пока задача одного проекта была в работе, файлы, которые правились в это же время в другом, попадали в её способ решения и в перечень «что изменено» у созревшей. Теперь отбор ограничен каталогом проекта задачи
- **Переоткрытая задача больше не объявляется созревшей из прошлой жизни.** Улики и времена при переоткрытии намеренно не стираются — они часть истории задачи. Но созревание считалось по ним без оглядки на то, что работа началась заново: задача, однажды закрытая с зелёной уликой, сразу после переоткрытия снова предъявлялась к закрытию, не дождавшись ни одной правки нового круга. Теперь созревание опирается на текущий круг работы: и правка, и улика должны быть свежее взятия в работу. Задачи, заведённые до появления этих времён, правилом не задеты
- **Повторное «взять в работу» не сдвигает время взятия.** Время ставилось при любом вызове со статусом `active`, включая вызов на уже активной задаче — например, попутно с изменением приоритета. Сдвиг вперёд отрезал от способа решения все файлы, изменённые до него: работа была, а в перечне её не видно. Теперь время ставится один раз, на настоящем переходе в работу
- **Одно испорченное поле задачи перестало обнулять все остальные.** Разбор читал шесть полей задачи одной операцией, и любое нечитаемое значение — число вместо строки в списке файлов, испорченная дата у одной улики — молча возвращал пустоту вместо всех шести сразу. Задача с настоящей правкой и зелёным прогоном переставала считаться созревшей, и ничто об этом не сообщало. Теперь поля читаются по отдельности, а улики — поэлементно: теряется только то, что действительно испорчено
- **Задача в MCP больше не находится по узлу другого типа.** Поиск задачи по неточному имени в крайнем случае падал на полнотекстовый поиск без ограничения типа — и находил решение или проблему с похожей меткой. Вызов «закрыть задачу» при опечатке в имени молча правил чужой узел вместо отказа. В командной строке это ограничение стояло с самого начала; теперь пути совпали
- **Наряд перестал обходить оба правила круга.** `au task claim` брал задачу в работу, не считаясь с тем, что в проекте уже есть активная — третий путь мимо правила «одна активная задача на проект», которое соблюдают два других. А `au task release --verdict done` закрывал задачу, не ставя ни времени закрытия, ни способа решения, и выбрасывал обязательный аргумент `--evidence` целиком: текст проверки, ради которой наряд и закрывался, нигде не сохранялся. Теперь оба идут через те же правила, что и остальные пути закрытия, а текст проверки становится обычной уликой прогона. Взятие наряда при живой чужой аренде отклоняется, а не вытесняет её: вытеснение вернуло бы задачу в общий пул, откуда её взял бы третий владелец при ещё не истёкшей аренде — одна двойная выдача чинилась бы ценой другой
- **Координаты секретов перестали течь в обычный поиск.** Координата — узел настроек, и полнотекстовый индекс брал его наравне со всеми: запрос по названию сервиса возвращал место хранения ключа тому, кто искал совсем другое. Фильтр стоял только в дампе памяти, то есть ровно тот «другой автоматический путь», который комментарий в коде объявлял невозможным, был открыт. Теперь правило одно и общее: явный запрос координат работает, поиск и подача памяти их не отдают
- **Защита от записи самого значения вместо места пропускала ключи AWS.** Проверка отбрасывала как «путь» любую строку с косой чертой — а секретный ключ AWS почти всегда её содержит, и ни один известный префикс к нему не подходит. Само значение записывалось как координата и оттуда уходило в поиск. Признак добавлен узко, по фактическому формату ключа, чтобы настоящие координаты — записи менеджера паролей, пути к файлам — по-прежнему принимались
- **Запрет паник стал проверяться компилятором.** Принцип III конституции запрещает `unwrap`/`expect`/`panic` на рантайм-путях, но `unwrap_used` и `expect_used` в clippy выключены по умолчанию, поэтому даже прогон с `-D warnings` их пропускал — правило держалось на памяти пишущего. Включены в `deny`; четыре законных места (`Regex::new().expect()` внутри `OnceLock`) накрыты точечными разрешениями с объяснением инварианта

### Removed
- **`au sync` и `au capture` изъяты — ломающее изменение командной строки.** Ревизия по фактическим вызовам: `au sync` не вызывается ни хуками, ни человеком, ни через MCP ни в одном из 19 репозиториев; хук, ради которого существовала `au capture`, не подключён ни в одном проекте, при этом README обещал его как рабочий. Вместе с ними ушли `timeforged.rs` и `connector.rs`. Обе команды остаются разбираемыми и отвечают внятным сообщением с кодом 1, а не ошибкой разбора аргументов: тот, у кого они в скриптах, узнает причину, а не увидит мусор

---

## [v2.1.0] — 2026-08-29

Задача научилась выдаваться исполнителю на срок и возвращаться в очередь сама, а очередь — отвечать на вопрос, какую задачу машина вообще способна закрыть без человека. Это две нижние ступени автономного прогона очереди; сам ночной цикл ими ещё не запускается.

### Added
- **Наряд — аренда задачи со сроком, владельцем и счётчиком попыток.** Задача, выданная исполнителю, до сих пор ничем не отличалась от лежащей в очереди: два процесса брали одну и ту же, а брошенная не возвращалась никогда. `au task claim` выдаёт задачу на срок и никому больше её не отдаёт, `renew` продлевает аренду, `release` закрывает или отпускает, `give-up` сдаёт наряд, когда исполнитель упёрся в человека: задача блокируется, а не возвращается в очередь — крутить её дальше машиной бессмысленно. Аренда с истёкшим сроком возвращается в пул сама, счётчик попыток при этом растёт: задача, трижды никем не доведённая до конца, уходит в блок вместо того, чтобы вечно кружить по очереди. Выдача идёт одним запросом с `RETURNING`, поэтому два одновременных `claim` физически не могут получить одну задачу — не «маловероятно», а невозможно
- **Отсев исполнимости — `au task fitness`.** Вопрос, на который отвечает разметка: является ли критерий приёмки проверкой с однозначным исходом. Не «упоминается ли в нём команда» — первая редакция гейтов спрашивала именно это и завышала машинный пул в полтора раза, засчитывая «Читает NodeInbound» за node-скрипт. Критерий признаётся проверкой, только если команда стоит в начале строки, заключена в обратные кавычки или соседствует с маркером исхода. Задача без такого критерия помечается требующей человека, задача со смешанными критериями откладывается целиком, без попыток автоматически расщепить
- **Вердикт обязан объяснять себя и стареет вместе с задачей.** Каждая пометка несёт обоснование — какой именно критерий признан проверяемым либо почему ни один не признан, — и хеш содержания задачи. Переписали текст задачи — прежний вердикт становится недействительным, а не тихо остаётся приговором к тому, чего уже нет. Пустое обоснование отклоняется: пометка без причины неотличима от угаданной
- **Разметка не может тронуть ничего, кроме своего вердикта.** Это не соглашение, а устройство: запись идёт отдельным `json_set` по `data.fitness`, общего пути правки узла у неё нет
- **Отклоняются задачи, заведомо не помещающиеся в наряд.** Эпик с настоящим критерием приёмки проходил бы отсев как машинный и сгорал по времени трижды подряд, тратя три полных бутстрапа исполнителя. Так же отклоняется задача, чей критерий требует запрещённого автономной работе действия: «PR зелёный в CI» — критерий безупречный, но закрыть его исполнитель не вправе, и наряд провалился бы на собственном предохранителе
- **`--dry-run` — сухой прогон по всей очереди, ничего не пишущий.** Печатает вердикт с обоснованием по каждой открытой задаче и итоговые числа. Читать надо именно обоснования: разметка автоматическая, человеком не утверждается, и это единственное место, где видно, за что задача признана машинной

### Notes
- Ночной цикл этой версией не запускается: внешний драйвер, гоняющий наряды до опустошения очереди, в неё не входит. Выпущены две нижние ступени — выдача задачи и отбор того, что вообще можно выдать
- Прогон по живой очереди на момент выпуска: восемь машинных задач из 228 открытых. Из них одна размечена ошибочно — «набрать статистику observe» имеет безупречный критерий, но статистика копится от работы человека, а не машины
- Известны два класса задач, проходящих отсев машинными вопреки смыслу: объём, названный числом («разрезать 86 файлов») вместо слова «эпик», и запись в живое невосстановимое хранилище. Оба заведены задачами и закрываются вместе с драйвером — до его появления наряды никто не исполняет
- Правило запрета огрублено намеренно: упоминание PR отклоняет задачу даже там, где PR — предмет работы, а не действие исполнителя. На всей очереди таких две, и обе и без правила требуют человека

---

## [v2.0.1] — 2026-08-17

### Fixed
- **Поиск требовал совпадения всех слов сразу, и одна неудачная форма обнуляла выдачу.** Пробел между словами в FTS5 означает `AND`, поэтому запрос «телеграм алертов» не находил ничего: слова «алертов» в индексе нет, есть «алерта». Пустой ответ при этом читался как «знания нет» — то есть отправлял в разведку за тем, что в памяти уже лежало. Слова соединяются через `OR`, а порядок наводит ранжирование: сначала записи, где совпало больше слов запроса, внутри группы — по релевантности bm25. Плохая форма слова теперь портит порядок, а не результат. Явный оператор заглавными (`a AND b`) по-прежнему исполняется как написан: намерение важнее умолчания
- **Выдача была отсортирована от худшего к лучшему.** `ORDER BY rank … DESC` — а `rank` в FTS5 это bm25, отрицательное число, где меньше значит релевантнее. Самое подходящее уходило в хвост и обрезалось лимитом. Найдено при разборе первой проблемы: пока выдача была AND-узкой, порядок внутри неё почти не был виден
- **Инструмент не отличал «не нашлось» от «запрос не сработал».** `memory_search` и `au search` возвращают `unmatched_terms` — слова, не давшие ни одного попадания, — и подсказку попробовать основу или префикс. Первое означает «иди и выясняй», второе — «спроси иначе», и цена ошибки между ними — целая ветка ненужной разведки
- **Подсказка утверждала больше, чем знала.** Первая её редакция на пустую выдачу отвечала «похоже, этого знания здесь просто нет» — утверждение о мире, которого у поиска на руках нет: по запросу «воркеры» выдача была пустой, а «~20 воркеров» лежало в трёх узлах, не совпала одна лишь словоформа. Ложное успокоение дороже молчания: читатель идёт делать заново уже сделанное. Теперь подсказка говорит ровно про строку поиска — «не встречается в этой форме», — и явно оговаривает, что об отсутствии знания она не свидетельствует
- **Ранжирование не смотрело на заголовок.** bm25 считает по всему документу, поэтому длинная сессионная запись, где нужные слова рассыпаны по простыне, обходила запись, состоящую из этих слов почти целиком: «Горячая перезагрузка флагов доведена до алертов и разбита на модули» не попадала в топ-3 по запросу из тех же слов. Совпадение теперь взвешивается по полю — `label` ×3, `claim` ×2, `note` ×1
- **Префикс в подсказке резался по фиксированной длине и на коротких словах не менял ничего.** Первые пять символов от «батчи» — это снова «батчи», то есть совет `батчи*` давал те же ноль попаданий, тогда как `батч*` находит четыре записи. Совет не работал ровно там, где нужен больше всего: у коротких слов окончание и есть вся разница. Теперь снимается одна последняя буква («воркеры» → `воркер*`), а слову короче четырёх букв префикс не предлагается вовсе — расширять там нечего, и несбыточный совет хуже его отсутствия
- **Проба валила `measured` на импортах по алиасу.** Из текста извлекаются путеподобные токены и проверяются на диске; `@/config/env.js` превращался в `/config/env.js`, которого нет и быть не может — алиас разворачивается сборщиком (`@/*` → `src/*`), а расширение в ESM-импорте не совпадает с расширением на диске (`.js` против `.ts`). Любая запись, цитирующая импорт, была обречена провалить пробу. Токены после `@` и `~` пробами больше не становятся
- **Провал пробы больше не понижает `confidence`.** Понижение прожило один день и успело обесценить то, ради чего вводилось: все записи одного проекта разом прочитались как `unverified`. Проба — улика слабая по устройству (путь в прозе, машина другая, репозиторий чужой), `evidence` с командой и кодом возврата — сильная, и слабая не имеет права перебивать сильную. Провал остаётся в `probe_warnings` как замечание, а не как приговор

### Notes
- Морфология по-прежнему не решена: `OR` с ранжированием снимает основную боль, но «алертов» и «алерт» для индекса остаются разными словами. Trigram-токенайзер, стеммер или семантический поиск — отдельная работа, требующая перестройки индекса.

---

## [v2.0.0] — 2026-08-17

Seven fixes on one root: memory accepted a claim about the world without asking where
it came from, and stayed silent when it accepted only half of what it was handed.

### Added
- **A fact now carries its provenance, and `confidence` is required.** A guess landed in the graph looking exactly like a measurement, and six weeks later read back as truth — the project already pays for this rule in money, where a number is never spoken without the query that produced it and the time it was measured. `memory_add` and `au note` take `confidence` (`measured` | `inferred` | `reported` | `unverified`), `evidence` — the command or query verbatim — and `measured_at`. `measured` without `evidence` is refused rather than accepted: a measurement nobody can repeat is an inference. An absent `confidence` reads as `unverified`, never as "probably measured", and anything below `measured` is marked on the way out, so the reader sees the doubt without having to look for it
- **Volatility, because `semantic` / `episodic` never caught it.** "The .env says true" is neither an event nor an eternal truth: it holds until the first edit of that file, and leaves no trace in git. `volatility` (`immutable` | `slow`, 30 d | `volatile`, 1 d) plus `verify_with` mean that past its age the fact comes back with "старше N дн — перепроверь вот этим" attached, instead of presenting itself as current
- **Contradictions are refused at the door.** "Disabled" and "enabled" could sit in the graph side by side without a word of objection, and the `supersedes` edge between them got placed by hand — that is, from memory, when someone happened to remember. An optional `subject` (`xhub:.env:REFUND_REQUESTS_ENABLED`) names what is being asserted; a second fact about the same subject is refused, listing what is already on record, until `resolution` says `supersede`, `refine` or `coexist`. The resolution then becomes an edge, so the next reader sees how the two relate instead of guessing
- **`claim` — the assertion in one or two lines, returned whole.** Recall clipped by character count, so the startup snapshot cut every fact mid-word: the substance sat at the end of a long note and never survived the ellipsis. A `claim` (≤240 chars) is never clipped; the long reasoning stays in the note and comes out on demand
- **`au capture --hook` — catching a fact at the moment of discovery.** The session-end hook shouts "save, compaction is near", which means memory gets written from an already degraded context, from a recollection of what happened. This one fires the other way round: `psql`/`ssh`/`kubectl`/`curl` just returned data, the output is still on screen, and the hook offers to save it as a measured fact with that exact command already in `evidence`. It writes nothing itself — a command with output is not yet a fact worth keeping, and auto-saving every successful query would turn the graph into a dump. The recogniser is deliberately narrow: a hook that fires on everything is read for a day and ignored thereafter
- **The seven provenance fields exist on both doors and share one parser.** `au note --confidence …` and `memory_add(confidence=…)` cannot drift apart on what a measured fact is

### Fixed
- **An unknown parameter name is now an error instead of a silent skip — this is the expensive one.** `memory_session` was called twice with wrong parameter names; both times the answer was `created: true`, and the only sign of trouble was the string `[unknown]` inside a label. The decisions and the next steps were gone, and two orphan nodes were left hanging outside the project. Every MCP call is now checked against the same `inputSchema` served in `tools/list` — one source of truth, so the check cannot drift from the contract — and an unknown name is refused with a "did you mean" suggestion and the words "ничего не записано". All wrong names are listed at once: fixing a call one name per attempt is the same waste the silent skip was. Enum values are checked too, which closes a matching hole — `NodeType::parse` used to turn a typo into `Custom(…)`, giving a node a type no query looks for, while the CLI rejected the same string
- **The response now says what was stored and what was dropped.** A parameter with a correct name and an empty value looked delivered: `decisions: []` silently records nothing, and the caller found out only by opening the graph. `memory_add` and `memory_session` return `stored_fields` and `dropped_fields`, and the session response additionally counts `decisions_written` and `problems_written`
- **A failed probe now downgrades `confidence` to `unverified` by itself.** `probe_warnings` worked and reported honestly, but it was a string in a response, and a string is easy to ignore. A fact whose verification failed no longer presents itself as measured — the field does the work without being read

### Notes
- BREAKING for callers of `memory_add`: `confidence` is now required. The intent is exactly that — a fact whose origin nobody stated is a fact nobody can trust.
- Schema V14 adds one partial expression index for `subject`. Records written before this release simply carry no provenance: they read as `unverified`, which is what they always were.
- The MCP server is a long-lived process: restart the client before expecting the new parameters to be accepted.

---

## [v1.12.0] — 2026-08-16 · вышло в составе v2.0.0

Собственного тега у 1.12.0 нет: релиз-PR провисел неслитым до следующей работы, и
обе вошли одним выпуском. Секция оставлена отдельной — это две разные работы, а не одна.

### Added
- **`au session` — the record layer 4 of the snapshot reads, written without a model in the loop.** «Последние сессии» is assembled exclusively from `Session` nodes, and the only thing able to write one was `memory_session` over MCP. A mechanical hook could reach for `au note`, but a note is not a session: it landed in layer 5, among lasting facts, next to decisions that are meant to outlive the day. So the most important record of a session depended on whether the model remembered to call a tool — and what does not happen mechanically does not happen. The writing itself moved into the core (`graph::record_session`): the MCP handler and the CLI now call one function, and what is left in the handler is only what belongs to the transport — task linking, the active-tasks hint, the sync push. Accepts the same shape the tool takes (`summary`, `decisions`, `problems_solved`, `next_steps`, `key_files`), from arguments or from a single JSON on stdin, deduplicated by `sha256(project|summary)` so a hook that fires twice for one occasion leaves one record. An unknown key in that JSON is an error, not a shrug (117e6b9)
- **`au relate` — edges from mechanics.** `au note --json` returns the id of what it wrote, but there was nothing to attach it to: `memory_relate` lived only in MCP, so everything written mechanically settled into the graph without a single edge — a heap, not a graph. The relation vocabulary moved into the core alongside it (`Relation::KNOWN` / `parse_known`), because the hand-written copy in the tool description had already drifted, missing `subtask_of` and `blocks`. Hyphens are accepted next to underscores, `part-of` is a spelling of `belongs_to` rather than a new variant — two names for one relation would have split every project-scoped query — and `refines` is added as a real one, since "makes earlier knowledge more precise without replacing it" had no equivalent. Repeating a call returns the existing edge and `created: false`; `add_edge` inserts with `OR IGNORE` and used to hand back an id that was not in the database (117e6b9)
- **A record now carries the run that wrote it.** The journal could not tell sessions apart at all: `session_id` lived only in the labile recall window, and the nodes themselves carried nothing. A session-end hook therefore saw every record of the project and had no way to separate its own from yesterday's — "collect everything I wrote this run" was impossible mechanically, only by eyeballing timestamps. `au note --session`, `au session --session` and `memory_add`/`memory_session` (`session_id`) stamp it; `AURELIUS_SESSION_ID` is the fallback for hooks whose stdin is already occupied by the note text. `au journal --session <id>` reads it back, which is the other half of the feature — stamping without a way to select is the same as not stamping. An unknown run stays an *absent* key rather than an empty string, so the selection can never accidentally match records nobody marked
- **`au context` prints the edges themselves**, not merely how many there were (117e6b9)

### Fixed
- **A hyphen in a search query was being parsed as an operator, so half the names in this graph were unsearchable.** `memory_search("rust-clean-code")` answered "no such column: clean" and `"skills-store"` answered "no such column: store": FTS5 reads a `MATCH` string as an expression, where `-` is `NOT` and `:` selects a column. Almost every skill and half the projects are named that way, so the symptom read as a broken database rather than as query syntax. Every word is now wrapped as a phrase — inside a phrase the operators lose their power, and the tokenizer still splits `skills-store` into two tokens and finds them adjacent — while explicit `AND`/`OR`/`NOT`/`NEAR` and a trailing prefix star survive, because existing callers lean on them. Wired into graph search, `doc_recall` and the web-search cache
- **Traversal returned every edge twice.** An edge is visible from both of its endpoints, so BFS collected it again on the next hop: the "N edges" count in `au context` was wrong and `memory_context` duplicated edges in its JSON. Invisible until the edges themselves were printed (117e6b9)
- **The exit codes were inverted.** clap answered `2` for a typo in an argument, while everything else — including an unreachable database — collapsed into `1`. The contract is now `0` done, `1` bad call, `2` storage unreachable, classified by walking the anyhow chain; no call site needed touching, since `db::open` already returns a typed `DbError`. The `--hook` variants remain the deliberate exception: they never fail and stay silent (117e6b9)

### Notes
- Schema V13 adds one partial expression index for the run stamp. Records written before this release simply carry no run — they read as "unknown", never as a match.
- The MCP server is a long-lived process: restart the client before expecting `memory_search` to stop failing on hyphens.

---

## [v1.11.1] — 2026-08-15

### Fixed
- **A URL, a version number and a fragment of a path were all being probed as files.** Stage-2 probes extract verifiable claims from a node's text and run them against the file system, so a claim that fails is reported back in `probe_warnings`. A single release note — one containing a GitHub link and a path to a source file — produced three warnings about paths that were never claimed and never existed. Three separate false positives shared one regex. A URL matched the pattern twice over: inside `https://`, the fragment `s:/` reads as a Windows drive, and everything after the host reads as an absolute path; URLs are now stripped *before* extraction, because once a URL has been cut into pieces its pieces are indistinguishable from paths. `/Blysspeak/aurelius/releases/tag/v1.11` passed as a path whose extension was `.11` — an extension made only of digits is a version number, not a file. And `crates/aurelius-core/src/graph/search.rs` was matched from its interior slash onward, yielding `/aurelius-core/src/graph/search.rs`, an assertion the author never made; Rust's regex engine has no lookbehind and `\b` does not help here, since there *is* a word boundary before a slash, so the character preceding a match is now examined directly. A false probe is worse than a missing one: it fires on every write and trains the caller to stop reading warnings, which is the only thing warnings are for (c7e3f16)

---

## [v1.11.0] — 2026-08-15

### Added
- **`au snapshot --json` — the snapshot in a shape a program can rely on.** Hooks are ordinary processes: they cannot reach the MCP server, so the CLI is their only channel, and until now that channel spoke Markdown. A consumer parsing `## N · Heading` with a regex depends on the layout, which means the next change of layout breaks it as quietly as a closed channel does. The shape is now fixed: `{"project":…,"facts":[{"kind","text","at"}]}`, where `kind` names the source layer (`userfact`, `task`, `problem`, `obligation`, `session`, `decision`, `concept`, `skill`, `digest`). An empty `facts` with exit 0 means "nothing to say"; no output, or a non-zero exit, means "broken" — the two states a silent channel used to be indistinguishable between. Separate exit codes per state are deliberately not added: the shape already tells them apart. Fact text is returned whole, without the per-layer budget clipping the Markdown form applies, because the budget belongs to the consumer and a silently shortened fact reads exactly like a short one. Both forms are assembled from a single gathering pass, so they cannot drift apart unnoticed (f49ec85)
- **`memory_add` takes a `project` and says so when a node is left unattached.** A node linked to no project — neither by the `[project]` label prefix nor by an edge — is not returned by any project-scoped query, and `memory_add` still answered `"created": true`. A write nobody will ever find has no business looking like a success. Passing `project` now creates the `belongs_to` edge (and the project node, if it is missing); omitting it puts an `attachment_warning` in the response naming the consequence — the node will appear in neither `memory_status(project=…)` nor the snapshot. Types that are global by nature (`project`, `userfact`, `skill`) are exempt. The rule is stated in the tool schema, because a parameter an agent never reads about is a parameter that does not exist (183f7b2)

### Fixed
- **Project membership was a string convention, so half the graph was invisible to the project it belonged to.** A snapshot of a project with a full graph behind it returned only the two housekeeping layers — the node counter and the distillate — while `memory_status` for the same project returned nothing at all. Membership was read exclusively from the label: `label LIKE '[project]%'`, with `memory_status` additionally hunting for knowledge by full-text-searching the literal `"[project]"`. But `memory_add` stores a plain label and the link to the project is a separate edge created by `memory_relate` — an edge no query read. The documented way to record knowledge produced nodes unreachable by every project-scoped lookup, and the failure was silent in both directions: an empty section is indistinguishable from an empty subject. Four hand-rolled filters are replaced by one predicate, `project_scope_sql` — label **or** edge. Edge direction and relation type are deliberately not checked: `memory_relate` links node→project, the indexer links project→file, and the relation vocabulary is open, so a miss there would again mean losing knowledge quietly; false positives stay bounded by the caller's node-type filter. Two smaller defects of the same family are fixed alongside. `task_create` writes `status: "backlog"` while every query asked for `active,blocked`, so a freshly created task was invisible everywhere until someone activated it by hand — creating a task meant losing it; `OPEN_TASK_STATUSES` now covers all three. And the placeholder distillate, "Хвостов нет — чисто.", is dropped from both output forms: it carries zero information, spends layer budget, and made a brand-new project return a one-line body instead of an empty one (32db29e)

### Notes
- No schema change: V12 is still the current version, and the fix is entirely in how the graph is queried. Knowledge written before this release becomes visible as soon as the new binary runs — nothing needs re-importing.
- The MCP server is a long-lived process. After upgrading, restart the client (Claude Code) before expecting `memory_status` or `memory_add` to behave differently.
- The Markdown snapshot is unchanged in shape and remains the default; `--json` is additive and conflicts only with `--hook`, which prints its own SessionStart envelope.

---

## [v1.10.0] — 2026-08-12

### Added
- **A seven-layer memory snapshot, injected at session start.** `build_snapshot` renders a frozen Markdown slice of the graph under a hard character budget per layer (~4.5K in total), read-only and instant. `consolidate` distils a project into a single `Digest` node — the next steps recorded by recent sessions plus the problems still unsolved — idempotently, so running it twice changes nothing. Two node types anchor the ends of the range: `UserFact` for what is known about the owner (layer 1) and `Digest` for the distillate (layer 7). Exposed as the `memory_snapshot` and `memory_consolidate` MCP tools and as `au snapshot [--project] [--hook]`, wired to Claude Code's SessionStart hook, which injects the slice straight into the context. A failing hook is swallowed: memory has no right to break the start of a session (b3ed9d4, 61837ce)
- **Bit-i-Delo, stage 1 — an append-only journal of what the agent actually did.** Schema V9 adds `act_trace`, a write-ahead log of actions mirrored into FTS5, alongside the `probes`, `pathways`, `labile_window`, `trace_attribution` and `corrections` tables the later stages consume. `trace.rs` ingests a trace with a payload ceiling, SHA-256 hashes of the file state before and after, and a strict enum of kinds — an unknown kind is the caller's error, not a new typo in the journal. Append-only is enforced by database triggers and pinned by tests: the history is not edited after the fact. `au trace` takes a trace from the command line or, with `--hook`, from Claude Code's PostToolUse JSON on stdin. The architecture behind all seven stages is written up as spec `003-bit-i-delo` (809fa5f)
- **Stage 2 — claims checked against ground truth.** `probes.rs` extracts verifiable statements from a node's text deterministically — file paths and git SHAs — and executes them against reality: the file system and `git cat-file`. `memory_add` records the outcome and returns `probe_warnings`. Advisory for now: a failed probe warns but does not yet change what is stored (66e268c)
- **Stage 3 — a surprise gate, recall as a transaction, and an outcome judge that calls no model.** `codec.rs` scores new text by normalised compression distance against zstd dictionaries trained per scope, so restating what is already expected costs almost nothing and reads as almost no news. `window.rs` turns recall into a transaction: a query signature, labile windows, path locking, and corrections served first; `memory_search` accepts a `session_id` and opens a window. `differ.rs` is a pure `judge(traces) -> Verdict` over `reinforce` / `erode` / `fork` / `null` — the verdict is computed, not asked; the reconsolidator writes revisions into `node_version`, and an `erode` debits the path and mints a correction. Wired as `au judge [--hook]` on the Stop hook. Schema V10 adds `codec`, `delta` and `node_version` (3a44c5c)
- **Stage 4 — clearing, obligations and bankruptcy, closing the loop.** `ledger.rs` clears the session ledger: a yield bonus for windows that reinforced, a penalty for what was rendered and never used, and `node_value_bits` as the single currency ranking is done in. `bankrupt_and_absorb` is garbage collection by insolvency — a node that has earned nothing is absorbed by its strongest neighbour and hands over its edges, reversibly through `node_version`; `memory_gc` triggers it. `obligations.rs` adds the prospective contour: a promise is taken in when a commissive marker appears, deduplicated by the trace that produced it, and settled only by a later event sharing at least two significant words with it — one shared word is usually the project name and would settle the wrong debt. Tension grows with age and with how often that counterparty has broken promises before, and the snapshot gained a «Давление» (pressure) layer to show it. Schema V11 (b6cb6b7)

### Fixed
- **An obligation is born from speech, not from a shell command.** The pressure layer was showing the owner lines like «aurelius blyss force foreach item local path remove silentlycontinue». Two defects sat behind it. Intake was fed the raw payload of a trace, and `au trace -m "надо потом добить клиринг"` mentions a promise without being one; the marker was also matched as a substring, so `todo` was found inside `TodoWrite`. A speech gate now rejects text that is too long or too dense in punctuation, and the marker is matched on word boundaries — the gate is deliberately one-sided, since a missed obligation is cheaper than an invented one. Separately, what was displayed was `object_fp`, the alphabetically sorted bag of tokens used for deduplication and search, so even a correct extraction read as noise; the readable sentence is now stored beside the fingerprint and the snapshot shows that instead. Migration V12 adds the column and re-checks every obligation already on record against its original text in `act_trace`: what fails the gate should never have existed and is deleted (2df74f3)

### Notes
- Schema V9 through V12 all land here. Migrations run in a single transaction and are applied on first open.
- The snapshot now has eight sections; «Давление» sits third, between what is in progress and the recent sessions.
- The three Claude Code hooks close a loop: `au snapshot` on SessionStart feeds the context, `au trace` on PostToolUse records what happened, `au judge` on Stop settles it.

---

## [v1.9.0] — 2026-08-08

### Added
- **Documents into Markdown, converted locally.** Word, PowerPoint, Excel, OpenDocument, RTF, EPUB, CSV, PDF, HTML and plain text, via the `anydoc` and `htmd` crates — in-process, with no network call, no API key and no external binary. Three MCP tools (`doc_convert`, `doc_read`, `doc_recall`) plus `au doc convert` / `au doc recall`. Output over `max_inline_chars` spills to a `.md` file and returns an outline and preview instead, so a 200-page PDF cannot fill an agent's context in one call. Conversions are cached by the SHA-256 of the file contents and full-text indexed, which is the point: a contract read in July stays findable in September after the original file is gone. Audio, video and scanned images are refused by name with the reason — transcription and OCR need external services and are deliberately out of scope (8a022b2)
- **Sync — one graph across machines and people.** New `aurelius-sync-server` crate, the `au share` command family, and automatic push/pull at session boundaries. Every node and edge carries who created and last updated it; deletions propagate as tombstones rather than reappearing on the next pull; conflicting edits keep the losing version on the node instead of discarding it, surfaced by `au context --verbose`. Collaborator access is granted per project by an issued token and can be revoked (92872ba, f2cc833, 36c5382, d92ff92, d8f7bbc, 2f033b0)
- `au home use` / `current` / `reset` — a persisted active profile, so a chosen home survives the shell that set it (6e83cd8)
- `au share admin-set` — stores this machine's admin token per server, so `issue` and `revoke` no longer need `AURELIUS_SYNC_ADMIN_TOKEN` exported every session (3c99ff9)
- Identity falls back to `git config --global` when unset, so attribution works before anyone configures it (858af64)

### Fixed
- **A push could write outside its project.** The server now enforces the granted project scope on every pushed node and edge, instead of trusting the client's own labelling (91440a5)
- `aurelius-sync-server` binds to loopback only by default. Exposing it needs a deliberate choice, not an oversight (6a3f661)
- `au share issue --for` defaults to the local identity instead of failing when omitted (6ffe192)
- `au share admin-set` shows its `<server> <token>` usage in `--help` (a80f0d9)

### Documentation
- Spec-kit features `003-doc-to-markdown` and the project-sync set — specification, plan, research, data model, contracts, quickstart and task list (8a022b2, be9f6f3, 4bdcbd7, a18b124)

### Notes
- **Sync ships for the first time here.** It landed across PRs #5–#12 after the v1.8.0 tag, so this release is the first one that contains it.
- Schema V8 adds the document cache and its FTS mirror; V6 and V7, which sync introduced, are also first released here. Migrations run in a single transaction and are applied on first open.

---

## [v1.8.0] — 2026-07-29

### Added
- `au db check [PATH]` — the check now takes an optional path, so a snapshot can be verified. A snapshot is an ordinary database, so verifying one is the same command pointed at a different file (a22d373)
- The rolling backup hook verifies every snapshot it writes. An unverified backup is a guess. A snapshot that fails the check is renamed to `.FAILED-CHECK` — out of the `aurelius-*.db` pattern, so it can never be mistaken for a good backup nor counted by retention — and kept rather than deleted, because a bad snapshot is evidence worth reading (a22d373)

### Fixed
- `au db backup` reports a missing database instead of failing inside the snapshot call (a22d373)

---

## [v1.7.0] — 2026-07-29

### Fixed
- **A damaged database is refused instead of silently rewritten.** Every `db::open` now checks the file's own 100-byte header against its size and refuses an image whose header describes less than the file holds — the exact fingerprint of a file-level copy over a live WAL database. The error names the file, the finding, the two commands to run, and the rule that caused the damage, instead of the bare `database disk image is malformed` (d781fe2)
- **A failed read is no longer mistaken for an empty database.** `get_schema_version(...).unwrap_or(0)` turned `SQLITE_BUSY` and `SQLITE_CORRUPT` into "brand-new database" and re-ran the destructive migration chain over live data on every single invocation. Version reads now propagate their errors; zero means only that the `schema_version` table is absent (d781fe2)
- **Migrations are atomic.** The whole chain runs in one `BEGIN IMMEDIATE` transaction with the version re-read inside it, so a failure mid-`migrate_v4` can no longer leave the FTS index dropped, its triggers gone and the version advanced. Concurrent processes block instead of racing — 8 simultaneous opens of a fresh database used to fail with `UNIQUE constraint failed: schema_version.version` (d781fe2)
- **Concurrent access waits instead of failing.** Every connection sets `busy_timeout` before anything can take a lock — a hook spawns a writer on every file edit, and several MCP servers run at once, so contention is the norm rather than the exception (d781fe2)
- **WAL mode is verified, not assumed.** `PRAGMA journal_mode` reports a refused switch as a result row rather than an error, and `execute_batch` discarded that row — a connection could silently run in rollback-journal mode. The mode is now read back and checked, with bounded retries for the brief exclusive lock a fresh database needs (d781fe2)
- Databases written by a newer binary are refused instead of being written to under an older understanding of the schema (d781fe2)
- Failed edge writes propagate instead of being discarded at 20 call sites; `touch_node` and `ensure_indexed` stay best-effort — an access counter must never fail a read — but are logged rather than silently dropped (d781fe2)
- `migrate_v2` detects existing columns structurally via `pragma_table_info` instead of matching the English text of an error message (d781fe2)
- The database path had three divergent definitions that disagreed on their fallback; on a machine without a data directory the CLI and the MCP server would have used different files. Now resolved in one place (d781fe2)

### Added
- `au db backup [--out PATH]` — safe snapshot of a live database via SQLite's own `VACUUM INTO`, including data still sitting in an un-checkpointed `-wal`. **Copying `aurelius.db` with `cp`/`mv`/`rsync` while `au` or an MCP server is running is what corrupts it** — use this instead (d781fe2)
- `au db check [--full]` — read-only integrity report that never migrates and never writes a page. Exits non-zero when damaged, so it can gate a script or a hook (d781fe2)
- Skills subsystem — 4 MCP tools (`skill_save`, `skill_list`, `skill_get`, `skill_remove`), `au skills`, and session auto-injection via a SessionStart hook. Released as v1.6.0 but never merged to the default branch; folded in here (78dad01)
- First automated tests in the workspace: concurrent open, migration rollback, corruption refusal, newer-schema refusal, backup round-trip through an un-checkpointed WAL, fresh-open idempotence. Each was observed failing against the previous code (d781fe2)

### Documentation
- Spec-kit feature `002-db-durability-hardening` — specification, plan, research, data model, CLI contract, quickstart, task list — and the project constitution the plan is gated against (8d799ce)
- README: backup section with the reason file-level copying destroys a WAL database, and the manual restore procedure (d781fe2)

### Notes
- **v1.6.0 is contained in this release.** Its tag pointed at a commit that never reached the default branch, so the repository read as 1.5.0 while the installed binary reported 1.6.0. This release supersedes it.
- Verified against the preserved database from the 2026-07-27 incident: 22 consecutive operations, zero bytes changed, and both the refusal and the report name the file-level-copy signature in plain words.

---

## [v1.6.0] — 2026-06-21

### Added
- **Skills subsystem** — reusable procedural "how-to" cards with progressive disclosure. 4 MCP tools:
  - `skill_save` — create/update a skill (upsert by name). Trigger → FTS-indexed note (discoverable); body + tags → `data` (not keyword-indexed, so the body never pollutes search).
  - `skill_list` — cheap index (name + trigger + tags + uses), ranked by usage. Optional FTS `query` / `tag` filter.
  - `skill_get` — full markdown body by name, with fuzzy FTS fallback + `other_matches` for disambiguation. Bumps `access_count`.
  - `skill_remove` — delete a skill by name.
- New node type `Skill`.
- Skills surface automatically in `memory_recall` (new `skills` bucket) and `memory_status` (top skills by usage).
- **SessionStart hook** (`aurelius-skills.sh` / `au skills --hook`) injects the skill index into context every session via `hookSpecificOutput.additionalContext`.

### Changed
- CLI: new `au skills [--hook]` command.

---

## [v1.5.0] — 2026-04-19

### Added
- `memory_merge` — merge two duplicate/related nodes into one: rewires all edges from source to target, removes self-loops and duplicate edges, appends source's note to target, deletes source. CLI: `au merge <source> <target>` (909b06d)
- `task_stats` — analytics over tasks: counts by status/priority, completion rate, avg/median active→done duration, currently blocked count, oldest active age, done-in-window. CLI: `au task stats [--project] [--since-days]` (909b06d)

### Documentation
- Update tool count 19→21 in README and CLAUDE.md (238792c)

---

## [v1.4.1] — 2026-04-01

### Added
- Auto-index project on first use — no manual `au init`/`au reindex` needed (73afe86)

### Documentation
- Update README to v1.4.0 — task management, 19 tools, new sections (32da52e)

---

## [v1.4.0] — 2026-04-01

### Added
- **Task management system** — 5 MCP tools (`task_create`, `task_update`, `task_list`, `task_log`, `task_view`) + full CLI (`au task`) (6857f8f)
- New node types: `Task`, `WorkLog`; new relations: `SubtaskOf`, `Blocks`
- Tasks as hub nodes: collect work logs, decisions, problems, solutions via `contains` edges
- Acceptance criteria, priority-based sorting, auto-activation on first log
- `memory_status` shows active tasks; `memory_session` accepts `tasks` param and returns active hints
- `memory_recall` includes tasks in grouped results

### Other
- Verify post-commit hook links to project (31b55e5)
- Clean test commits (6ea8c12)

---

## [1.3.0] — 2026-03-28

### Fixed
- **`memory_session` auto-creates project nodes** — sessions now create their project hub node if it doesn't exist, and link all child nodes (decisions, problems, solutions) to it via `belongs_to` edges. Previously, sessions silently skipped project linking when the project node was missing, leaving the graph fragmented.
- **Project filter includes hub node** — sidebar project filter now includes the project node itself, keeping the graph connected when filtering by project.

### Improved
- **Obsidian-style graph physics** — reworked force simulation: no node pinning after drag (nodes release back into simulation), gentle center force, stronger link forces for cluster cohesion. Drag a node and its neighbors follow naturally.
- **Cleaner graph labels** — only project nodes show labels by default; other nodes reveal labels on hover/select with neighbor highlighting.
- **Softer link styling** — links are subtle by default, brighten on highlight (Obsidian-inspired).
- **Smaller node sizes** — reduced node radii for cleaner visualization at scale.
- **Project navigation in sidebar** — new "Projects" section extracts project names from `[project-name]` label prefix, allows one-click project scoping.

### Removed
- Position persistence (localStorage pinning) — graph recalculates layout each session, matching Obsidian behavior.

---

## [1.0.0] — 2026-03-21

### Added
- **`memory_gc`** — garbage collection: removes duplicate edges, orphaned edges, and duplicate nodes (by content_hash)
- **`memory_status` project filter** — optional `project` parameter to scope decisions, problems, sessions to a specific project
- **`memory_search` since filter** — optional `since` parameter for time-based queries (`today`, `yesterday`, `7d`, `24h`, ISO 8601)
- **Batch BFS** — context traversal uses batch queries (`WHERE id IN (...)`) instead of N+1 per-node queries
- **Relevance-ranked search** — FTS results boosted by `access_count` for frequently accessed nodes
- **V3 migration** — composite indexes: `edges(to_id, relation)`, unique `edges(from_id, to_id, relation)`, `nodes(content_hash)`, `nodes(node_type, created_at)`
- **V4 migration** — rebuilt FTS5 index without `data` column to eliminate JSON key noise in search results
- **Edge deduplication** — `INSERT OR IGNORE` prevents duplicate edges on same `(from_id, to_id, relation)` triple

### Refactored
- **`graph.rs`** (531 lines) → `graph/{crud, search, traverse}.rs` — modular graph operations
- **`handlers.rs`** (594 lines) → `handlers/{crud, session, status}.rs` — modular MCP handlers

### Fixed
- Project-scoped `memory_status` now uses `search_typed` for proper SQL-level type+FTS filtering
- Project-scoped `open_problems` uses `get_unsolved_problems` with label prefix filter
- FTS5 bracket escaping for `[project]` prefix queries
- V3 migration cleans duplicate edges before creating UNIQUE index

### Removed
- Dead code: unused `get_edges` single-node query (replaced by batch version)
- TimeForged sync — evaluated and rejected (time data not useful for AI memory)

---

## [0.5.0] — 2026-03-21

### Optimized
- **`memory_status`** — uses SQL LIMIT instead of fetching all nodes and truncating in Rust; 6x fewer rows deserialized
- **`get_unsolved_problems`** — parameterized node types (no hardcoded JSON strings), added LIMIT
- **`memory_session`** — deduplication via SHA-256 content_hash; duplicate calls return existing session instead of creating duplicates
- **`memory_session`** — removed double storage: decisions/problems no longer stored in Session node's `data` JSON (they're already separate graph nodes)

### Added
- `find_node_by_content_hash()` — lookup nodes by content hash for dedup

---

## [0.4.0] — 2026-03-21

### Added
- **`memory_recall`** — smart topic recall: combines FTS search with BFS traversal, returns results grouped by type (decisions, problems, solutions, sessions, other). One call instead of separate search+context
- **`memory_search` type filter** — optional `type` parameter to filter results by node type (e.g. `type: "decision"`)
- **`get_unsolved_problems()`** — SQL query that finds problems without a linked solution (via `solves` edge)
- **`search_typed()`** — FTS search with node type filter in core

### Improved
- **`memory_status`** — `open_problems` now shows only unsolved problems (those without a `solves` edge from a Solution node), not all problems
- **Web UI** — graph physics now always active (`cooldownTicks=Infinity`, `d3AlphaMin=0`), Obsidian-like behavior

### Fixed
- Graph visualization froze after 5-10 seconds due to d3-force simulation cooling down

---

## [0.3.0] — 2026-03-21

### Added
- **`memory_session`** — record session summaries with decisions, problems solved, and next steps; creates episodic Session node linked to project, plus Decision and Problem/Solution nodes with proper graph relations
- **`memory_update`** — update existing node's note and/or data by UUID or label; enables enriching nodes with additional context after creation
- **`memory_add` enhanced** — now accepts `data` (arbitrary JSON metadata) and `memory_kind` (semantic/episodic) parameters

### Improved
- **`memory_status`** — now returns recent solutions alongside problems, session details with full node info (not just brief), and uses lightweight count queries for stats
- **`memory_add`** — uses `add_node_full` internally, supporting all node fields

---

## [0.2.0] — 2026-03-21

### Improved
- **`memory_search`** — empty query (`""`) or wildcard (`"*"`) now returns most recent nodes instead of FTS5 error
- **`memory_dump`** — added pagination with `offset` and `limit` parameters (default: 50 items) to prevent exceeding MCP token limits; response includes `total_nodes`/`total_edges` counts for navigation

### Added
- `get_recent_nodes()` — fetch N most recent nodes by creation date
- `get_nodes_paginated()` / `get_edges_paginated()` — paginated graph queries
- `count_nodes()` / `count_edges()` — lightweight count queries

---

## [0.1.0] — 2026-03-21

### Added
- **Knowledge Graph Core** — SQLite-backed graph with FTS5 full-text search, WAL mode, versioned migrations
- **Domain Model** — 14 node types (Project, Crate, File, Decision, Concept, Problem, Solution, etc.), 16 relation types, MemoryKind (Semantic/Episodic)
- **Graph Operations** — add/delete/update nodes, BFS traversal, FTS search, touch (access tracking), find by label/data field
- **Project Indexer** — parses Cargo.toml workspaces, discovers crates, files, dependencies; SHA256 content hashing for incremental re-index
- **TimeForged Connector** — async integration with TimeForged time tracking daemon; pulls sessions, projects, languages into the graph
- **MCP Server** — JSON-RPC 2.0 over stdio, 8 tools: `memory_status`, `memory_context`, `memory_search`, `memory_add`, `memory_relate`, `memory_index`, `memory_forget`, `memory_dump`
- **CLI (`au`)** — 9 subcommands: `init`, `note`, `context`, `search`, `sync`, `reindex`, `view`, `export`, `mcp`, `touch`
- **Web UI** — React + TypeScript + Tailwind CSS + react-force-graph-2d; interactive graph visualization with Obsidian-style physics, sidebar filters, node detail panel, search
- **Claude Code Integration** — MCP server config, PostToolUse hook (tracks file access), Stop hook (auto re-index on session end), git post-commit hook (captures decisions)
- **Install script** — `install.sh` for one-command setup: build, install, configure hooks
