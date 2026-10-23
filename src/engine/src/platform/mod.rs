mod app;
mod boot_handoff;
pub mod fatal_error;
mod key_input;
pub mod logging;

use app::App;
use winit::event_loop::EventLoop;

use crate::rendering::debug_ui::ToastType;
use crate::rendering::fatal_runtime_error::FatalRuntimeError;
use crate::startup::StartupError;

pub fn run(startup_toasts: Vec<(String, ToastType)>) -> anyhow::Result<()> {
    let event_loop: EventLoop<()> = EventLoop::new()?;
    let mut app = App::new(startup_toasts)?;
    event_loop.run_app(&mut app)?;
    outcome(app.take_startup_error(), app.take_runtime_error())
}

/// The result of a run that ended on its own: a startup error or a fatal runtime error is
/// returned so `main` reports it to the user; neither means the user closed the window.
fn outcome(
    startup_error: Option<StartupError>,
    runtime_error: Option<FatalRuntimeError>,
) -> anyhow::Result<()> {
    match (startup_error, runtime_error) {
        (Some(error), _) => Err(error.into()),
        (None, Some(error)) => Err(error.into()),
        (None, None) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioError;
    use crate::rendering::device_loss::DeviceLost;

    #[test]
    fn a_run_without_errors_succeeds() {
        assert!(outcome(None, None).is_ok());
    }

    #[test]
    fn a_fatal_runtime_error_is_returned_so_main_reports_it() {
        let lost = DeviceLost {
            reason: wgpu::DeviceLostReason::Unknown,
            message: "driver reset".to_string(),
        };

        let error = outcome(None, Some(lost.into())).unwrap_err();

        assert!(format!("{error:#}").contains("driver reset"), "{error:#}");
    }

    #[test]
    fn a_fatal_audio_error_is_returned_so_main_reports_it() {
        let error = outcome(
            None,
            Some(AudioError::Device("no device".to_string()).into()),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("no device"), "{error:#}");
    }

    #[test]
    fn a_startup_error_takes_precedence() {
        let startup = StartupError::Device(anyhow::anyhow!("no gpu"));
        let runtime = AudioError::Device("no device".to_string()).into();

        let error = outcome(Some(startup), Some(runtime)).unwrap_err();

        assert!(format!("{error:#}").contains("no gpu"), "{error:#}");
    }
}
