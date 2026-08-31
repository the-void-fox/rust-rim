//! Запуск RimWorld: поиск установки, префиксов и сборка команды.

pub mod launch;
pub mod paths;

pub use launch::{LaunchSettings, Mode, Plan, Runner};
pub use paths::{Executable, Prefix};
