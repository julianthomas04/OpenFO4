//! Typed keys. A consumer declares each value it needs as a `const` key; the key's kind fixes what
//! the lookup returns, so a text field cannot be read as a shape and a number not as a string.

use std::marker::PhantomData;

use super::error::UiLayoutError;
use super::layout::{FONT_CONFIG_DOCUMENT, TRANSLATE_DOCUMENT, UiLayout};
use super::values::{MovieStage, Placement, Scalar, Shape, TextField, Timeline};

pub trait KeyKind {
    type Output;
    const DECLARED: DeclaredKind;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<Self::Output, UiLayoutError>
    where
        Self: Sized;
}

pub struct Key<K> {
    pub movie: &'static str,
    pub path: &'static str,
    kind: PhantomData<fn() -> K>,
}

impl<K> Key<K> {
    pub const fn new(movie: &'static str, path: &'static str) -> Self {
        Self {
            movie,
            path,
            kind: PhantomData,
        }
    }
}

impl<K> Clone for Key<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for Key<K> {}

pub mod kind {
    pub struct Instance;
    pub struct Text;
    pub struct Shape;
    pub struct Timeline;
    pub struct InstanceClass;
    pub struct SymbolTimeline;
    pub struct MainTimeline;
    pub struct Stage;
    pub struct Number;
    pub struct Str;
    pub struct Flag;
    pub struct Translate;
    pub struct FontClass;
    pub struct Bounds;
}

/// Placement of an instance (local and world matrix, colour, filters, track).
pub type PlacementKey = Key<kind::Instance>;
/// A text field: its character data plus placement.
pub type TextKey = Key<kind::Text>;
pub type ShapeKey = Key<kind::Shape>;
/// Frame count and labels of the sprite at an instance path.
pub type TimelineKey = Key<kind::Timeline>;
/// The exported class of the sprite at an instance path.
pub type InstanceClassKey = Key<kind::InstanceClass>;
/// Frame count and labels of an exported symbol; the path is its class name.
pub type SymbolTimelineKey = Key<kind::SymbolTimeline>;
/// The movie's main timeline; the path is empty.
pub type MainTimelineKey = Key<kind::MainTimeline>;
/// The movie's stage rectangle and frame rate; the path is empty.
pub type StageKey = Key<kind::Stage>;
/// An `ActionScript` constant, `Class.member`.
pub type NumberKey = Key<kind::Number>;
pub type StringKey = Key<kind::Str>;
pub type FlagKey = Key<kind::Flag>;
/// A `$KEY` of the translate table; the movie is the table's document name.
pub type TranslateKey = Key<kind::Translate>;
/// A font class such as `$MAIN_Font_Bold`, resolved to its family by the font configuration.
pub type FontClassKey = Key<kind::FontClass>;
/// The bounds of a display object and everything below it, in the space of its path's root.
pub type BoundsKey = Key<kind::Bounds>;

impl TranslateKey {
    pub const fn text(key: &'static str) -> Self {
        Self::new(TRANSLATE_DOCUMENT, key)
    }
}

impl FontClassKey {
    pub const fn class(class: &'static str) -> Self {
        Self::new(FONT_CONFIG_DOCUMENT, class)
    }
}

impl MainTimelineKey {
    pub const fn of(movie: &'static str) -> Self {
        Self::new(movie, "")
    }
}

impl KeyKind for kind::Instance {
    const DECLARED: DeclaredKind = DeclaredKind::Placement;
    type Output = Placement;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<Placement, UiLayoutError> {
        layout.movie(key.movie)?.placement(key.path)
    }
}

impl KeyKind for kind::Text {
    const DECLARED: DeclaredKind = DeclaredKind::Text;
    type Output = TextField;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<TextField, UiLayoutError> {
        layout.movie(key.movie)?.text_field(key.path)
    }
}

impl KeyKind for kind::Shape {
    const DECLARED: DeclaredKind = DeclaredKind::Shape;
    type Output = Shape;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<Shape, UiLayoutError> {
        layout.movie(key.movie)?.shape(key.path)
    }
}

impl KeyKind for kind::Timeline {
    const DECLARED: DeclaredKind = DeclaredKind::Timeline;
    type Output = Timeline;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<Timeline, UiLayoutError> {
        layout.movie(key.movie)?.instance_timeline(key.path)
    }
}

impl KeyKind for kind::InstanceClass {
    const DECLARED: DeclaredKind = DeclaredKind::InstanceClass;
    type Output = String;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<String, UiLayoutError> {
        layout.movie(key.movie)?.instance_class(key.path)
    }
}

impl KeyKind for kind::SymbolTimeline {
    const DECLARED: DeclaredKind = DeclaredKind::SymbolTimeline;
    type Output = Timeline;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<Timeline, UiLayoutError> {
        layout.movie(key.movie)?.symbol_timeline(key.path)
    }
}

impl KeyKind for kind::MainTimeline {
    const DECLARED: DeclaredKind = DeclaredKind::MainTimeline;
    type Output = Timeline;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<Timeline, UiLayoutError> {
        Ok(layout.movie(key.movie)?.main_timeline())
    }
}

impl StageKey {
    pub const fn of(movie: &'static str) -> Self {
        Self::new(movie, "")
    }
}

impl KeyKind for kind::Stage {
    const DECLARED: DeclaredKind = DeclaredKind::Stage;
    type Output = MovieStage;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<MovieStage, UiLayoutError> {
        Ok(layout.movie(key.movie)?.stage())
    }
}

impl KeyKind for kind::Bounds {
    const DECLARED: DeclaredKind = DeclaredKind::Bounds;
    type Output = [f32; 4];
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<[f32; 4], UiLayoutError> {
        layout.movie(key.movie)?.subtree_bounds(key.path)
    }
}

impl KeyKind for kind::Number {
    const DECLARED: DeclaredKind = DeclaredKind::Number;
    type Output = f64;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<f64, UiLayoutError> {
        let movie = layout.movie(key.movie)?;
        match movie.constant(key.path)? {
            Scalar::Number(value) => Ok(*value),
            other => Err(movie.wrong_scalar(key.path, "a number", other)),
        }
    }
}

impl KeyKind for kind::Str {
    const DECLARED: DeclaredKind = DeclaredKind::Str;
    type Output = String;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<String, UiLayoutError> {
        let movie = layout.movie(key.movie)?;
        match movie.constant(key.path)? {
            Scalar::Text(value) => Ok(value.clone()),
            other => Err(movie.wrong_scalar(key.path, "a string", other)),
        }
    }
}

impl KeyKind for kind::Flag {
    const DECLARED: DeclaredKind = DeclaredKind::Flag;
    type Output = bool;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<bool, UiLayoutError> {
        let movie = layout.movie(key.movie)?;
        match movie.constant(key.path)? {
            Scalar::Flag(value) => Ok(*value),
            other => Err(movie.wrong_scalar(key.path, "a boolean", other)),
        }
    }
}

impl KeyKind for kind::Translate {
    const DECLARED: DeclaredKind = DeclaredKind::Translate;
    type Output = String;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<String, UiLayoutError> {
        layout.translate_text(key.path)
    }
}

impl KeyKind for kind::FontClass {
    const DECLARED: DeclaredKind = DeclaredKind::FontClass;
    type Output = String;
    fn resolve(layout: &UiLayout, key: &Key<Self>) -> Result<String, UiLayoutError> {
        layout.font_family(key.path)
    }
}

/// The type of the values a literal list holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarKind {
    Number,
    Text,
    Flag,
}

impl ScalarKind {
    fn matches(self, value: &Scalar) -> bool {
        matches!(
            (self, value),
            (Self::Number, Scalar::Number(_))
                | (Self::Text, Scalar::Text(_))
                | (Self::Flag, Scalar::Flag(_))
        )
    }

    fn expected(self) -> &'static str {
        match self {
            Self::Number => "a number",
            Self::Text => "a string",
            Self::Flag => "a boolean",
        }
    }
}

/// A list of `ActionScript` literals `Class.method#signature` whose length must equal `count`.
#[derive(Clone, Copy)]
pub struct LiteralKey {
    pub movie: &'static str,
    pub path: &'static str,
    pub count: usize,
    pub scalar: ScalarKind,
}

impl LiteralKey {
    /// A list of numbers.
    pub const fn new(movie: &'static str, path: &'static str, count: usize) -> Self {
        Self {
            movie,
            path,
            count,
            scalar: ScalarKind::Number,
        }
    }

    /// A list of strings.
    pub const fn texts(movie: &'static str, path: &'static str, count: usize) -> Self {
        Self {
            scalar: ScalarKind::Text,
            ..Self::new(movie, path, count)
        }
    }

    /// A list of booleans.
    pub const fn flags(movie: &'static str, path: &'static str, count: usize) -> Self {
        Self {
            scalar: ScalarKind::Flag,
            ..Self::new(movie, path, count)
        }
    }
}

/// What a declared key asks the layout for; lets tests build a layout that satisfies a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredKind {
    Placement,
    Text,
    Shape,
    Timeline,
    InstanceClass,
    SymbolTimeline,
    MainTimeline,
    Stage,
    Number,
    Str,
    Flag,
    Translate,
    FontClass,
    Bounds,
    Literal { count: usize, scalar: ScalarKind },
    Label { label: &'static str, symbol: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declaration {
    pub movie: &'static str,
    pub path: &'static str,
    pub kind: DeclaredKind,
}

/// Something a UI module needs from the layout: it reports every failure instead of the first.
pub trait KeyCheck: Sync {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError>;
    /// Every key this check reads.
    fn declare(&self, out: &mut Vec<Declaration>);
}

impl<K: KeyKind> KeyCheck for Key<K> {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError> {
        K::resolve(layout, self).err().into_iter().collect()
    }

    fn declare(&self, out: &mut Vec<Declaration>) {
        out.push(Declaration {
            movie: self.movie,
            path: self.path,
            kind: K::DECLARED,
        });
    }
}

impl KeyCheck for LiteralKey {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError> {
        let values = match layout.literal_list(self) {
            Ok(values) => values,
            Err(error) => return vec![error],
        };
        values
            .iter()
            .filter(|value| !self.scalar.matches(value))
            .map(|value| UiLayoutError::WrongKind {
                movie: self.movie.to_owned(),
                key: self.path.to_owned(),
                expected: self.scalar.expected(),
                actual: value.type_name().to_owned(),
            })
            .collect()
    }

    fn declare(&self, out: &mut Vec<Declaration>) {
        out.push(Declaration {
            movie: self.movie,
            path: self.path,
            kind: DeclaredKind::Literal {
                count: self.count,
                scalar: self.scalar,
            },
        });
    }
}

/// The frame a label names on a sprite's timeline: an instance's sprite, or an exported
/// symbol's own timeline when `symbol`.
#[derive(Clone, Copy)]
pub struct LabelKey {
    pub movie: &'static str,
    pub path: &'static str,
    pub label: &'static str,
    pub symbol: bool,
}

impl LabelKey {
    pub const fn instance(movie: &'static str, path: &'static str, label: &'static str) -> Self {
        Self {
            movie,
            path,
            label,
            symbol: false,
        }
    }

    pub const fn symbol(movie: &'static str, class: &'static str, label: &'static str) -> Self {
        Self {
            movie,
            path: class,
            label,
            symbol: true,
        }
    }

    /// The 1-based frame of the label.
    pub fn resolve(&self, layout: &UiLayout) -> Result<u32, UiLayoutError> {
        let movie = layout.movie(self.movie)?;
        let timeline = if self.symbol {
            movie.symbol_timeline(self.path)?
        } else {
            movie.instance_timeline(self.path)?
        };
        timeline
            .labels
            .get(self.label)
            .copied()
            .ok_or_else(|| UiLayoutError::MissingKey {
                movie: self.movie.to_owned(),
                key: format!("{}:{}", self.path, self.label),
                kind: "frame label",
                nearby: timeline.labels.keys().cloned().collect(),
            })
    }
}

impl KeyCheck for LabelKey {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError> {
        self.resolve(layout).err().into_iter().collect()
    }

    fn declare(&self, out: &mut Vec<Declaration>) {
        out.push(Declaration {
            movie: self.movie,
            path: self.path,
            kind: DeclaredKind::Label {
                label: self.label,
                symbol: self.symbol,
            },
        });
    }
}

/// The keys of one UI module, checked together.
pub struct KeyGroup(pub &'static [&'static dyn KeyCheck]);

impl KeyCheck for KeyGroup {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError> {
        self.0
            .iter()
            .flat_map(|check| check.failures(layout))
            .collect()
    }

    fn declare(&self, out: &mut Vec<Declaration>) {
        for check in self.0 {
            check.declare(out);
        }
    }
}

impl<T: KeyCheck, const N: usize> KeyCheck for [T; N] {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError> {
        self.iter()
            .flat_map(|check| check.failures(layout))
            .collect()
    }

    fn declare(&self, out: &mut Vec<Declaration>) {
        for check in self {
            check.declare(out);
        }
    }
}

impl<T: KeyCheck> KeyCheck for Option<T> {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError> {
        self.as_ref()
            .map(|check| check.failures(layout))
            .unwrap_or_default()
    }

    fn declare(&self, out: &mut Vec<Declaration>) {
        if let Some(check) = self {
            check.declare(out);
        }
    }
}

/// Implements [`KeyCheck`] for a struct of keys by checking each named field.
macro_rules! key_check_fields {
    ($type:ty { $($field:ident),+ $(,)? }) => {
        impl $crate::ui_layout::KeyCheck for $type {
            fn failures(
                &self,
                layout: &$crate::ui_layout::UiLayout,
            ) -> Vec<$crate::ui_layout::UiLayoutError> {
                let mut failures = Vec::new();
                $(failures.extend($crate::ui_layout::KeyCheck::failures(&self.$field, layout));)+
                failures
            }

            fn declare(&self, out: &mut Vec<$crate::ui_layout::Declaration>) {
                $($crate::ui_layout::KeyCheck::declare(&self.$field, out);)+
            }
        }
    };
}
pub(crate) use key_check_fields;

/// The keys of one UI module and the step that resolves them together into the module's
/// layout, which can fail even when every key resolves on its own (a shape that must be
/// axis-aligned, a literal that must name a mode).
pub struct ModuleKeys {
    pub keys: &'static [&'static dyn KeyCheck],
    pub resolve: fn(&UiLayout) -> Result<(), UiLayoutError>,
}

impl KeyCheck for ModuleKeys {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError> {
        let failures = KeyGroup(self.keys).failures(layout);
        if failures.is_empty() {
            (self.resolve)(layout).err().into_iter().collect()
        } else {
            failures
        }
    }

    fn declare(&self, out: &mut Vec<Declaration>) {
        KeyGroup(self.keys).declare(out);
    }
}
