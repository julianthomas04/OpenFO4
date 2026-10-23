use std::path::Path;

use super::{StartupError, StartupTimer};
use crate::rendering::boot::{BootParts, StartupInputs, StateInputs};
use crate::rendering::{State, device, game_settings::GameSettings, shaders};
use crate::spawn_cell;

const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<State>();
};

/// The startup worker's whole job: everything that does not have to run on the event-loop thread.
/// A `DebugUi` is created here from the window; winit allows that off the main thread on Windows.
pub fn run_startup(inputs: StartupInputs, parts: BootParts) -> Result<State, StartupError> {
    let mut timer = StartupTimer::start(inputs.cancel.clone());
    let asset_server = &inputs.asset_server;
    let asset_root = Path::new(asset_server.asset_location());

    let settings = GameSettings::load(asset_root)?;
    timer.mark("settings")?;
    shaders::startup::initialize(
        asset_root,
        &shaders::composed::all(),
        device::REQUIRED_FEATURES,
    )?;
    timer.mark("shaders")?;
    let spawn_cell = spawn_cell::from_env(&asset_server.db_path())?;
    timer.mark("spawn_cell")?;

    let state = State::build(
        parts,
        StateInputs {
            startup: inputs,
            settings,
            spawn_cell,
        },
        &mut timer,
    )?;
    timer.log_summary();
    Ok(state)
}
