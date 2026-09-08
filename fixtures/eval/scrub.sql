-- 010-waking-memory T001 — механическая часть обезличивания из quickstart.md §1.
-- Гоняется ТОЛЬКО по копии, снятой `au db backup` (VACUUM INTO, commands.rs:3252).
-- Живая база на запись не открывается ни на одном шаге (FR-025).
--
-- ЧЕГО ЗДЕСЬ НЕТ: трёх `UPDATE ... replace(...)` по nodes.label/note/data и по
-- act_trace.payload. Им нужен список подстановок владельца, а его в репозитории
-- нет; придуманные аргументы дали бы фикстуру, которая называется обезличенной и
-- ею не является. Триггеры act_trace сняты именно под этот будущий проход.

PRAGMA foreign_keys=ON;                   -- как в db.rs:111; edges.from_id/to_id — ON DELETE CASCADE

DROP TRIGGER IF EXISTS act_trace_ro;      -- append-only, RAISE(ABORT) на UPDATE
DROP TRIGGER IF EXISTS act_trace_nodel;   -- append-only, RAISE(ABORT) на DELETE

-- Координаты секретов: Config с data.kind = 'secret_ref' (secret.rs:55).
DELETE FROM nodes WHERE node_type = '"config"'
                    AND json_extract(data, '$.kind') = 'secret_ref';

-- Кэш веб-поиска: запросы владельца дословно. Триггер search_cache_ad
-- вычищает search_fts сам.
DELETE FROM search_cache;

-- У act_trace_fts триггер только AFTER INSERT — отсюда явная пересборка.
INSERT INTO act_trace_fts(act_trace_fts) VALUES('rebuild');

VACUUM;
