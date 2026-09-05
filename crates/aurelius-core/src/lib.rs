// В юнит-тестах (`#[cfg(test)] mod tests` внутри модулей) unwrap/expect
// используются массово и намеренно — падение теста и есть сигнал. Гасим
// запрет только для сборки под `--cfg test`: обычная (нетестовая) сборка
// той же самой библиотеки — которую clippy тоже проверяет в рамках
// `--all-targets` — по-прежнему запрещает их на рантайм-путях.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod codec;
pub mod db;
pub mod differ;
pub mod fts;
pub mod graph;
pub mod home;
pub mod identity;
pub mod indexer;
pub mod ledger;
pub mod models;
pub mod obligations;
pub mod probes;
pub mod provenance;
pub mod secret;
pub mod sync;
pub mod tasks;
pub mod trace;
pub mod window;

pub use db::{db_path, CheckReport, DbError};
