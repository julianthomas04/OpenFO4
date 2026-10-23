//! One loaded movie document and the typed queries on it.

use super::document::{
    CharacterDocument, InstanceDocument, MovieDocument, SpriteDocument, TextDocument,
};
use super::error::UiLayoutError;
use super::values::{Align, MovieStage, Placement, Scalar, Shape, TextField, Timeline};

const NEARBY_LIMIT: usize = 5;

const HTML_LEADING: &str = "TEXTFORMAT.LEADING";
const HTML_LEFT_MARGIN: &str = "TEXTFORMAT.LEFTMARGIN";
const HTML_RIGHT_MARGIN: &str = "TEXTFORMAT.RIGHTMARGIN";
const HTML_INDENT: &str = "TEXTFORMAT.INDENT";
const HTML_LETTER_SPACING: &str = "FONT.LETTERSPACING";
const HTML_ALIGN: &str = "P.ALIGN";

pub struct Movie {
    name: String,
    document: MovieDocument,
}

impl Movie {
    pub fn new(name: &str, document: MovieDocument) -> Self {
        Self {
            name: name.to_owned(),
            document,
        }
    }

    fn missing(&self, key: &str, kind: &'static str, nearby: Vec<String>) -> UiLayoutError {
        UiLayoutError::MissingKey {
            movie: self.name.clone(),
            key: key.to_owned(),
            kind,
            nearby,
        }
    }

    fn unresolved(&self, key: &str, reason: impl Into<String>) -> UiLayoutError {
        UiLayoutError::Unresolved {
            movie: self.name.clone(),
            key: key.to_owned(),
            reason: reason.into(),
        }
    }

    fn bad_format(&self, reason: String) -> UiLayoutError {
        UiLayoutError::BadFormat {
            movie: self.name.clone(),
            reason,
        }
    }

    fn wrong_kind(&self, key: &str, expected: &'static str, actual: &str) -> UiLayoutError {
        UiLayoutError::WrongKind {
            movie: self.name.clone(),
            key: key.to_owned(),
            expected,
            actual: actual.to_owned(),
        }
    }

    fn instance(&self, path: &str) -> Result<&InstanceDocument, UiLayoutError> {
        if let Some(instance) = self.document.instances.get(path) {
            return Ok(instance);
        }
        if let Some(reason) = self.document.unresolved.get(path) {
            return Err(self.unresolved(path, reason));
        }
        let parent = path.rfind('/').map_or("", |slash| &path[..=slash]);
        let nearby = nearest(self.document.instances.keys(), |key| {
            key.starts_with(parent) && !key[parent.len()..].contains('/')
        });
        Err(self.missing(path, "instance", nearby))
    }

    fn character(&self, key: &str, id: u32) -> Result<&CharacterDocument, UiLayoutError> {
        self.document
            .characters
            .get(&id.to_string())
            .ok_or_else(|| {
                self.bad_format(format!(
                    "'{key}' shows character {id}, which the document does not define"
                ))
            })
    }

    fn placement_of(instance: &InstanceDocument) -> Placement {
        Placement {
            depth: instance.depth,
            name: instance.name.clone(),
            frames: instance.frames,
            local: instance.local.clone(),
            world: instance.world.clone(),
            track: instance.track.clone(),
        }
    }

    pub fn placement(&self, path: &str) -> Result<Placement, UiLayoutError> {
        self.instance(path).map(Self::placement_of)
    }

    pub fn text_field(&self, path: &str) -> Result<TextField, UiLayoutError> {
        let instance = self.instance(path)?;
        if instance.kind != "text" {
            return Err(self.wrong_kind(path, "a text field", &instance.kind));
        }
        let CharacterDocument::Text(text) = self.character(path, instance.char)? else {
            return Err(self.bad_format(format!(
                "'{path}' is a text instance of a character that is not text"
            )));
        };
        let placement = Self::placement_of(instance);
        if !placement.world.matrix.is_axis_aligned() {
            return Err(self.unresolved(path, "the text field is rotated or skewed"));
        }
        self.build_text_field(path, text, placement)
    }

    fn build_text_field(
        &self,
        path: &str,
        text: &TextDocument,
        placement: Placement,
    ) -> Result<TextField, UiLayoutError> {
        let font = text
            .font
            .clone()
            .ok_or_else(|| self.unresolved(path, "the text field names no font"))?;
        let size = text
            .size
            .ok_or_else(|| self.unresolved(path, "the text field has no font size"))?;
        let html = text
            .flags
            .html
            .then_some(text.html_format.as_ref())
            .flatten();
        let html_value = |name: &str| html.and_then(|map| map.get(name));
        let html_number = |name: &str| match html_value(name) {
            None => Ok(None),
            Some(Scalar::Number(value)) => Ok(Some(*value as f32)),
            Some(other) => Err(self.unresolved(
                path,
                format!(
                    "the html format's {name} is {}, not a number",
                    other.type_name()
                ),
            )),
        };
        let align = match html_value(HTML_ALIGN) {
            None => text.align.unwrap_or(Align::Left),
            Some(Scalar::Text(name)) => parse_align(name)
                .ok_or_else(|| self.unresolved(path, format!("unknown alignment '{name}'")))?,
            Some(other) => {
                return Err(self.unresolved(
                    path,
                    format!(
                        "the html format's {HTML_ALIGN} is {}, not text",
                        other.type_name()
                    ),
                ));
            }
        };
        let left_margin = html_number(HTML_LEFT_MARGIN)?;
        let right_margin = html_number(HTML_RIGHT_MARGIN)?;
        let indent = html_number(HTML_INDENT)?;
        let leading = html_number(HTML_LEADING)?;
        let letter_spacing = html_number(HTML_LETTER_SPACING)?;
        Ok(TextField {
            bounds: text.bounds,
            font,
            size,
            color: text.color,
            align,
            left_margin: left_margin.or(text.left_margin).unwrap_or(0.0),
            right_margin: right_margin.or(text.right_margin).unwrap_or(0.0),
            indent: indent.or(text.indent).unwrap_or(0.0),
            leading: leading.or(text.leading).unwrap_or(0.0),
            letter_spacing: letter_spacing.unwrap_or(0.0),
            multiline: text.flags.multiline,
            word_wrap: text.flags.word_wrap,
            auto_size: text.flags.auto_size,
            html: text.flags.html,
            initial_text: text.initial_text.clone(),
            placement,
        })
    }

    pub fn shape(&self, path: &str) -> Result<Shape, UiLayoutError> {
        let instance = self.instance(path)?;
        if instance.kind != "shape" {
            return Err(self.wrong_kind(path, "a shape", &instance.kind));
        }
        let CharacterDocument::Shape(shape) = self.character(path, instance.char)? else {
            return Err(self.bad_format(format!(
                "'{path}' is a shape instance of a character that is not a shape"
            )));
        };
        Ok(Shape {
            bounds: shape.bounds,
            edge_bounds: shape.edge_bounds.unwrap_or(shape.bounds),
            fills: shape.fills.clone(),
            lines: shape.lines.clone(),
            paths: shape.paths.clone(),
            placement: Self::placement_of(instance),
        })
    }

    pub fn instance_timeline(&self, path: &str) -> Result<Timeline, UiLayoutError> {
        let instance = self.instance(path)?;
        if instance.kind != "sprite" {
            return Err(self.wrong_kind(path, "a sprite", &instance.kind));
        }
        self.sprite_timeline(path, instance.char)
    }

    /// The class the sprite at `path` is exported under.
    pub fn instance_class(&self, path: &str) -> Result<String, UiLayoutError> {
        let instance = self.instance(path)?;
        if instance.kind != "sprite" {
            return Err(self.wrong_kind(path, "a sprite", &instance.kind));
        }
        match self.character(path, instance.char)? {
            CharacterDocument::Sprite(SpriteDocument {
                class: Some(class), ..
            }) => Ok(class.clone()),
            _ => Err(self.unresolved(path, "the sprite has no exported class")),
        }
    }

    pub fn symbol_timeline(&self, class: &str) -> Result<Timeline, UiLayoutError> {
        let Some(&id) = self.document.symbols.get(class) else {
            let nearby = nearest(self.document.symbols.keys(), |_| true);
            return Err(self.missing(class, "symbol", nearby));
        };
        self.sprite_timeline(class, id)
    }

    fn sprite_timeline(&self, key: &str, id: u32) -> Result<Timeline, UiLayoutError> {
        match self.character(key, id)? {
            CharacterDocument::Sprite(sprite) => Ok(Timeline {
                frame_count: sprite.frame_count,
                labels: sprite.labels.clone(),
            }),
            CharacterDocument::Shape(_) => Err(self.wrong_kind(key, "a sprite", "shape")),
            CharacterDocument::Text(_) => Err(self.wrong_kind(key, "a sprite", "text")),
            CharacterDocument::Other => Err(self.wrong_kind(key, "a sprite", "another character")),
        }
    }

    pub fn stage(&self) -> MovieStage {
        MovieStage {
            rect: self.document.stage,
            fps: self.document.fps,
        }
    }

    /// Every instance key equal to `root` or below it: `root/...` (children) and `root:...`
    /// (the frame and label views of its timeline), sorted.
    pub fn paths_under(&self, root: &str) -> Vec<String> {
        let below = |key: &str| {
            key.strip_prefix(root)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', ':']))
        };
        let mut paths: Vec<String> = self
            .document
            .instances
            .keys()
            .filter(|key| below(key))
            .cloned()
            .collect();
        paths.sort();
        paths
    }

    pub fn main_timeline(&self) -> Timeline {
        Timeline {
            frame_count: self.document.frame_count,
            labels: self.document.labels.clone(),
        }
    }

    /// `Class.member`, inherited from the nearest superclass that states it when the class does
    /// not.
    pub fn constant(&self, key: &str) -> Result<&Scalar, UiLayoutError> {
        let Some((class, member)) = key.rsplit_once('.') else {
            return self.own_constant(key);
        };
        let mut current = class;
        for _ in 0..=self.document.super_classes.len() {
            let qualified = format!("{current}.{member}");
            match self.own_constant(&qualified) {
                Err(UiLayoutError::MissingKey { .. }) => {}
                found => return found,
            }
            match self.document.super_classes.get(current) {
                Some(parent) => current = parent,
                None => break,
            }
        }
        self.own_constant(key)
    }

    fn own_constant(&self, key: &str) -> Result<&Scalar, UiLayoutError> {
        if let Some(value) = self.document.constants.get(key) {
            return Ok(value);
        }
        if let Some(reason) = self.document.unresolved.get(key) {
            return Err(self.unresolved(key, reason));
        }
        let class = key.rfind('.').map_or(key, |dot| &key[..=dot]);
        let nearby = nearest(self.document.constants.keys(), |other| {
            other.starts_with(class)
        });
        Err(self.missing(key, "constant", nearby))
    }

    /// The union of the stroke-inclusive bounds of every shape and text field at or below
    /// `path` on its frame-1 display list, in the space of the path's root: what Flash reports as
    /// the display object's bounds.
    pub fn subtree_bounds(&self, path: &str) -> Result<[f32; 4], UiLayoutError> {
        let below = format!("{path}/");
        let mut union: Option<[f32; 4]> = None;
        for (key, instance) in &self.document.instances {
            let inside = key == path
                || key
                    .strip_prefix(&below)
                    .is_some_and(|rest| !rest.contains(':'));
            if !inside {
                continue;
            }
            let local = match self.character(key, instance.char)? {
                CharacterDocument::Shape(shape) => shape.bounds,
                CharacterDocument::Text(text) => text.bounds,
                CharacterDocument::Sprite(_) | CharacterDocument::Other => continue,
            };
            let [x0, y0, x1, y1] = instance.world.matrix.bounds(local);
            union = Some(union.map_or([x0, y0, x1, y1], |[a, b, c, d]| {
                [a.min(x0), b.min(y0), c.max(x1), d.max(y1)]
            }));
        }
        union.ok_or_else(|| {
            let nearby = nearest(self.document.instances.keys(), |key| key.starts_with(path));
            self.missing(path, "display object with bounds", nearby)
        })
    }

    pub fn literals(&self, key: &str, expected: usize) -> Result<&[Scalar], UiLayoutError> {
        if let Some(list) = self.document.literals.get(key) {
            if list.values.len() != expected {
                return Err(UiLayoutError::CountMismatch {
                    movie: self.name.clone(),
                    key: key.to_owned(),
                    expected,
                    actual: list.values.len(),
                });
            }
            return Ok(&list.values);
        }
        if let Some(reason) = self.document.unresolved.get(key) {
            return Err(self.unresolved(key, reason));
        }
        let method = key.rfind('#').map_or(key, |hash| &key[..=hash]);
        let nearby = nearest(self.document.literals.keys(), |other| {
            other.starts_with(method)
        });
        Err(self.missing(key, "literal list", nearby))
    }

    pub fn wrong_scalar(
        &self,
        key: &str,
        expected: &'static str,
        actual: &Scalar,
    ) -> UiLayoutError {
        self.wrong_kind(key, expected, actual.type_name())
    }
}

fn parse_align(name: &str) -> Option<Align> {
    match name.to_ascii_lowercase().as_str() {
        "left" => Some(Align::Left),
        "right" => Some(Align::Right),
        "center" => Some(Align::Center),
        "justify" => Some(Align::Justify),
        _ => None,
    }
}

fn nearest<'a>(
    keys: impl Iterator<Item = &'a String>,
    close: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut found: Vec<String> = keys.filter(|key| close(key)).cloned().collect();
    found.sort();
    found.truncate(NEARBY_LIMIT);
    found
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::ui_layout::test_support::{
        assert_all_close, assert_close, instance, layout_with_movie, movie_json,
    };

    #[test]
    fn paths_under_a_root_include_children_and_frame_views_but_not_namesakes() {
        let parts = json!({
            "fps": 24.0,
            "stage": [0, 0, 640, 360],
            "instances": {
                "Clip": instance(1, "sprite", 0.0, 0.0),
                "Clip/@1": instance(2, "shape", 0.0, 0.0),
                "Clip:3/@3": instance(2, "shape", 0.0, 0.0),
                "Clip:3/@3/@1": instance(2, "shape", 0.0, 0.0),
                "ClipOther": instance(1, "sprite", 0.0, 0.0),
                "Other/Clip": instance(1, "sprite", 0.0, 0.0),
            },
        });
        let layout = layout_with_movie("Fixture", &movie_json("Fixture", &parts));
        let paths = layout.instance_paths("Fixture", "Clip").unwrap();
        assert_eq!(paths, ["Clip", "Clip/@1", "Clip:3/@3", "Clip:3/@3/@1"]);
        let movie = layout.movie("Fixture").unwrap();
        assert_close(movie.stage().fps, 24.0);
        assert_all_close(&movie.stage().rect, &[0.0, 0.0, 640.0, 360.0]);
    }
}
