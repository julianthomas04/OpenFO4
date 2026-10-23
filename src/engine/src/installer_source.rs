//! The `const` declarations of the installer's C# sources, read by tests that hold the engine's
//! copy of an install contract value to the installer's.

const MODIFIERS: [&str; 4] = ["public", "internal", "private", "protected"];

/// The name and value of every one-line `const` declaration in `source`; a string value is
/// given without its quotes.
pub fn constants(source: &str) -> impl Iterator<Item = (&str, &str)> {
    source.lines().filter_map(declaration)
}

/// The value of the one `const` declaration named `name` in `source`.
pub fn constant<'a>(source: &'a str, name: &str) -> &'a str {
    let mut values = constants(source)
        .filter(|&(declared, _)| declared == name)
        .map(|(_, value)| value);
    let value = values
        .next()
        .unwrap_or_else(|| panic!("the installer source declares no const {name}"));
    assert!(
        values.next().is_none(),
        "the installer source declares const {name} more than once"
    );
    value
}

fn declaration(line: &str) -> Option<(&str, &str)> {
    let mut words = line.trim_start();
    loop {
        let (word, rest) = words.split_once(' ')?;
        if word == "const" {
            words = rest;
            break;
        }
        if !MODIFIERS.contains(&word) {
            return None;
        }
        words = rest;
    }
    let (_type, assignment) = words.split_once(' ')?;
    let (name, value) = assignment.split_once('=')?;
    let (value, _) = value.split_once(';')?;
    let value = value.trim();
    let unquoted = value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'));
    Some((name.trim(), unquoted.unwrap_or(value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
namespace Example
{
    public static class Values
    {
        public const string Folder = "assets";
        internal const int Version = 3;
        const uint Magic = 0x20;  // trailing comment
        private static readonly int NotConst = 4;
        public static string Describe() => "const Hidden = 5;";
    }
}
"#;

    #[test]
    fn every_const_declaration_is_read_with_string_quotes_removed() {
        assert_eq!(
            constants(SOURCE).collect::<Vec<_>>(),
            [("Folder", "assets"), ("Version", "3"), ("Magic", "0x20")]
        );
    }

    #[test]
    fn a_constant_is_found_by_its_name() {
        assert_eq!(constant(SOURCE, "Version"), "3");
    }

    #[test]
    #[should_panic(expected = "declares no const Missing")]
    fn a_missing_constant_fails() {
        constant(SOURCE, "Missing");
    }

    #[test]
    #[should_panic(expected = "more than once")]
    fn an_ambiguous_constant_fails() {
        constant("const int A = 1;\nconst int A = 2;", "A");
    }
}
