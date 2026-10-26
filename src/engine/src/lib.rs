// The Windows process heap serializes under multithreaded allocation: with
// the loading-mode worker pool, every streaming stage (terrain build, GLB
// parse, SQLite row decode) inflated linearly with worker count until the
// pool scaled not at all (measured 2026-06-12: ~11s of worker wall time at
// 8, 16, and 31 workers alike). mimalloc restores per-thread allocation.
#[global_allocator]
static GLOBAL_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod anim;
pub mod assets;
pub mod audio;
pub mod bethesda_space;
pub mod footsteps;
pub mod game_flow;
#[cfg(test)]
pub(crate) mod installer_source;
pub mod navmesh;
pub mod platform;
pub mod random;
pub mod rendering;
pub mod shared_cache;
pub mod sim;
pub mod spawn_cell;
pub mod startup;
pub mod ui_layout;
pub mod utils;
pub mod version;
pub mod video;
pub mod water_world;
pub mod weather;

pub use assets::types::mesh;
pub use platform::run;
