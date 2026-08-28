# Specification Quality Checklist: Наряд и смена

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-08-28
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (languages, frameworks, APIs)
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (no implementation details)
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] No implementation details leak into specification

## Notes

**Первый прогон валидации, 2026-08-28.**

Исправлено по ходу написания: вся техническая механика (запросы к хранилищу, имена хуков и файлов, устройство полей) вынесена в [research.md](../research.md) и в спеку не попала. Спека говорит, что система обязана делать, research — как именно и почему выбран этот путь.

**Второй прогон, 2026-08-28.** Оба маркера [NEEDS CLARIFICATION] сняты решениями владельца, записаны в раздел Decisions спеки:

1. **Потолок расхода** — не задаётся. Владелец предупреждён о риске застревания одной задачи в цикле проб; риск покрывается ограничением времени на задачу и остановкой после трёх подряд неудач. Пересмотр — по фактическим числам первого ночного прогона.
2. **Смешанные задачи** — откладываются человеку целиком, автоматическое расщепление отклонено.

Все пункты чеклиста закрыты. Спека готова к `/speckit-plan`.

**Третий прогон, 2026-08-28 — после `/speckit-clarify`.** Задано четыре вопроса из квоты в пять, все закрыты решениями владельца и внесены в раздел Clarifications:

1. **Разметка автоматическая**, человек вердикты не утверждает. Следствие внесено требованиями FR-003 и FR-003a: раз ошибку разметки некому поймать до прогона, вердикт обязан нести обоснование, и это единственный способ увидеть её утром.
2. **Линия работы на каждый наряд.** Переписаны FR-027 и FR-028, уточнён SC-009, добавлен пограничный случай о конфликте двух нарядов над одним файлом.
3. **Сводка не уходит наружу** — ни содержанием, ни фактом завершения. Уточнён FR-023.
4. **Некодовых задач в машинном пуле не бывает.** Добавлен FR-002a: отчёт исполнителя не заменяет проверочное действие.

Пятый вопрос — конкретные числа потолков — не задавался: закрыт разумным значением по умолчанию в разделе Assumptions (двадцать задач, восемь часов на прогон, сорок минут на задачу) с пометкой, что числа уточняются по первой ночи.

Требований стало 31, критериев успеха 11. Все пункты чеклиста остаются закрытыми. Спека готова к `/speckit-plan`.

**Отдельно зафиксировано в спеке как условие допуска:** история 4 (ограничения автономной работы) имеет приоритет P1 наравне с первой, хотя стоит четвёртой по порядку. Без неё прогон нельзя запускать ни разу — это не улучшение, а допуск.
