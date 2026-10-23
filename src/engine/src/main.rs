#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use openfo4::platform::{fatal_error, logging};
use openfo4::rendering::debug_ui::ToastType;

fn main() -> anyhow::Result<()> {
    let install_dir = openfo4::assets::install_dir_from_environment().inspect_err(|error| {
        fatal_error::report("OpenFO4 cannot start", &format!("{error:#}"));
    })?;
    let (log_session, logging_warning) = logging::init(&install_dir);
    if let Some(path) = log_session.log_file() {
        tracing::info!("logging to {}", path.display());
    }
    let startup_toasts = logging_warning
        .map(|warning| (warning.to_string(), ToastType::Error))
        .into_iter()
        .collect();
    let result = openfo4::run(startup_toasts);
    if let Err(error) = &result {
        tracing::error!("fatal: {error:?}");
        fatal_error::report("OpenFO4 stopped", &format!("{error:#}"));
    }
    drop(log_session);
    result
}
