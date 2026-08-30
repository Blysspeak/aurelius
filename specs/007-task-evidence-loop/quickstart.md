# Phase 1: как прогнать круг руками

Проверка, ради которой всё затевалось: завести задачу, поработать, ничего не сказать про закрытие
— и получить предъявление от машины. Ниже последовательность, где каждый шаг наблюдаем.

## Круг целиком

```bash
au task new "Починить обрезку записей в дампе" --project aurelius
```

```bash
au task activate <id>
```

Проверить, что прежняя активная задача вытеснена и это сказано вслух:

```bash
au task list --project aurelius --status active
```

Внести правку в любой файл кода — хук `au trace --hook` сработает сам. Затем прогнать проверку
через обвязку, чтобы улика записалась:

```bash
node A:/workSpace/ulika/scripts/verify-run.mjs cargo test --workspace
```

Теперь ничего не говорить про закрытие и запросить созревшие:

```bash
au task ripe --project aurelius
```

Ожидаемо: задача предъявлена, показаны улика, её время и перечень изменённого. Закрыть:

```bash
au task done <id> --commit $(git rev-parse --short HEAD)
```

```bash
au task view <id>
```

Ожидаемо: три времени заполнены, способ решения записан, `confirmed` истинно.

## Отрицательные проверки

Улика старше правки не должна давать созревания — прогнать проверку, затем внести правку, затем
запросить созревшие: список пуст.

Красная улика не должна давать созревания:

```bash
node A:/workSpace/ulika/scripts/verify-run.mjs cargo test --workspace -- --nonexistent-filter-xyz
```

Отказ от закрытия не должен повторяться до новой работы: отклонить предложение, запросить
созревшие снова — список пуст, пока не появится новая правка.

## Секреты

```bash
au secret add --name STRIPE_SECRET_KEY --where "1password://Private/Stripe/api-key" --project boostix
```

```bash
au secret list --project boostix
```

Проверка отказа — попытка положить в координату похожее на само значение:

```bash
au secret add --name TEST_KEY --where "sk-proj-abc123def456ghi789jkl012mno345" --project aurelius
```

Ожидаемо: код возврата 1 и объяснение, какой признак сработал. Записи не появилось.

## Дамп

```bash
au snapshot --project aurelius
```

Ожидаемо: активная задача присутствует целиком, ни одна запись не обрывается посреди слова.

## Снимок после сжатия

Довести сессию до сжатия контекста, затем:

```bash
au snapshot --project aurelius
```

Ожидаемо: снимок незакрытой работы стоит в разделе последних сессий, а не среди решений и знаний.

## Гейты

```bash
node A:/workSpace/ulika/scripts/verify-run.mjs cargo fmt --check
```

```bash
node A:/workSpace/ulika/scripts/verify-run.mjs cargo clippy --workspace --all-targets -- -D warnings
```

```bash
node A:/workSpace/ulika/scripts/verify-run.mjs cargo test --workspace
```

Плюс прогон настоящего бинаря против живой базы для всякой фазы, которая меняет чтение или запись
задач — принцип V конституции.
