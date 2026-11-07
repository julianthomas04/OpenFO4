//! How strongly the weather's lights (its Sunlight and directional ambient colours) light a
//! surface, against the fog and sky colours, which reach the picture as they are.
//!
//! The probe's material row (plain, river-bed ground, boulder and bridge-plank boards, each under
//! its vanilla material) gives every board's sun term per unit of Sunlight colour falling square
//! on it, in the fog colour's units. Over each material's diffuse texture albedo that is 0.64 for
//! the ground, 0.58 for the rock and 0.59 for the wood (the plain board, whose material adds a
//! metal cube map, 0.75). The ground ring around the four terrain boards, rendered against vanilla
//! side by side, matches at 0.57 to 0.64 at noon and 0.53 at night under the clear weather.

/// The light a Lambertian surface of albedo 1 returns per unit of weather light colour falling
/// square on it: the median of the real materials.
pub const WEATHER_LIGHT_RESPONSE: f32 = 0.59;
