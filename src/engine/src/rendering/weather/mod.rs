//! Every weather a climate can pick, loaded from the database into typed definitions. Pure
//! values: the loader reads the database once, nothing here touches the GPU.

pub mod clouds;
pub mod data;
pub mod definition;
pub mod error;
mod flag_set;
pub mod library;
pub mod precipitation;

pub use clouds::{CLOUD_LAYER_COUNT, CLOUD_TEXTURE_COUNT, CloudLayer, CloudSlotValues, Clouds};
pub use data::{
    FadeRange, LightningParams, SoundKind, WeatherFlag, WeatherFlags, WeatherSound, WindParams,
};
pub use definition::WeatherDef;
pub use error::WeatherLibraryError;
pub use library::{ClimateEntry, WeatherId, WeatherLibrary};
pub use precipitation::PrecipitationParams;
