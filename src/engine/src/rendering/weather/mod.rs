//! Every weather a climate can pick, loaded from the database into typed definitions. The values
//! shared with the simulation and audio live in [`crate::weather`]; what is left here is what
//! only the renderer reads. The loader reads the database once; the weather frame blends the
//! weathers of the weather cycle every frame. Nothing here touches the GPU.

pub mod clouds;
pub mod data;
pub mod definition;
pub mod error;
pub mod frame;
pub mod library;
pub mod panel;
pub mod precipitation;
pub mod rain;
pub mod sounds;
pub mod startup;
pub mod sync;

pub use clouds::{CLOUD_LAYER_COUNT, CLOUD_TEXTURE_COUNT, CloudLayer, CloudSlotValues, Clouds};
pub use definition::WeatherDef;
pub use error::WeatherLibraryError;
pub use frame::{PrecipitationFrame, WeatherFrame, WeatherFrameError, WindFrame};
pub use library::WeatherLibrary;
pub use precipitation::PrecipitationParams;
pub use startup::{StartupWeather, StartupWeatherError};
pub use sync::{WeatherStep, WeatherSync};
