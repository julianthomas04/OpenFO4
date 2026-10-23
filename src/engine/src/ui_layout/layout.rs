//! The loaded UI layout: translate table, font configuration and the movie documents of
//! `<install>/assets/interface/layout/`.
//!
//! Movies load lazily. The translate table and the font configuration are small and needed by
//! nearly every consumer, so they load when the layout opens. A movie document can be large and
//! only the movies a consumer declares keys for are ever read: the first access parses the file
//! and caches it, and [`UiLayout::validate`] forces that for every declared key at boot, so a
//! missing or malformed document still stops startup.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::document::{
    self, FONT_CONFIG_KIND, FontConfigDocument, FontFace, FontMap, MOVIE_KIND, MovieDocument,
    TRANSLATE_KIND, TranslateDocument,
};
use super::error::UiLayoutError;
use super::keys::{Key, KeyCheck, KeyKind, LiteralKey, NumberKey};
use super::movie::Movie;
use super::scaleform_text::FontMetrics;
use super::values::Scalar;

pub const TRANSLATE_DOCUMENT: &str = "translate";
pub const FONT_CONFIG_DOCUMENT: &str = "fontconfig";

const NORMAL_STYLE: &str = "Normal";

pub struct UiLayout {
    dir: PathBuf,
    translate: TranslateDocument,
    fonts: FontConfigDocument,
    movies: Mutex<HashMap<String, Arc<Movie>>>,
}

impl UiLayout {
    /// Opens the layout folder: reads the translate table and the font configuration.
    pub fn open(dir: &Path) -> Result<Self, UiLayoutError> {
        let translate = read_document(dir, TRANSLATE_DOCUMENT, TRANSLATE_KIND, "translate.json")?;
        let fonts = read_document(
            dir,
            FONT_CONFIG_DOCUMENT,
            FONT_CONFIG_KIND,
            "fontconfig.json",
        )?;
        Ok(Self {
            dir: dir.to_path_buf(),
            translate,
            fonts,
            movies: Mutex::new(HashMap::new()),
        })
    }

    /// A layout over in-memory documents, for tests: `movies` are `(name, json)` pairs that
    /// would otherwise be read lazily from disk.
    #[cfg(test)]
    pub fn from_documents(
        translate: &str,
        fonts: &str,
        movies: &[(&str, &str)],
    ) -> Result<Self, UiLayoutError> {
        let layout = Self {
            dir: PathBuf::from("<memory>"),
            translate: document::parse(TRANSLATE_DOCUMENT, TRANSLATE_KIND, translate)?,
            fonts: document::parse(FONT_CONFIG_DOCUMENT, FONT_CONFIG_KIND, fonts)?,
            movies: Mutex::new(HashMap::new()),
        };
        for (name, json) in movies {
            let document: MovieDocument = document::parse(name, MOVIE_KIND, json)?;
            layout
                .movies
                .lock()
                .unwrap()
                .insert((*name).to_owned(), Arc::new(Movie::new(name, document)));
        }
        Ok(layout)
    }

    /// The movie `name` (its file path below the layout folder without `.json`), loading it on
    /// first use.
    pub fn movie(&self, name: &str) -> Result<Arc<Movie>, UiLayoutError> {
        if let Some(movie) = self.cached(name) {
            return Ok(movie);
        }
        let document: MovieDocument =
            read_document(&self.dir, name, MOVIE_KIND, &format!("{name}.json"))?;
        let movie = Arc::new(Movie::new(name, document));
        let mut movies = self
            .movies
            .lock()
            .expect("the movie cache is never poisoned");
        Ok(movies.entry(name.to_owned()).or_insert(movie).clone())
    }

    fn cached(&self, name: &str) -> Option<Arc<Movie>> {
        self.movies
            .lock()
            .expect("the movie cache is never poisoned")
            .get(name)
            .cloned()
    }

    /// The instance keys of `movie` at or below `root`; see [`Movie::paths_under`].
    pub fn instance_paths(&self, movie: &str, root: &str) -> Result<Vec<String>, UiLayoutError> {
        Ok(self.movie(movie)?.paths_under(root))
    }

    pub fn get<K: KeyKind>(&self, key: &Key<K>) -> Result<K::Output, UiLayoutError> {
        K::resolve(self, key)
    }

    /// The `index`-th value of a literal list whose length must be the key's count.
    pub fn lit_at(&self, key: &LiteralKey, index: usize) -> Result<Scalar, UiLayoutError> {
        let list = self.literal_list(key)?;
        let actual = list.len();
        list.into_iter()
            .nth(index)
            .ok_or_else(|| UiLayoutError::CountMismatch {
                movie: key.movie.to_owned(),
                key: key.path.to_owned(),
                expected: index + 1,
                actual,
            })
    }

    pub fn lit_number(&self, key: &LiteralKey, index: usize) -> Result<f64, UiLayoutError> {
        match self.lit_at(key, index)? {
            Scalar::Number(value) => Ok(value),
            other => Err(self
                .movie(key.movie)?
                .wrong_scalar(key.path, "a number", &other)),
        }
    }

    /// The `index`-th value of a literal list as a whole number from 0 to `u32::MAX`.
    pub fn lit_whole(&self, key: &LiteralKey, index: usize) -> Result<u32, UiLayoutError> {
        whole(self.lit_number(key, index)?, key.movie, key.path)
    }

    /// An `ActionScript` constant as a whole number from 0 to `u32::MAX`.
    pub fn whole_constant(&self, key: &NumberKey) -> Result<u32, UiLayoutError> {
        whole(self.get(key)?, key.movie, key.path)
    }

    pub fn lit_string(&self, key: &LiteralKey, index: usize) -> Result<String, UiLayoutError> {
        match self.lit_at(key, index)? {
            Scalar::Text(value) => Ok(value),
            other => Err(self
                .movie(key.movie)?
                .wrong_scalar(key.path, "a string", &other)),
        }
    }

    pub fn literal_list(&self, key: &LiteralKey) -> Result<Vec<Scalar>, UiLayoutError> {
        self.movie(key.movie)?
            .literals(key.path, key.count)
            .map(<[Scalar]>::to_vec)
    }

    pub fn translate_text(&self, key: &str) -> Result<String, UiLayoutError> {
        if let Some(text) = self.translate.entries.get(key) {
            return Ok(text.clone());
        }
        if let Some(reason) = self.translate.unresolved.get(key) {
            return Err(UiLayoutError::Unresolved {
                movie: TRANSLATE_DOCUMENT.to_owned(),
                key: key.to_owned(),
                reason: reason.clone(),
            });
        }
        Err(UiLayoutError::MissingKey {
            movie: TRANSLATE_DOCUMENT.to_owned(),
            key: key.to_owned(),
            kind: "translation",
            nearby: Vec::new(),
        })
    }

    /// The family a font class maps to: the only mapping of the class, else the `Normal` one.
    pub fn font_family(&self, class: &str) -> Result<String, UiLayoutError> {
        self.font_map(class).map(|map| map.family.clone())
    }

    /// The vertical metrics of the face a font class maps to, in font sizes.
    pub fn font_metrics(&self, class: &str) -> Result<FontMetrics, UiLayoutError> {
        let map = self.font_map(class)?;
        self.face_metrics(&map.family, Some(&map.style), class)
    }

    /// The vertical metrics, in font sizes, of the only face of `family` the font configuration
    /// describes, whatever font class maps to it. A family with several styles is an error.
    pub fn family_metrics(&self, family: &str) -> Result<FontMetrics, UiLayoutError> {
        self.face_metrics(family, None, family)
    }

    /// The face of `family` in `style`, or the family's only face when `style` is `None`.
    fn face_metrics(
        &self,
        family: &str,
        style: Option<&str>,
        key: &str,
    ) -> Result<FontMetrics, UiLayoutError> {
        let mut faces =
            self.fonts.fonts.iter().filter(|face| {
                face.family == family && style.is_none_or(|style| face.style == style)
            });
        let unresolved = |reason: String| UiLayoutError::Unresolved {
            movie: FONT_CONFIG_DOCUMENT.to_owned(),
            key: key.to_owned(),
            reason,
        };
        match (faces.next(), faces.next()) {
            (Some(face), None) => Self::metrics_of(face, key),
            (None, _) => Err(unresolved(match style {
                Some(style) => format!("no metrics for '{family}' ({style})"),
                None => format!("no metrics for '{family}'"),
            })),
            (Some(_), Some(_)) => Err(unresolved(format!(
                "'{family}' has several styles; the face is ambiguous without one"
            ))),
        }
    }

    fn metrics_of(face: &FontFace, key: &str) -> Result<FontMetrics, UiLayoutError> {
        if face.em_size <= 0.0 {
            return Err(UiLayoutError::Unresolved {
                movie: FONT_CONFIG_DOCUMENT.to_owned(),
                key: key.to_owned(),
                reason: format!("'{}' has no positive em size", face.family),
            });
        }
        Ok(FontMetrics {
            ascent: face.ascent / face.em_size,
            descent: face.descent / face.em_size,
        })
    }

    fn font_map(&self, class: &str) -> Result<&FontMap, UiLayoutError> {
        let mapped: Vec<_> = self
            .fonts
            .maps
            .iter()
            .filter(|map| map.class == class)
            .collect();
        match mapped.as_slice() {
            [] => Err(UiLayoutError::MissingKey {
                movie: FONT_CONFIG_DOCUMENT.to_owned(),
                key: class.to_owned(),
                kind: "font class",
                nearby: Vec::new(),
            }),
            [only] => Ok(only),
            several => several
                .iter()
                .find(|map| map.style == NORMAL_STYLE)
                .copied()
                .ok_or_else(|| UiLayoutError::Unresolved {
                    movie: FONT_CONFIG_DOCUMENT.to_owned(),
                    key: class.to_owned(),
                    reason: format!("mapped several times, none with style '{NORMAL_STYLE}'"),
                }),
        }
    }

    /// Resolves every declared key and returns one error listing all failures.
    pub fn validate(&self, checks: &[&dyn KeyCheck]) -> Result<(), UiLayoutError> {
        let mut failures: Vec<UiLayoutError> = Vec::new();
        let mut seen = HashSet::new();
        for failure in checks.iter().flat_map(|check| check.failures(self)) {
            if seen.insert(failure.to_string()) {
                failures.push(failure);
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(UiLayoutError::Invalid { failures })
        }
    }
}

fn whole(value: f64, movie: &str, key: &str) -> Result<u32, UiLayoutError> {
    if (0.0..=f64::from(u32::MAX)).contains(&value) && value.fract() == 0.0 {
        Ok(value as u32)
    } else {
        Err(UiLayoutError::unresolved(
            movie,
            key,
            format!("{value} is not a whole number from 0 to {}", u32::MAX),
        ))
    }
}

fn read_document<T: serde::de::DeserializeOwned>(
    dir: &Path,
    name: &str,
    kind: &str,
    file: &str,
) -> Result<T, UiLayoutError> {
    let path = dir.join(file);
    let json = std::fs::read_to_string(&path).map_err(|source| match source.kind() {
        std::io::ErrorKind::NotFound => UiLayoutError::MissingMovie {
            movie: name.to_owned(),
            path: path.clone(),
        },
        _ => UiLayoutError::Unreadable {
            movie: name.to_owned(),
            path: path.clone(),
            source,
        },
    })?;
    document::parse(name, kind, &json)
}
