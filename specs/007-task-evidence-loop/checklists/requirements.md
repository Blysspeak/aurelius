# Specification Quality Checklist: Круг «задача — работа — улика — закрытие»

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-08-30
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

- Проверка прошла с двумя итерациями правок.

  Первый прогон завалил «No implementation details»: в требованиях стояли имена функций и путей
  файлов, взятые из постановки (`gather()`, `snapshot.rs`, `precompact.mjs`, `note()`,
  `.ulika/artifacts/*.log`), а также числовые бюджеты в символах. Всё это переписано на язык
  наблюдаемого поведения: «подаваемая память», «система наблюдения за ходом», «артефакт прогона»,
  «гарантированный объём». Кодовые ориентиры не потеряны — им место в `plan.md`, а не в спеке.

  Второй прогон завалил «Requirements are testable»: FR по секретам говорил «не хранить секреты»
  без признака, по которому это проверяется. Добавлен FR-026 с отклонением похожего на значение
  ввода и SC-007 со счётом «ноль случаев».

- Восьмое требование постановки (ревизия команд) намеренно сформулировано критерием
  (FR-028…FR-030), а не списком имён. Список даёт отдельная разведка по фактическим вызовам; она
  станет входными данными для плана, а не для спеки.

- Требование о проектном охвате слоя «Владелец» в спеку не входит — вынесено в Out of Scope,
  закрывается параллельным прогоном.
