//! Every weather a climate can pick, loaded from the database into typed definitions. The values
//! shared with the simulation and audio live in [`crate::weather`]; what is left here is what
//! only the renderer reads. The loader reads the database once, nothing here touches the GPU.

pub mod clouds;
pub mod data;
pub mod definition;
pub mod error;
pub mod library;
pub mod precipitation;

pub use clouds::{CLOUD_LAYER_COUNT, CLOUD_TEXTURE_COUNT, CloudLayer, CloudSlotValues, Clouds};
pub use definition::WeatherDef;
pub use error::WeatherLibraryError;
pub use library::WeatherLibrary;
pub use precipitation::PrecipitationParams;
