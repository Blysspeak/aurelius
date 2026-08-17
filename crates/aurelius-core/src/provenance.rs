//! Происхождение факта: откуда он взялся, когда измерен и как быстро протухает.
//!
//! Память принимала утверждение о мире, не спрашивая, чем оно подтверждено.
//! Ложное «флаги выключены» ложилось ровно так же, как измеренное запросом, и
//! на выдаче они были неразличимы. Это перенос в инструмент правила, которое в
//! проектах уже написано кровью: число не произносится без команды, которой
//! получено, и времени замера.
//!
//! Поля живут в `data` узла, а не колонками — по той же причине, что и
//! `agent_session`: колонка потребовала бы вручную править десяток рукописных
//! списков `SELECT`, и пропущенный упал бы в рантайме на `row.get`, а не при
//! компиляции.
//!
//! Разбор здесь ровно один. CLI собирает из своих флагов тот же JSON, что
//! приходит по MCP, и зовёт [`Provenance::parse`] — одна запись, две двери.

use anyhow::{bail, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub const CLAIM_KEY: &str = "claim";
pub const EVIDENCE_KEY: &str = "evidence";
pub const MEASURED_AT_KEY: &str = "measured_at";
pub const CONFIDENCE_KEY: &str = "confidence";
pub const VOLATILITY_KEY: &str = "volatility";
pub const VERIFY_WITH_KEY: &str = "verify_with";
pub const SUBJECT_KEY: &str = "subject";

/// Потолок длины утверждения. Смысл `claim` в том, что он отдаётся ЦЕЛИКОМ и
/// никогда не режется на полуслове, — а «целиком» имеет право существовать
/// только пока оно ограничено. Длинное обоснование живёт в `note`.
pub const CLAIM_MAX_CHARS: usize = 240;

/// Чем подтверждено утверждение.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// Получено командой или запросом, дословно записанным в `evidence`.
    Measured,
    /// Выведено из измеренного, но само не измерялось.
    Inferred,
    /// Сказано человеком или документацией; не проверялось.
    Reported,
    /// Происхождение не названо. Значение по умолчанию: отсутствие поля
    /// означает «неизвестно», а не «наверное измерено».
    Unverified,
}

impl Confidence {
    pub const KNOWN: &'static [&'static str] = &["measured", "inferred", "reported", "unverified"];

    #[must_use]
    pub fn parse_known(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "measured" => Self::Measured,
            "inferred" => Self::Inferred,
            "reported" => Self::Reported,
            "unverified" => Self::Unverified,
            _ => return None,
        })
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Measured => "measured",
            Self::Inferred => "inferred",
            Self::Reported => "reported",
            Self::Unverified => "unverified",
        }
    }
}

/// Как быстро утверждение перестаёт быть правдой.
///
/// Деление `semantic`/`episodic` этого не ловит: «в .env стоит true» не событие
/// и не вечная истина — оно живёт до первой правки файла, не оставляющей следа
/// в git.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Volatility {
    /// Не меняется: адрес функции в собранном бинаре, идентификатор коммита.
    Immutable,
    /// Меняется редко и заметно: схема БД, состав зависимостей.
    Slow,
    /// Меняется тихо и в любой момент: содержимое `.env`, статус процесса.
    Volatile,
}

impl Volatility {
    pub const KNOWN: &'static [&'static str] = &["immutable", "slow", "volatile"];

    #[must_use]
    pub fn parse_known(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "immutable" => Self::Immutable,
            "slow" => Self::Slow,
            "volatile" => Self::Volatile,
            _ => return None,
        })
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Immutable => "immutable",
            Self::Slow => "slow",
            Self::Volatile => "volatile",
        }
    }

    /// Через сколько дней утверждение перестаёт считаться свежим.
    /// `None` у неизменного — устареть нечему.
    #[must_use]
    pub fn stale_after_days(self) -> Option<i64> {
        match self {
            Self::Immutable => None,
            Self::Slow => Some(30),
            Self::Volatile => Some(1),
        }
    }
}

/// Происхождение одного факта.
///
/// `volatility` намеренно `Option`: дефолт «slow» был бы тем же молчаливым
/// враньём, против которого весь этот модуль. Не сказали — не знаем, и приписки
/// про устаревание не будет.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Provenance {
    /// Короткое утверждение, которое отдаётся целиком и никогда не режется.
    pub claim: Option<String>,
    /// Команда или запрос ДОСЛОВНО — то, чем это получено.
    pub evidence: Option<String>,
    pub measured_at: Option<DateTime<Utc>>,
    pub confidence: Option<Confidence>,
    pub volatility: Option<Volatility>,
    /// Команда перепроверки. Без неё приписка «устарело» бесполезна: она
    /// сообщает о беде и не говорит, чем её закрыть.
    pub verify_with: Option<String>,
    /// Ключ предмета, о котором утверждение: `xhub:.env:REFUND_ENABLED`.
    /// Два факта с одним ключом не могут быть истинны одновременно.
    pub subject: Option<String>,
}

/// Строка непустая после обрезки краёв, иначе `None`. Пустое значение обязано
/// исчезнуть, а не лечь в граф: `""` в `subject` совпал бы с чужим `""`.
fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

impl Provenance {
    /// Разобрать из параметров вызова. Единственная дверь: и MCP, и CLI зовут
    /// её, поэтому проверки нельзя обойти, собрав структуру руками мимо неё.
    ///
    /// # Errors
    /// Неизвестное значение `confidence`/`volatility`, `claim` длиннее
    /// [`CLAIM_MAX_CHARS`], `measured_at` не по RFC 3339, а также `measured`
    /// без `evidence` — измеренное без команды, которой измерено, это `inferred`.
    pub fn parse(params: &serde_json::Value) -> Result<Self> {
        let text = |key: &str| non_empty(params.get(key).and_then(serde_json::Value::as_str));

        let confidence = match text(CONFIDENCE_KEY) {
            None => None,
            Some(raw) => Some(Confidence::parse_known(&raw).ok_or_else(|| {
                anyhow::anyhow!(
                    "неизвестный confidence '{raw}'. Известные: {}",
                    Confidence::KNOWN.join(", ")
                )
            })?),
        };

        let volatility = match text(VOLATILITY_KEY) {
            None => None,
            Some(raw) => Some(Volatility::parse_known(&raw).ok_or_else(|| {
                anyhow::anyhow!(
                    "неизвестная volatility '{raw}'. Известные: {}",
                    Volatility::KNOWN.join(", ")
                )
            })?),
        };

        let measured_at = match text(MEASURED_AT_KEY) {
            None => None,
            Some(raw) => Some(
                DateTime::parse_from_rfc3339(&raw)
                    .map_err(|e| {
                        anyhow::anyhow!("measured_at '{raw}' не разбирается как RFC 3339: {e}")
                    })?
                    .with_timezone(&Utc),
            ),
        };

        let claim = text(CLAIM_KEY);
        if let Some(c) = claim.as_deref() {
            let len = c.chars().count();
            if len > CLAIM_MAX_CHARS {
                bail!(
                    "claim длиной {len} символов при потолке {CLAIM_MAX_CHARS}: \
                     он отдаётся целиком и потому обязан быть коротким. \
                     Длинное обоснование — в note"
                );
            }
        }

        let evidence = text(EVIDENCE_KEY);
        if confidence == Some(Confidence::Measured) && evidence.is_none() {
            bail!(
                "confidence=measured без evidence. Измеренное без команды, \
                 которой измерено, — это inferred"
            );
        }

        let mut provenance = Self {
            claim,
            evidence,
            measured_at,
            confidence,
            volatility,
            verify_with: text(VERIFY_WITH_KEY),
            subject: text(SUBJECT_KEY),
        };

        // Замер без времени замера бесполезен: устарел он или нет — неизвестно.
        // Момент записи здесь и есть момент измерения; иное значение вызывающий
        // передаёт сам.
        if provenance.confidence == Some(Confidence::Measured) && provenance.measured_at.is_none() {
            provenance.measured_at = Some(Utc::now());
        }

        Ok(provenance)
    }

    /// Прочитать обратно из `data` узла. Снисходительно и без ошибок: чужие или
    /// испорченные значения читаются как «не сказано», потому что выдача не
    /// имеет права упасть из-за одной кривой записи.
    #[must_use]
    pub fn from_data(data: &serde_json::Value) -> Self {
        let text = |key: &str| non_empty(data.get(key).and_then(serde_json::Value::as_str));
        Self {
            claim: text(CLAIM_KEY),
            evidence: text(EVIDENCE_KEY),
            measured_at: text(MEASURED_AT_KEY)
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc)),
            confidence: text(CONFIDENCE_KEY).and_then(|s| Confidence::parse_known(&s)),
            volatility: text(VOLATILITY_KEY).and_then(|s| Volatility::parse_known(&s)),
            verify_with: text(VERIFY_WITH_KEY),
            subject: text(SUBJECT_KEY),
        }
    }

    /// Дописать поля в `data` узла, не трогая остальное.
    pub fn write_into(&self, data: &mut serde_json::Value) {
        if data.is_null() {
            *data = serde_json::json!({});
        }
        let Some(obj) = data.as_object_mut() else {
            return;
        };
        let mut put = |key: &str, value: Option<String>| {
            if let Some(v) = value {
                obj.insert(key.to_owned(), v.into());
            }
        };
        put(CLAIM_KEY, self.claim.clone());
        put(EVIDENCE_KEY, self.evidence.clone());
        put(MEASURED_AT_KEY, self.measured_at.map(|d| d.to_rfc3339()));
        put(
            CONFIDENCE_KEY,
            self.confidence.map(|c| c.as_str().to_owned()),
        );
        put(
            VOLATILITY_KEY,
            self.volatility.map(|v| v.as_str().to_owned()),
        );
        put(VERIFY_WITH_KEY, self.verify_with.clone());
        put(SUBJECT_KEY, self.subject.clone());
    }

    /// Как называть уверенность на выдаче. Отсутствие поля — это `unverified`,
    /// а не «наверное измерено».
    #[must_use]
    pub fn confidence_or_default(&self) -> Confidence {
        self.confidence.unwrap_or(Confidence::Unverified)
    }

    /// Пометка уверенности для выдачи. `None` у измеренного: помечать надо
    /// сомнительное, а не бесспорное, иначе пометка станет фоном.
    #[must_use]
    pub fn confidence_mark(&self) -> Option<&'static str> {
        match self.confidence_or_default() {
            Confidence::Measured => None,
            other => Some(other.as_str()),
        }
    }

    /// Сколько дней факту и чем его перепроверить, если он уже просрочен.
    ///
    /// `fallback_at` — время создания узла: им меряется возраст, когда замер не
    /// датирован. `None` означает «протухать нечему или ещё рано».
    #[must_use]
    pub fn staleness(&self, fallback_at: DateTime<Utc>, now: DateTime<Utc>) -> Option<Stale> {
        let limit = self.volatility?.stale_after_days()?;
        let measured = self.measured_at.unwrap_or(fallback_at);
        let age = now.signed_duration_since(measured);
        if age < Duration::days(limit) {
            return None;
        }
        Some(Stale {
            days: age.num_days(),
            verify_with: self.verify_with.clone(),
        })
    }
}

/// Как новое утверждение соотносится с уже записанным о том же предмете.
///
/// Отдельным типом и в ядре: разбор строки в двух местах однажды разъехался бы,
/// а одно из мест оказалось бы после создания узла — и отказ уже ничего бы не
/// предотвратил.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Старое больше не верно.
    Supersede,
    /// Старое верно, новое делает его точнее.
    Refine,
    /// Оба верны — сказано осознанно.
    Coexist,
}

impl Resolution {
    pub const KNOWN: &'static [&'static str] = &["supersede", "refine", "coexist"];

    #[must_use]
    pub fn parse_known(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "supersede" | "supersedes" => Self::Supersede,
            "refine" | "refines" => Self::Refine,
            "coexist" => Self::Coexist,
            _ => return None,
        })
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supersede => "supersede",
            Self::Refine => "refine",
            Self::Coexist => "coexist",
        }
    }

    /// Разобрать то, что пришло от вызывающего: пусто и пробелы — «не сказано»,
    /// незнакомое слово — ошибка со списком известных.
    ///
    /// Разбор общий для `memory_add` и `au note` намеренно: разъехавшись, две
    /// двери начали бы по-разному понимать, что такое разрешение противоречия.
    ///
    /// # Errors
    /// Непустая строка, не совпавшая ни с одним известным разрешением.
    pub fn parse_arg(raw: Option<&str>) -> Result<Option<Self>> {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None => Ok(None),
            Some(s) => Self::parse_known(s).map(Some).ok_or_else(|| {
                anyhow::anyhow!(
                    "неизвестное resolution '{s}'. Известные: {}",
                    Self::KNOWN.join(", ")
                )
            }),
        }
    }

    /// Ребро, которым разрешение записывается в граф. `None` у `coexist`:
    /// «оба верны» — это отсутствие отношения, а не отношение «никакое».
    #[must_use]
    pub fn relation(self) -> Option<crate::models::Relation> {
        match self {
            Self::Supersede => Some(crate::models::Relation::Supersedes),
            Self::Refine => Some(crate::models::Relation::Refines),
            Self::Coexist => None,
        }
    }
}

/// Сколько чужих утверждений об одном предмете показывать в отказе. Больше пяти
/// — это уже не разрешение противоречия, а разбор завала.
const CONFLICTS_SHOWN: usize = 5;

/// Проверить, не сказано ли уже что-то об этом же предмете.
///
/// Правило живёт в ядре, а не в MCP-хендлере, по той же причине, что и запись
/// сессии: иначе `au note --subject` и `memory_add` разойдутся в том, что
/// считается противоречием, — а разошлись бы они молча.
///
/// # Errors
/// Утверждение о предмете уже есть, а `resolution` не назван. Ничего при этом
/// не записывается: отказ обязан случиться ДО создания узла, иначе в графе
/// останется ровно тот второй факт, из-за которого отказ и произошёл.
pub fn guard_subject(
    conn: &rusqlite::Connection,
    subject: Option<&str>,
    resolution_given: bool,
) -> Result<Vec<crate::models::Node>> {
    let Some(subject) = subject else {
        return Ok(Vec::new());
    };
    let existing =
        crate::graph::find_nodes_by_data_field(conn, SUBJECT_KEY, subject, CONFLICTS_SHOWN)?;
    if existing.is_empty() || resolution_given {
        return Ok(existing);
    }

    let listed = existing
        .iter()
        .map(|n| {
            let p = Provenance::from_data(&n.data);
            format!(
                "  {} · {} · {} · {}",
                n.id,
                p.confidence_or_default().as_str(),
                n.created_at.format("%Y-%m-%d"),
                p.claim.as_deref().unwrap_or(&n.label)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    bail!(
        "о предмете '{subject}' уже сказано:\n{listed}\nНичего не записано. \
         Два утверждения об одном предмете не могут быть истинны разом — \
         повтори с resolution: supersede (старое больше не верно), \
         refine (старое верно, новое точнее) или coexist (оба верны, \
         сказано осознанно)"
    )
}

/// Просроченный факт: сколько ему дней и чем перепроверить.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stale {
    pub days: i64,
    pub verify_with: Option<String>,
}

impl Stale {
    /// Приписка к тексту факта на выдаче.
    #[must_use]
    pub fn note(&self) -> String {
        match &self.verify_with {
            Some(cmd) => format!("старше {} дн — перепроверь: {cmd}", self.days),
            None => format!("старше {} дн — перепроверь", self.days),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn measured_without_evidence_is_refused() {
        let err = Provenance::parse(&json!({ "confidence": "measured" }))
            .expect_err("измеренное без команды — не измеренное");
        assert!(format!("{err}").contains("inferred"), "{err}");
    }

    #[test]
    fn measured_gets_a_timestamp_even_when_none_was_given() {
        let p = Provenance::parse(&json!({
            "confidence": "measured",
            "evidence": "psql -c 'select flag from settings'",
        }))
        .expect("parse");
        assert!(
            p.measured_at.is_some(),
            "замер без времени замера не отличить от вчерашнего"
        );
    }

    #[test]
    fn an_unnamed_origin_reads_as_unverified_not_as_measured() {
        let p = Provenance::parse(&json!({})).expect("parse");
        assert_eq!(p.confidence, None);
        assert_eq!(p.confidence_or_default(), Confidence::Unverified);
        assert_eq!(p.confidence_mark(), Some("unverified"));
    }

    #[test]
    fn only_doubtful_confidence_is_marked() {
        let measured = Provenance {
            confidence: Some(Confidence::Measured),
            ..Provenance::default()
        };
        assert_eq!(measured.confidence_mark(), None);
    }

    #[test]
    fn unknown_values_are_refused_with_the_known_list() {
        let err = Provenance::parse(&json!({ "confidence": "точно" })).expect_err("не значение");
        assert!(format!("{err}").contains("measured"), "{err}");
        let err = Provenance::parse(&json!({ "volatility": "быстро" })).expect_err("не значение");
        assert!(format!("{err}").contains("immutable"), "{err}");
    }

    #[test]
    fn an_overlong_claim_is_refused_rather_than_clipped() {
        let long = "я".repeat(CLAIM_MAX_CHARS + 1);
        let err = Provenance::parse(&json!({ "claim": long })).expect_err("длинный claim");
        assert!(format!("{err}").contains("note"), "{err}");
        assert!(Provenance::parse(&json!({ "claim": "я".repeat(CLAIM_MAX_CHARS) })).is_ok());
    }

    #[test]
    fn volatile_facts_go_stale_in_a_day_and_immutable_never_do() {
        let now = Utc::now();
        let yesterday = now - Duration::days(2);

        let volatile = Provenance {
            volatility: Some(Volatility::Volatile),
            verify_with: Some("cat /home/xhub/app/.env".to_owned()),
            ..Provenance::default()
        };
        let stale = volatile.staleness(yesterday, now).expect("протух");
        assert_eq!(stale.days, 2);
        assert!(
            stale.note().contains("перепроверь: cat"),
            "{}",
            stale.note()
        );

        let immutable = Provenance {
            volatility: Some(Volatility::Immutable),
            ..Provenance::default()
        };
        assert_eq!(immutable.staleness(yesterday, now), None);

        let slow = Provenance {
            volatility: Some(Volatility::Slow),
            ..Provenance::default()
        };
        assert_eq!(
            slow.staleness(yesterday, now),
            None,
            "30 дней ещё не прошло"
        );
    }

    /// Не сказали про волатильность — не выдумываем. Приписки быть не должно.
    #[test]
    fn an_unknown_volatility_never_claims_staleness() {
        let now = Utc::now();
        let ancient = now - Duration::days(3650);
        assert_eq!(Provenance::default().staleness(ancient, now), None);
    }

    #[test]
    fn round_trips_through_node_data() {
        let parsed = Provenance::parse(&json!({
            "claim": "REFUND_REQUESTS_ENABLED=false",
            "evidence": "ssh xhub 'grep REFUND /home/xhub/app/.env'",
            "confidence": "measured",
            "volatility": "volatile",
            "verify_with": "ssh xhub 'grep REFUND /home/xhub/app/.env'",
            "subject": "xhub:.env:REFUND_REQUESTS_ENABLED",
        }))
        .expect("parse");

        let mut data = json!({ "existing": true });
        parsed.write_into(&mut data);
        assert_eq!(data["existing"], true, "чужие ключи не тронуты");

        let read_back = Provenance::from_data(&data);
        assert_eq!(read_back, parsed);
    }

    #[test]
    fn blank_values_disappear_instead_of_landing_in_the_graph() {
        let p = Provenance::parse(&json!({ "subject": "   ", "claim": "" })).expect("parse");
        assert_eq!(p.subject, None, "пустой subject совпал бы с чужим пустым");
        assert_eq!(p.claim, None);
    }
}
