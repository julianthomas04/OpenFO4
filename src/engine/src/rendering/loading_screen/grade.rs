//! How the large loading screen's picture is graded: by the image space the default object
//! manager names for loading screens (`LSIS`) and the colour LUT the `sLoadScreenLUT` setting
//! names, over a picture lifted by a small neutral amount.

use crate::assets::db::DbClient;
use crate::assets::ini::{GameIni, IniError, IniKey};
use crate::assets::layout::named_texture_key;
use crate::rendering::lighting::exterior_weather::{ExteriorWeatherError, load_image_space};
use crate::rendering::lighting::tonemap_settings::ImageSpaceCinematic;

const LOAD_SCREEN_LUT: IniKey = IniKey::new("Display", "sLoadScreenLUT");

/// The value of `sLoadScreenLUT` when no game INI sets it, as in vanilla.
const VANILLA_LOAD_SCREEN_LUT: &str = r"Textures\Effects\LUTs\LoadingMenuColorLUT.dds";

/// The default object slot of the loading screens' image space.
const LOAD_SCREEN_IMAGE_SPACE_SLOT: &str = "LSIS";

/// The folder the `sLoadScreenLUT` path starts in, which texture keys leave out.
const TEXTURES_PREFIX: &str = "textures\\";

/// The game's data folder, which a path may start with.
const DATA_PREFIX: &str = "data\\";

/// Inferred, not read from any game data: a neutral amount of linear light added to the whole
/// picture before it is graded. Without it a vanilla screenshot's background is matched about
/// four times worse (rms 3.9 against 1.5 of 255), with it the fitted lift is the same in every
/// channel (0.0035, 0.0037, 0.0034); about one step of 255 once encoded.
pub const INFERRED_LIFT: f32 = 0.0035;

/// The texture key of the colour LUT loading screens are graded with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadScreenLut {
    pub key: String,
}

impl LoadScreenLut {
    /// `sLoadScreenLUT`, or its vanilla value with a note in the log when no INI sets it.
    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        let path = ini
            .optional_valid::<String>(LOAD_SCREEN_LUT, "a texture path", |path| {
                !path.trim().is_empty()
            })?
            .unwrap_or_else(|| {
                tracing::info!(
                    "{} is not set in any game INI; loading screens take the vanilla {VANILLA_LOAD_SCREEN_LUT}",
                    LOAD_SCREEN_LUT.name
                );
                VANILLA_LOAD_SCREEN_LUT.to_owned()
            });
        Ok(Self::of_path(&path))
    }

    /// The key of a path that starts in the game's data folder (`Textures\...`), with or
    /// without a leading `Data\` or separator. A path without a `Textures\` folder is taken
    /// to be below it already.
    fn of_path(path: &str) -> Self {
        let path = path.trim().replace('/', "\\");
        let path = path.trim_start_matches('\\');
        let below_data = strip_prefix_ignoring_case(path, DATA_PREFIX).unwrap_or(path);
        let below_textures =
            strip_prefix_ignoring_case(below_data, TEXTURES_PREFIX).unwrap_or(below_data);
        Self {
            key: named_texture_key(below_textures),
        }
    }
}

fn strip_prefix_ignoring_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

#[derive(Debug, thiserror::Error)]
pub enum GradeDataError {
    #[error("the database cannot be opened: {0}")]
    Database(#[source] anyhow::Error),
    #[error("the database query for the {LOAD_SCREEN_IMAGE_SPACE_SLOT} default object failed: {0}")]
    Query(#[from] rusqlite::Error),
    #[error("no image space is assigned to the {LOAD_SCREEN_IMAGE_SPACE_SLOT} default object")]
    Unassigned,
    #[error(transparent)]
    ImageSpace(#[from] ExteriorWeatherError),
}

/// The cinematic stage of the loading screens' image space in the database at `db_path`.
pub fn load_screen_cinematic(db_path: &str) -> Result<ImageSpaceCinematic, GradeDataError> {
    let db = DbClient::new(db_path).map_err(GradeDataError::Database)?;
    cinematic_in(&db)
}

fn cinematic_in(db: &DbClient) -> Result<ImageSpaceCinematic, GradeDataError> {
    let image_space = db
        .get_default_object(LOAD_SCREEN_IMAGE_SPACE_SLOT)?
        .ok_or(GradeDataError::Unassigned)?;
    Ok(load_image_space(db, &image_space)?.cinematic)
}

/// What the colour pass applies besides the LUT.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradeLook {
    pub cinematic: ImageSpaceCinematic,
    pub lift: f32,
}

/// The uniform block of `loading_grade.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct GradeParams {
    look: [f32; 4],
    cinematic: [f32; 4],
    tint: [f32; 4],
}

impl GradeParams {
    pub fn new(look: &GradeLook, fade: f32, target_decodes_srgb: bool) -> Self {
        let cinematic = &look.cinematic;
        Self {
            look: [
                look.lift,
                fade.clamp(0.0, 1.0),
                if target_decodes_srgb { 1.0 } else { 0.0 },
                0.0,
            ],
            cinematic: [
                cinematic.saturation,
                cinematic.brightness,
                cinematic.contrast,
                cinematic.tint_amount,
            ],
            tint: cinematic.tint_color.extend(0.0).to_array(),
        }
    }
}

/// The colour the pass writes, encoded, for a linear `picture` colour, `lut` grading an encoded
/// colour: the reference the shader is tested against. Brightness, contrast and tint are left
/// out; the loading screens' image space leaves them neutral.
#[cfg(test)]
pub fn graded(
    picture: glam::Vec3,
    look: &GradeLook,
    lut: impl Fn(glam::Vec3) -> glam::Vec3,
) -> glam::Vec3 {
    use glam::Vec3;
    const LUMA: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);
    let lifted = picture + Vec3::splat(look.lift);
    let luma = Vec3::splat(lifted.dot(LUMA));
    let saturated = luma + (lifted - luma) * look.cinematic.saturation;
    let encode = |linear: f32| {
        let linear = linear.clamp(0.0, 1.0);
        if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        }
    };
    lut(Vec3::from(saturated.to_array().map(encode)))
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the expected values are copied, not computed"
)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::rendering::lighting::cell_lighting::srgb_unit_to_linear;

    fn ini(text: &str) -> GameIni {
        GameIni::from_texts([text])
    }

    #[test]
    fn without_the_setting_the_vanilla_lut_is_graded_with() {
        let lut = LoadScreenLut::from_ini(&ini("")).unwrap();
        assert_eq!(lut.key, r"textures\effects\luts\loadingmenucolorlut.dds");
    }

    #[test]
    fn the_setting_names_the_lut_from_the_data_folder() {
        let lut = LoadScreenLut::from_ini(&ini(
            "[Display]\nsLoadScreenLUT=Textures/Effects/LUTs/Mine.DDS\n",
        ))
        .unwrap();
        assert_eq!(lut.key, r"textures\effects\luts\mine.dds");
    }

    #[test]
    fn a_path_names_the_same_lut_with_or_without_the_data_folder_or_a_leading_separator() {
        let expected = r"textures\effects\luts\mine.dds";
        for path in [
            r"Textures\Effects\LUTs\Mine.dds",
            r"Data\Textures\Effects\LUTs\Mine.dds",
            r"data/textures/effects/luts/mine.dds",
            r"\Textures\Effects\LUTs\Mine.dds",
            "/Data/Textures/Effects/LUTs/Mine.dds",
            r"  Textures\Effects\LUTs\Mine.dds ",
        ] {
            assert_eq!(LoadScreenLut::of_path(path).key, expected, "{path}");
        }
    }

    #[test]
    fn a_path_without_a_textures_folder_is_taken_to_be_below_it() {
        assert_eq!(
            LoadScreenLut::of_path(r"Effects\LUTs\Mine.dds").key,
            r"textures\effects\luts\mine.dds"
        );
    }

    #[test]
    fn a_textures_folder_deeper_in_the_path_is_kept() {
        assert_eq!(
            LoadScreenLut::of_path(r"Effects\Textures\Mine.dds").key,
            r"textures\effects\textures\mine.dds"
        );
    }

    #[test]
    fn a_blank_setting_is_rejected() {
        assert!(matches!(
            LoadScreenLut::from_ini(&ini("[Display]\nsLoadScreenLUT= \n")),
            Err(IniError::Invalid { .. })
        ));
    }

    const LOAD_SCREEN_IS: &str = r#"{"Hdr":{"EyeAdaptSpeed":10,"TonemapE":7,"BloomThreshold":0.1,"BloomScale":0.08,"AutoExposureMax":2,"AutoExposureMin":1,"SunlightScale":1,"SkyScale":1,"MiddleGray":0},"Cinematic":{"Saturation":0.9,"Brightness":1,"Contrast":1},"Tint":{"Amount":0,"Color":{"R":254,"G":247,"B":203}},"Lut":"","DepthOfField":{"Strength":0,"Distance":0,"Range":0,"Mode":2,"VignetteRadius":0,"VignetteStrength":0}}"#;

    fn database(slot_target: Option<&str>, image_space: Option<&str>) -> DbClient {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE default_object_slots (manager TEXT NOT NULL, slot TEXT NOT NULL, target TEXT, PRIMARY KEY (manager, slot));
             CREATE TABLE image_spaces (form_key TEXT PRIMARY KEY, editor_id TEXT, data TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO default_object_slots VALUES ('000800:Fallout4.esm', 'LSIS', ?)",
            [slot_target],
        )
        .unwrap();
        if let Some(data) = image_space {
            conn.execute(
                "INSERT INTO image_spaces VALUES ('0FF9DB:Fallout4.esm', 'LoadScreenIS', ?)",
                [data],
            )
            .unwrap();
        }
        DbClient::from_connection(conn).unwrap()
    }

    #[test]
    fn the_cinematic_stage_is_the_load_screen_image_spaces() {
        let db = database(Some("0FF9DB:Fallout4.esm"), Some(LOAD_SCREEN_IS));
        let cinematic = cinematic_in(&db).unwrap();
        assert_eq!(cinematic.saturation, 0.9);
        assert_eq!((cinematic.brightness, cinematic.contrast), (1.0, 1.0));
        assert_eq!(cinematic.tint_amount, 0.0);
    }

    #[test]
    fn an_unassigned_slot_or_an_unknown_image_space_is_an_error() {
        let unassigned = database(None, Some(LOAD_SCREEN_IS));
        assert!(matches!(
            cinematic_in(&unassigned),
            Err(GradeDataError::Unassigned)
        ));
        let unknown = database(Some("0FF9DB:Fallout4.esm"), None);
        assert!(matches!(
            cinematic_in(&unknown),
            Err(GradeDataError::ImageSpace(
                ExteriorWeatherError::ImageSpaceNotFound { .. }
            ))
        ));
    }

    #[test]
    fn the_uniform_carries_the_lift_the_fade_the_target_and_the_cinematic_stage() {
        let look = GradeLook {
            cinematic: ImageSpaceCinematic {
                saturation: 0.9,
                ..ImageSpaceCinematic::NEUTRAL
            },
            lift: INFERRED_LIFT,
        };
        let params = GradeParams::new(&look, 3.0, true);
        assert_eq!(params.look, [INFERRED_LIFT, 1.0, 1.0, 0.0]);
        assert_eq!(params.cinematic, [0.9, 1.0, 1.0, 0.0]);
        assert_eq!(GradeParams::new(&look, -1.0, false).look[1..3], [0.0, 0.0]);
    }

    #[test]
    fn saturation_pulls_a_colour_towards_its_luma_and_spares_grey() {
        let look = GradeLook {
            cinematic: ImageSpaceCinematic {
                saturation: 0.5,
                ..ImageSpaceCinematic::NEUTRAL
            },
            lift: 0.0,
        };
        let decoded =
            |display: glam::Vec3| glam::Vec3::from(display.to_array().map(srgb_unit_to_linear));

        let grey = graded(glam::Vec3::splat(0.2), &look, decoded);
        assert!(
            (grey - glam::Vec3::splat(0.2)).abs().max_element() < 1e-4,
            "{grey:?}"
        );
        let red = graded(glam::Vec3::new(0.5, 0.0, 0.0), &look, decoded);
        let expected = glam::Vec3::new(0.303_15, 0.053_15, 0.053_15);
        assert!((red - expected).abs().max_element() < 1e-4, "{red:?}");
    }

    #[test]
    fn the_lift_is_added_to_the_linear_picture_before_it_is_encoded() {
        let look = GradeLook {
            cinematic: ImageSpaceCinematic::NEUTRAL,
            lift: INFERRED_LIFT,
        };
        let black = graded(glam::Vec3::ZERO, &look, |display| display);
        for channel in black.to_array() {
            assert!(
                (srgb_unit_to_linear(channel) - INFERRED_LIFT).abs() < 1e-6,
                "{black:?}"
            );
        }
    }
}
