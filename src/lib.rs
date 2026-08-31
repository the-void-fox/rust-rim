// Библиотечный таргет: даёт доступ к модулям из tests/ и examples/.
// Бинарник (main.rs) использует эти же модули через `use rust_rim::...`.

pub mod app;
pub mod description;
pub mod fs_util;
pub mod job;
pub mod log_analysis;
pub mod mod_data;
pub mod settings;
pub mod sorting;
pub mod steam;
pub mod ui;
