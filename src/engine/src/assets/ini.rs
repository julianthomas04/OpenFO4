//! Reader for the game's INI settings cascade, copied into the install by the
//! installer (step 2d, `<install>/assets/data`). Resolution order mirrors the
//! game's: `Fallout4_Default.ini` (shipped) is overridden by the user's
//! Fallout4.ini, Fallout4Prefs.ini, then Fallout4Custom.ini; later files win.
//!
//! Everything the game stores in its INIs comes from here and nowhere else:
//! the engine carries no copy of a vanilla value. A required file that is
//! missing or unreadable, a setting that no file sets, and a value that does
//! not parse or break its rule are all errors the caller turns into a fatal
//! startup error. The one leniency is a value above a limit the engine itself
//! imposes (a capacity): [`capped_at_engine_limit`] clamps it with a warning.
//!
//! Keys are matched case-insensitively and globally rather than per-section:
//! Bethesda's Hungarian-prefixed setting names are unique in practice, and
//! this keeps lookups section-agnostic (the same key can legitimately appear
//! in Fallout4.ini's `[Display]` and Prefs' `[Display]`). The section a caller
//! names with an [`IniKey`] only tells the user where the game keeps the
//! setting. `Name:Section=value` sets `Name` as if written under `[Section]`.

use std::collections::HashMap;
use std::fmt::Display;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

struct CascadeFile {
    name: &'static str,
    /// The game ships (or always creates) it and the engine reads values only it holds.
    required: bool,
}

/// Lowest priority first. `Fallout4Prefs.ini` holds settings no other file does (shadow,
/// texture and HUD quality) and the game writes it on first launch; `Fallout4.ini` repeats the
/// shipped defaults and `Fallout4Custom.ini` exists only if the user made it.
const CASCADE: [CascadeFile; 4] = [
    CascadeFile {
        name: "Fallout4_Default.ini",
        required: true,
    },
    CascadeFile {
        name: "Fallout4.ini",
        required: false,
    },
    CascadeFile {
        name: "Fallout4Prefs.ini",
        required: true,
    },
    CascadeFile {
        name: "Fallout4Custom.ini",
        required: false,
    },
];

/// A setting the engine needs and the section the game keeps it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IniKey {
    pub section: &'static str,
    pub name: &'static str,
}

impl IniKey {
    pub const fn new(section: &'static str, name: &'static str) -> Self {
        Self { section, name }
    }
}

/// A type a setting can be read as, and how an error names what the value had to be.
pub trait IniValue: Sized {
    const EXPECTED: &'static str;

    fn parse_value(text: &str) -> Option<Self>;
}

macro_rules! ini_value_from_str {
    ($($type:ty => $expected:literal),+ $(,)?) => {$(
        impl IniValue for $type {
            const EXPECTED: &'static str = $expected;

            fn parse_value(text: &str) -> Option<Self> {
                text.parse().ok()
            }
        }
    )+};
}

ini_value_from_str! {
    i32 => "a whole number",
    String => "text",
}

/// A whole number too large for the type reads as the type's maximum, so the engine limit it is
/// clamped to (with its warning) applies instead of the value being called unreadable.
macro_rules! ini_value_unsigned {
    ($($type:ty),+ $(,)?) => {$(
        impl IniValue for $type {
            const EXPECTED: &'static str = "a whole number, 0 or more";

            fn parse_value(text: &str) -> Option<Self> {
                let digits = text.strip_prefix('+').unwrap_or(text);
                if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                Some(digits.parse().unwrap_or(Self::MAX))
            }
        }
    )+};
}

ini_value_unsigned!(u16, u32, usize);

impl IniValue for f32 {
    const EXPECTED: &'static str = "a number";

    /// The game's INIs write some floats with a trailing `f` (`0.5f`).
    fn parse_value(text: &str) -> Option<Self> {
        text.parse()
            .ok()
            .or_else(|| text.strip_suffix(['f', 'F'])?.parse().ok())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IniError {
    #[error(
        "fatal: required game INI {name} is missing from {dir}; the installer copies it from the \
         game (the game writes Fallout4Prefs.ini on its first launch), so launch the game once if \
         needed and run the installer's `inis-only` command"
    )]
    MissingFile { name: &'static str, dir: PathBuf },
    #[error("fatal: cannot read game INI {path}")]
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "fatal: game INI {path} is UTF-16 encoded or not text at all; save it as UTF-8 or ANSI"
    )]
    UnsupportedEncoding { path: PathBuf },
    #[error(
        "fatal: INI setting {key} (section [{section}]) is not set in any game INI; searched {searched}"
    )]
    MissingKey {
        key: &'static str,
        section: &'static str,
        searched: String,
    },
    #[error(
        "fatal: INI setting {key}={value:?} ({file}, section [{found_in}]) cannot be read: expected {expected}"
    )]
    Unparsable {
        key: &'static str,
        value: String,
        file: String,
        found_in: String,
        expected: &'static str,
    },
    #[error(
        "fatal: INI setting {key}={value} ({file}, section [{found_in}]) is invalid: expected {requirement}"
    )]
    Invalid {
        key: &'static str,
        value: String,
        file: String,
        found_in: String,
        requirement: String,
    },
}

/// `value`, or `limit` with a warning when `value` is above what the engine supports. Only for
/// engine capacities and implemented ranges; a value that is wrong in kind is an error instead.
pub fn capped_at_engine_limit<T: PartialOrd + Display + Copy>(
    key: IniKey,
    value: T,
    limit: T,
) -> T {
    if value > limit {
        tracing::warn!(
            "INI setting {}={value} is above what the engine supports; using {limit}",
            key.name
        );
        limit
    } else {
        value
    }
}

struct Entry {
    value: String,
    file: String,
    section: String,
}

pub struct GameIni {
    entries: HashMap<String, Entry>,
    /// Where and what was read, for the error naming the files a missing key was searched in.
    searched: String,
}

impl GameIni {
    /// Reads the cascade files in `ini_dir`. A missing required file, a file that exists but
    /// cannot be read and a UTF-16 file are errors; a missing optional file is skipped. Bytes
    /// that are not valid UTF-8 (the game's INIs are often ANSI-encoded) are replaced, so one
    /// stray character in a comment does not discard the whole file.
    pub fn load(ini_dir: &Path) -> Result<Self, IniError> {
        let mut files = Vec::with_capacity(CASCADE.len());
        for file in &CASCADE {
            let path = ini_dir.join(file.name);
            let text = match std::fs::read(&path) {
                Ok(bytes) if is_utf16_or_binary(&bytes) => {
                    return Err(IniError::UnsupportedEncoding { path });
                }
                Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && !file.required => None,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(IniError::MissingFile {
                        name: file.name,
                        dir: ini_dir.to_path_buf(),
                    });
                }
                Err(source) => return Err(IniError::Unreadable { path, source }),
            };
            files.push((file.name.to_string(), text));
        }
        Ok(Self::from_files(&ini_dir.display().to_string(), &files))
    }

    /// Parses INI texts given lowest-priority first; later texts override earlier ones.
    #[cfg(test)]
    pub fn from_texts<'a>(texts: impl IntoIterator<Item = &'a str>) -> Self {
        let files: Vec<_> = texts
            .into_iter()
            .enumerate()
            .map(|(index, text)| (format!("text {}", index + 1), Some(text.to_string())))
            .collect();
        Self::from_files("memory", &files)
    }

    /// `files` is lowest-priority first; a file that is `None` was not present.
    fn from_files(location: &str, files: &[(String, Option<String>)]) -> Self {
        let mut entries = HashMap::new();
        for (name, text) in files {
            if let Some(text) = text {
                read_text(&mut entries, name, text);
            }
        }
        Self {
            entries,
            searched: describe_search(location, files),
        }
    }

    /// The setting's value parsed as `T`.
    pub fn require<T: IniValue>(&self, key: IniKey) -> Result<T, IniError> {
        let entry = self.entry(key)?;
        T::parse_value(&entry.value).ok_or_else(|| IniError::Unparsable {
            key: key.name,
            value: entry.value.clone(),
            file: entry.file.clone(),
            found_in: entry.section.clone(),
            expected: T::EXPECTED,
        })
    }

    /// The name of the INI whose value for `key` wins, `None` when no file sets it.
    pub fn defined_in(&self, key: IniKey) -> Option<&str> {
        self.entry(key).ok().map(|entry| entry.file.as_str())
    }

    /// Like [`Self::require`], and `is_valid` must hold for the value; `requirement` says in the
    /// error what the value had to be.
    pub fn require_valid<T: IniValue>(
        &self,
        key: IniKey,
        requirement: &str,
        is_valid: impl Fn(&T) -> bool,
    ) -> Result<T, IniError> {
        let value = self.require::<T>(key)?;
        if is_valid(&value) {
            return Ok(value);
        }
        let entry = self.entry(key)?;
        Err(IniError::Invalid {
            key: key.name,
            value: entry.value.clone(),
            file: entry.file.clone(),
            found_in: entry.section.clone(),
            requirement: requirement.to_string(),
        })
    }

    pub fn require_in_range<T>(&self, key: IniKey, range: RangeInclusive<T>) -> Result<T, IniError>
    where
        T: IniValue + PartialOrd + Display,
    {
        let requirement = format!("{} to {}", range.start(), range.end());
        self.require_valid(key, &requirement, |value| range.contains(value))
    }

    /// Like [`Self::require`], clamped to `limit` with a warning (see [`capped_at_engine_limit`]).
    pub fn require_capped<T>(&self, key: IniKey, limit: T) -> Result<T, IniError>
    where
        T: IniValue + PartialOrd + Display + Copy,
    {
        Ok(capped_at_engine_limit(key, self.require(key)?, limit))
    }

    /// A `0` or `1` switch.
    pub fn require_flag(&self, key: IniKey) -> Result<bool, IniError> {
        let entry = self.entry(key)?;
        match entry.value.as_str() {
            "0" => Ok(false),
            "1" => Ok(true),
            _ => Err(IniError::Invalid {
                key: key.name,
                value: entry.value.clone(),
                file: entry.file.clone(),
                found_in: entry.section.clone(),
                requirement: "0 or 1".to_string(),
            }),
        }
    }

    pub fn require_non_negative(&self, key: IniKey) -> Result<f32, IniError> {
        self.require_valid(key, "a finite number, 0 or more", |v: &f32| {
            v.is_finite() && *v >= 0.0
        })
    }

    pub fn require_positive(&self, key: IniKey) -> Result<f32, IniError> {
        self.require_valid(key, "a finite number above 0", |v: &f32| {
            v.is_finite() && *v > 0.0
        })
    }

    fn entry(&self, key: IniKey) -> Result<&Entry, IniError> {
        self.entries
            .get(&key.name.to_ascii_lowercase())
            .ok_or_else(|| IniError::MissingKey {
                key: key.name,
                section: key.section,
                searched: self.searched.clone(),
            })
    }
}

/// A UTF-16 byte order mark, or a NUL byte (UTF-16 text without a mark has them between letters).
fn is_utf16_or_binary(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) || bytes.contains(&0)
}

fn describe_search(location: &str, files: &[(String, Option<String>)]) -> String {
    let names_where = |present: bool| -> Vec<&str> {
        files
            .iter()
            .filter(|(_, text)| text.is_some() == present)
            .map(|(name, _)| name.as_str())
            .collect()
    };
    let (read, absent) = (names_where(true), names_where(false));
    if absent.is_empty() {
        format!("{location}: {}", read.join(", "))
    } else {
        format!(
            "{location}: {} (not present: {})",
            read.join(", "),
            absent.join(", ")
        )
    }
}

/// Adds every setting of `text` (from the file `file`) to `entries`, replacing earlier values.
/// A leading UTF-8 byte order mark is ignored.
fn read_text(entries: &mut HashMap<String, Entry>, file: &str, text: &str) {
    let mut section = String::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            section = header.split(']').next().unwrap_or("").trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (name, setting_section) = match key.split_once(':') {
            Some((name, qualified)) => (name, qualified),
            None => (key, section.as_str()),
        };
        entries.insert(
            name.trim().to_ascii_lowercase(),
            Entry {
                value: value.split(';').next().unwrap_or("").trim().to_string(),
                file: file.to_string(),
                section: setting_section.trim().to_string(),
            },
        );
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use super::*;
    use crate::audio::test_support::SharedBuffer;
    use proptest::prelude::*;

    const KEY: IniKey = IniKey::new("Display", "iKey");

    fn key(name: &'static str) -> IniKey {
        IniKey::new("Display", name)
    }

    fn ini(text: &str) -> GameIni {
        GameIni::from_texts([text])
    }

    /// Everything `action` logs at warning level or above on this thread.
    fn capture_warnings<R>(action: impl FnOnce() -> R) -> String {
        let log = SharedBuffer::default();
        let guard = log.capture("warn");
        action();
        drop(guard);
        log.drain()
    }

    fn write(dir: &Path, file: &str, text: &str) {
        std::fs::write(dir.join(file), text).unwrap();
    }

    fn write_required(dir: &Path) {
        write(dir, "Fallout4_Default.ini", "");
        write(dir, "Fallout4Prefs.ini", "");
    }

    #[test]
    fn later_text_overrides_earlier_text() {
        let ini = GameIni::from_texts(["[Display]\nfFoo=1.0\n", "[Display]\nfFoo=2.5\n"]);
        assert_eq!(ini.require::<f32>(key("fFoo")).unwrap(), 2.5);
    }

    #[test]
    fn earlier_keys_survive_when_later_text_does_not_set_them() {
        let ini = GameIni::from_texts(["fA=1\nfB=2\n", "fB=3\n"]);
        assert_eq!(ini.require::<i32>(key("fA")).unwrap(), 1);
        assert_eq!(ini.require::<i32>(key("fB")).unwrap(), 3);
    }

    #[test]
    fn keys_match_case_insensitively() {
        assert_eq!(ini("iSize=4\n").require::<i32>(key("ISIZE")).unwrap(), 4);
        assert_eq!(ini("ISIZE=4\n").require::<i32>(key("iSize")).unwrap(), 4);
    }

    #[test]
    fn same_key_in_different_sections_is_one_setting_and_the_last_wins() {
        let ini = ini("[Display]\niSize=1\n[Other]\niSize=2\n");
        assert_eq!(ini.require::<i32>(key("iSize")).unwrap(), 2);
    }

    #[test]
    fn a_setting_is_found_whatever_section_it_is_under() {
        let ini = ini("[Interface]\niSize=3\n");
        assert_eq!(ini.require::<i32>(key("iSize")).unwrap(), 3);
    }

    #[test]
    fn a_section_qualified_name_sets_the_plain_name() {
        let ini = ini("[General]\nfSun:Display=0.5\n");
        assert_eq!(ini.require::<f32>(key("fSun")).unwrap(), 0.5);
        let error = ini.require_valid::<f32>(key("fSun"), "never", |_| false);
        assert!(error.unwrap_err().to_string().contains("[Display]"));
    }

    #[test]
    fn comments_sections_and_blank_lines_are_skipped() {
        let ini = ini("; iHidden=1\n[Section]\n\n  \niShown=2\n");
        assert!(matches!(
            ini.require::<i32>(key("iHidden")),
            Err(IniError::MissingKey { .. })
        ));
        assert_eq!(ini.require::<i32>(key("iShown")).unwrap(), 2);
    }

    #[test]
    fn a_section_header_may_carry_a_trailing_comment() {
        let ini = ini("[Display] ; the screen\niSize=1\n");
        let error = ini.require_valid::<i32>(key("iSize"), "never", |_| false);
        assert!(error.unwrap_err().to_string().contains("[Display]"));
    }

    #[test]
    fn windows_line_endings_do_not_leak_into_values() {
        let ini = ini("iA=1\r\nsB=text\r\n");
        assert_eq!(ini.require::<i32>(key("iA")).unwrap(), 1);
        assert_eq!(ini.require::<String>(key("sB")).unwrap(), "text");
    }

    #[test]
    fn byte_order_mark_does_not_break_the_first_key() {
        let ini = ini("\u{feff}iFirst=5\n");
        assert_eq!(ini.require::<i32>(key("iFirst")).unwrap(), 5);
    }

    #[test]
    fn trailing_comment_and_whitespace_are_stripped_from_value() {
        let ini = ini("  fRate =  0.5  ; half speed\n");
        assert_eq!(ini.require::<f32>(key("fRate")).unwrap(), 0.5);
    }

    #[test]
    fn value_keeps_equals_signs_after_the_first() {
        let ini = ini("sPath=a=b\n");
        assert_eq!(ini.require::<String>(key("sPath")).unwrap(), "a=b");
    }

    #[test]
    fn line_without_equals_sign_is_ignored() {
        assert!(matches!(
            ini("iNoValue\n").require::<i32>(key("iNoValue")),
            Err(IniError::MissingKey { .. })
        ));
    }

    #[test]
    fn missing_key_error_names_the_key_the_section_and_the_files_searched() {
        let ini = GameIni::from_texts(["iOther=1\n", "iOther=2\n"]);
        let message = ini
            .require::<i32>(IniKey::new("Interface", "fWanted"))
            .unwrap_err()
            .to_string();
        for expected in ["fWanted", "[Interface]", "text 1", "text 2", "memory"] {
            assert!(message.contains(expected), "{expected} not in {message}");
        }
    }

    #[test]
    fn empty_value_is_present_but_unparsable_as_number() {
        let error = ini("iEmpty=\n").require::<i32>(key("iEmpty")).unwrap_err();
        assert!(matches!(&error, IniError::Unparsable { value, .. } if value.is_empty()));
    }

    #[test]
    fn unparsable_error_names_key_value_file_section_and_type() {
        let ini = GameIni::from_texts(["[Display]\niCount=1\n", "[General]\niCount=lots\n"]);
        let message = ini.require::<i32>(key("iCount")).unwrap_err().to_string();
        for expected in [
            "iCount",
            "\"lots\"",
            "text 2",
            "[General]",
            "a whole number",
        ] {
            assert!(message.contains(expected), "{expected} not in {message}");
        }
    }

    #[test]
    fn a_negative_unsigned_setting_is_reported_as_needing_a_non_negative_whole_number() {
        let ini = ini("iNegative=-1\n");
        let messages = [
            ini.require::<u32>(key("iNegative"))
                .unwrap_err()
                .to_string(),
            ini.require::<usize>(key("iNegative"))
                .unwrap_err()
                .to_string(),
            ini.require::<u16>(key("iNegative"))
                .unwrap_err()
                .to_string(),
        ];
        for message in messages {
            assert!(message.contains("a whole number, 0 or more"), "{message}");
            assert!(
                !message.contains("u32") && !message.contains("usize"),
                "{message}"
            );
        }
    }

    #[test]
    fn a_whole_number_too_large_for_the_type_reads_as_its_maximum() {
        let ini = ini("iHuge=99999999999999999999999999\niLarge=70000\niPlus=+5\n");
        assert_eq!(ini.require::<u16>(key("iHuge")).unwrap(), u16::MAX);
        assert_eq!(ini.require::<u16>(key("iLarge")).unwrap(), u16::MAX);
        assert_eq!(ini.require::<u32>(key("iHuge")).unwrap(), u32::MAX);
        assert_eq!(ini.require::<usize>(key("iHuge")).unwrap(), usize::MAX);
        assert_eq!(ini.require::<u32>(key("iLarge")).unwrap(), 70000);
        assert_eq!(ini.require::<u16>(key("iPlus")).unwrap(), 5);
    }

    #[test]
    fn an_overflowing_count_is_clamped_to_the_engine_limit_instead_of_being_unreadable() {
        let ini = ini("iAnisotropy=70000\niMipSkip=99999999999\n");
        assert_eq!(
            ini.require_capped::<u16>(key("iAnisotropy"), 16).unwrap(),
            16
        );
        assert_eq!(ini.require_capped::<u32>(key("iMipSkip"), 4).unwrap(), 4);
    }

    #[test]
    fn text_that_is_not_a_whole_number_is_unreadable_for_unsigned_types() {
        let ini = ini("iEmpty=\niSign=+\niFraction=1.5\niHex=0x10\niSpaced=1 2\n");
        for name in ["iEmpty", "iSign", "iFraction", "iHex", "iSpaced"] {
            assert!(
                matches!(
                    ini.require::<u32>(key(name)),
                    Err(IniError::Unparsable { .. })
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn a_value_above_the_engine_limit_warns_with_the_key_and_the_limit() {
        let logged = capture_warnings(|| capped_at_engine_limit(KEY, 17, 16));
        assert!(logged.contains("iKey"), "{logged}");
        assert!(logged.contains("17") && logged.contains("16"), "{logged}");
    }

    #[test]
    fn a_value_within_the_engine_limit_logs_nothing() {
        let logged = capture_warnings(|| {
            capped_at_engine_limit(KEY, 16, 16);
            capped_at_engine_limit(KEY, 0, 16);
        });
        assert!(logged.is_empty(), "{logged}");
    }

    #[test]
    fn floats_accept_the_trailing_f_the_game_writes() {
        let ini = ini("fA=0.5f\nfB=2F\nfC=inf\nfD=f\n");
        assert_eq!(ini.require::<f32>(key("fA")).unwrap(), 0.5);
        assert_eq!(ini.require::<f32>(key("fB")).unwrap(), 2.0);
        assert!(ini.require::<f32>(key("fC")).unwrap().is_infinite());
        assert!(matches!(
            ini.require::<f32>(key("fD")),
            Err(IniError::Unparsable { .. })
        ));
    }

    #[test]
    fn a_flag_is_zero_or_one_and_nothing_else() {
        let ini = ini("bOff=0\nbOn=1\nbTwo=2\nbWord=true\nbEmpty=\nbNegative=-1\n");
        assert!(!ini.require_flag(key("bOff")).unwrap());
        assert!(ini.require_flag(key("bOn")).unwrap());
        for name in ["bTwo", "bWord", "bEmpty", "bNegative"] {
            let error = ini.require_flag(key(name)).unwrap_err();
            assert!(matches!(error, IniError::Invalid { .. }), "{name}");
            assert!(error.to_string().contains("0 or 1"), "{name}: {error}");
        }
        assert!(matches!(
            ini.require_flag(key("bMissing")),
            Err(IniError::MissingKey { .. })
        ));
    }

    #[test]
    fn the_file_that_sets_a_key_is_the_last_one_that_does() {
        let ini = GameIni::from_texts(["iA=1\niB=1\n", "iB=2\n"]);
        assert_eq!(ini.defined_in(key("iA")), Some("text 1"));
        assert_eq!(ini.defined_in(key("iB")), Some("text 2"));
        assert_eq!(ini.defined_in(key("iC")), None);
    }

    #[test]
    fn a_value_above_the_engine_limit_is_clamped_and_anything_else_is_kept() {
        assert_eq!(capped_at_engine_limit(KEY, 17, 16), 16);
        assert_eq!(capped_at_engine_limit(KEY, 16, 16), 16);
        assert_eq!(capped_at_engine_limit(KEY, 0, 16), 0);
        assert_eq!(capped_at_engine_limit(KEY, 9000.5_f32, 8192.0), 8192.0);
    }

    #[test]
    fn range_accepts_both_boundaries_and_rejects_just_outside() {
        let ini = ini("iLow=1\niHigh=8\niUnder=0\niOver=9\n");
        assert_eq!(ini.require_in_range(key("iLow"), 1..=8).unwrap(), 1);
        assert_eq!(ini.require_in_range(key("iHigh"), 1..=8).unwrap(), 8);
        for name in ["iUnder", "iOver"] {
            let error = ini.require_in_range::<i32>(key(name), 1..=8).unwrap_err();
            assert!(matches!(error, IniError::Invalid { .. }), "{name}");
        }
    }

    #[test]
    fn out_of_range_error_names_key_value_file_section_and_requirement() {
        let ini = GameIni::from_texts(["[Display]\niLevel=9\n"]);
        let message = ini
            .require_in_range::<i32>(key("iLevel"), 1..=8)
            .unwrap_err()
            .to_string();
        for expected in ["iLevel", "9", "text 1", "[Display]", "1 to 8"] {
            assert!(message.contains(expected), "{expected} not in {message}");
        }
    }

    #[test]
    fn require_valid_applies_the_predicate_to_the_parsed_value() {
        let ini = ini("iEven=4\niOdd=5\n");
        let even = |n: &i32| n % 2 == 0;
        assert_eq!(ini.require_valid(key("iEven"), "even", even).unwrap(), 4);
        assert!(matches!(
            ini.require_valid(key("iOdd"), "even", even),
            Err(IniError::Invalid { .. })
        ));
    }

    #[test]
    fn missing_key_is_reported_before_any_validation() {
        assert!(matches!(
            ini("").require_valid::<i32>(KEY, "never", |_| false),
            Err(IniError::MissingKey { .. })
        ));
    }

    #[test]
    fn float_requirements_reject_negative_zero_and_non_finite_values() {
        let ini = ini("fZero=0\nfNegative=-1\nfNan=NaN\nfInf=inf\nfPositive=2.5\n");
        assert_eq!(ini.require_positive(key("fPositive")).unwrap(), 2.5);
        assert_eq!(ini.require_non_negative(key("fZero")).unwrap(), 0.0);
        for name in ["fZero", "fNegative", "fNan", "fInf"] {
            assert!(ini.require_positive(key(name)).is_err(), "positive {name}");
        }
        for name in ["fNegative", "fNan", "fInf"] {
            assert!(
                ini.require_non_negative(key(name)).is_err(),
                "non-negative {name}"
            );
        }
    }

    #[test]
    fn load_cascade_order_is_default_then_user_then_prefs_then_custom() {
        let dir = tempfile::tempdir().unwrap();
        for (file, value) in [
            ("Fallout4Custom.ini", 4),
            ("Fallout4Prefs.ini", 3),
            ("Fallout4.ini", 2),
            ("Fallout4_Default.ini", 1),
        ] {
            write(dir.path(), file, &format!("iV={value}\n"));
        }
        let ini = GameIni::load(dir.path()).unwrap();
        assert_eq!(ini.require::<i32>(key("iV")).unwrap(), 4);
    }

    #[test]
    fn load_keeps_lower_files_values_the_higher_ones_do_not_set() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Fallout4_Default.ini", "iA=1\niB=1\n");
        write(dir.path(), "Fallout4Prefs.ini", "iB=2\n");
        let ini = GameIni::load(dir.path()).unwrap();
        assert_eq!(ini.require::<i32>(key("iA")).unwrap(), 1);
        assert_eq!(ini.require::<i32>(key("iB")).unwrap(), 2);
    }

    #[test]
    fn load_survives_non_utf8_bytes_in_a_comment() {
        let dir = tempfile::tempdir().unwrap();
        write_required(dir.path());
        std::fs::write(
            dir.path().join("Fallout4Custom.ini"),
            b"; caf\xE9 comment\niAfter=7\n",
        )
        .unwrap();
        let ini = GameIni::load(dir.path()).unwrap();
        assert_eq!(ini.require::<i32>(key("iAfter")).unwrap(), 7);
    }

    #[test]
    fn load_needs_only_the_required_files() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Fallout4_Default.ini", "iA=1\n");
        write(dir.path(), "Fallout4Prefs.ini", "iB=2\n");
        let ini = GameIni::load(dir.path()).unwrap();
        assert_eq!(ini.require::<i32>(key("iA")).unwrap(), 1);
        let message = ini.require::<i32>(key("iMissing")).unwrap_err().to_string();
        assert!(
            message.contains("Fallout4_Default.ini, Fallout4Prefs.ini"),
            "{message}"
        );
        assert!(
            message.contains("not present: Fallout4.ini, Fallout4Custom.ini"),
            "{message}"
        );
    }

    #[test]
    fn load_fails_naming_each_missing_required_file() {
        for (present, missing) in [
            ("Fallout4Prefs.ini", "Fallout4_Default.ini"),
            ("Fallout4_Default.ini", "Fallout4Prefs.ini"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            write(dir.path(), present, "iA=1\n");
            write(dir.path(), "Fallout4.ini", "iA=1\n");
            let error = GameIni::load(dir.path()).err().unwrap();
            assert!(
                matches!(&error, IniError::MissingFile { name, dir: d } if *name == missing && d == dir.path()),
                "{error}"
            );
            assert!(error.to_string().contains(missing), "{error}");
        }
    }

    #[test]
    fn load_of_missing_directory_fails_on_the_first_required_file() {
        let dir = tempfile::tempdir().unwrap();
        let error = GameIni::load(&dir.path().join("nope")).err().unwrap();
        assert!(matches!(
            error,
            IniError::MissingFile {
                name: "Fallout4_Default.ini",
                ..
            }
        ));
    }

    #[test]
    fn load_fails_on_an_unreadable_cascade_entry_instead_of_skipping_it() {
        let dir = tempfile::tempdir().unwrap();
        write_required(dir.path());
        std::fs::create_dir(dir.path().join("Fallout4Custom.ini")).unwrap();
        let error = GameIni::load(dir.path()).err().unwrap();
        assert!(
            matches!(&error, IniError::Unreadable { path, .. } if path.ends_with("Fallout4Custom.ini")),
            "{error}"
        );
    }

    #[test]
    fn an_unreadable_file_reports_its_cause_once_as_the_error_source() {
        let dir = tempfile::tempdir().unwrap();
        write_required(dir.path());
        std::fs::create_dir(dir.path().join("Fallout4Custom.ini")).unwrap();
        let error = GameIni::load(dir.path()).err().unwrap();
        let IniError::Unreadable { source, .. } = &error else {
            panic!("{error}");
        };
        assert!(!error.to_string().contains(&source.to_string()), "{error}");
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn a_missing_required_file_names_the_installer_command_that_fixes_it() {
        let dir = tempfile::tempdir().unwrap();
        let error = GameIni::load(dir.path()).err().unwrap();
        assert!(error.to_string().contains("`inis-only`"), "{error}");
    }

    #[test]
    fn a_utf16_or_binary_file_is_an_encoding_error_naming_it() {
        let utf16_le_with_mark: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(
                "[Display]\niA=1\n"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes),
            )
            .collect();
        let utf16_le_without_mark: Vec<u8> =
            "iA=1\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let utf16_be_with_mark = [0xFE, 0xFF, 0x00, b'i'];
        for bytes in [
            utf16_le_with_mark.as_slice(),
            utf16_le_without_mark.as_slice(),
            utf16_be_with_mark.as_slice(),
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_required(dir.path());
            std::fs::write(dir.path().join("Fallout4Custom.ini"), bytes).unwrap();
            let error = GameIni::load(dir.path()).err().unwrap();
            assert!(
                matches!(&error, IniError::UnsupportedEncoding { path } if path.ends_with("Fallout4Custom.ini")),
                "{error}"
            );
            assert!(error.to_string().contains("UTF-16"), "{error}");
        }
    }

    #[test]
    fn a_utf8_byte_order_mark_is_not_an_encoding_error() {
        let dir = tempfile::tempdir().unwrap();
        write_required(dir.path());
        std::fs::write(
            dir.path().join("Fallout4Custom.ini"),
            b"\xEF\xBB\xBFiFirst=5\n",
        )
        .unwrap();
        let ini = GameIni::load(dir.path()).unwrap();
        assert_eq!(ini.require::<i32>(key("iFirst")).unwrap(), 5);
    }

    proptest! {
        #[test]
        fn arbitrary_text_never_panics_the_parser(text in "\\PC{0,200}") {
            let ini = GameIni::from_texts([text.as_str()]);
            let _ = ini.require::<i32>(key("anything"));
        }

        #[test]
        fn a_written_setting_reads_back_unchanged(
            written_name in prop_oneof![Just("iKey"), Just("IKEY"), Just("ikey")],
            value in "[A-Za-z0-9_./-]{1,20}",
            padding in "[ \t]{0,3}",
        ) {
            let text = format!("[S]\n{padding}{written_name}{padding}={padding}{value}{padding}\n");
            let ini = GameIni::from_texts([text.as_str()]);
            prop_assert_eq!(ini.require::<String>(KEY).unwrap(), value);
        }
    }
}
