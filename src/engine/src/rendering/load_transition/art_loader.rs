//! Reads a loading screen's model on a worker thread, so the frame thread keeps drawing the
//! black screen and the logo meanwhile.

use std::io;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::rendering::loading_screen::{
    CpuArt, LoadingArt, LoadingArtError, ModelControls, StageError,
};

#[derive(Debug, thiserror::Error)]
pub(in crate::rendering) enum ArtLoadError {
    #[error(transparent)]
    Art(#[from] LoadingArtError),
    #[error(transparent)]
    Stage(#[from] StageError),
    #[error("the loading worker could not be started: {0}")]
    Spawn(#[source] io::Error),
    #[error("the loading worker stopped without a result")]
    WorkerLost,
}

pub(super) enum LoaderPoll {
    /// Nothing is being loaded.
    Idle,
    Pending,
    Done(Box<Result<CpuArt, ArtLoadError>>),
}

#[derive(Default)]
pub(super) struct ArtLoader {
    receiver: Option<Receiver<Result<CpuArt, LoadingArtError>>>,
}

impl ArtLoader {
    /// Starts reading `art`'s model; a load still running is forgotten, its result discarded.
    pub fn start(&mut self, art: LoadingArt, controls: ModelControls) -> Result<(), ArtLoadError> {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("loading-art".into())
            .spawn(move || {
                if sender.send(CpuArt::load(&art, controls)).is_err() {
                    tracing::debug!(
                        "a loading screen model finished after it was no longer wanted"
                    );
                }
            })
            .map_err(ArtLoadError::Spawn)?;
        self.receiver = Some(receiver);
        Ok(())
    }

    pub fn poll(&mut self) -> LoaderPoll {
        let Some(receiver) = &self.receiver else {
            return LoaderPoll::Idle;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result.map_err(ArtLoadError::from),
            Err(TryRecvError::Empty) => return LoaderPoll::Pending,
            Err(TryRecvError::Disconnected) => Err(ArtLoadError::WorkerLost),
        };
        self.receiver = None;
        LoaderPoll::Done(Box::new(result))
    }

    /// Forgets the load in progress.
    pub fn cancel(&mut self) {
        self.receiver = None;
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::rendering::loading_screen::test_support::cube_art;

    const WAIT: Duration = Duration::from_secs(20);

    fn finished(loader: &mut ArtLoader) -> Result<CpuArt, ArtLoadError> {
        let deadline = Instant::now() + WAIT;
        loop {
            match loader.poll() {
                LoaderPoll::Done(result) => return *result,
                LoaderPoll::Idle => panic!("nothing is being loaded"),
                LoaderPoll::Pending => {
                    assert!(Instant::now() < deadline, "the load did not finish");
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }

    fn missing_model(dir: &std::path::Path) -> LoadingArt {
        let mut art = cube_art(dir, "");
        art.model_path = dir.join("no_such_model.glb");
        art
    }

    #[test]
    fn a_loader_without_a_load_is_idle() {
        assert!(matches!(ArtLoader::default().poll(), LoaderPoll::Idle));
    }

    #[test]
    fn a_model_that_exists_arrives_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut loader = ArtLoader::default();
        loader
            .start(cube_art(dir.path(), ""), ModelControls::VANILLA)
            .unwrap();
        assert!(finished(&mut loader).is_ok());
        assert!(matches!(loader.poll(), LoaderPoll::Idle), "delivered once");
    }

    #[test]
    fn a_missing_model_arrives_as_the_error_that_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut loader = ArtLoader::default();
        loader
            .start(missing_model(dir.path()), ModelControls::VANILLA)
            .unwrap();
        let error = finished(&mut loader).err().expect("the load fails");
        assert!(
            matches!(
                error,
                ArtLoadError::Art(LoadingArtError::ModelMissing { .. })
            ),
            "{error}"
        );
        assert!(error.to_string().contains("no_such_model.glb"));
    }

    #[test]
    fn starting_again_discards_the_load_before() {
        let dir = tempfile::tempdir().unwrap();
        let mut loader = ArtLoader::default();
        loader
            .start(missing_model(dir.path()), ModelControls::VANILLA)
            .unwrap();
        loader
            .start(cube_art(dir.path(), ""), ModelControls::VANILLA)
            .unwrap();
        assert!(finished(&mut loader).is_ok());
    }

    #[test]
    fn a_cancelled_load_is_idle_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut loader = ArtLoader::default();
        loader
            .start(cube_art(dir.path(), ""), ModelControls::VANILLA)
            .unwrap();
        loader.cancel();
        assert!(matches!(loader.poll(), LoaderPoll::Idle));
    }
}
