/// DOOR record flag "Automatic": touching the door loads its destination.
const FLAG_AUTOMATIC: u32 = 0x02;

#[derive(Debug, thiserror::Error)]
pub enum DoorFlagsError {
    #[error("{subject} has no door flags value")]
    Missing { subject: String },
    #[error("{subject}: door flags {flags:?} are not a number")]
    Malformed { subject: String, flags: String },
}

/// Whether a door's `flags` column (a decimal number as text) has Automatic set;
/// `subject` names the door or ref in errors.
pub fn flags_are_automatic(subject: &str, flags: Option<&str>) -> Result<bool, DoorFlagsError> {
    let flags = flags.ok_or_else(|| DoorFlagsError::Missing {
        subject: subject.to_string(),
    })?;
    let value: u32 = flags
        .trim()
        .parse()
        .map_err(|_| DoorFlagsError::Malformed {
            subject: subject.to_string(),
            flags: flags.to_string(),
        })?;
    Ok(value & FLAG_AUTOMATIC != 0)
}

pub struct Door {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub name: Option<String>,
    pub model_path: Option<String>,
    pub open_sound: Option<String>,
    pub close_sound: Option<String>,
    pub flags: Option<String>,
    pub is_marker: bool,
}

fn row_to_door(row: &rusqlite::Row) -> rusqlite::Result<Door> {
    Ok(Door {
        form_key: row.get(0)?,
        editor_id: row.get(1)?,
        name: row.get(2)?,
        model_path: row.get(3)?,
        open_sound: row.get(4)?,
        close_sound: row.get(5)?,
        flags: row.get(6)?,
        is_marker: row.get(7)?,
    })
}

impl Door {
    pub fn is_automatic(&self) -> Result<bool, DoorFlagsError> {
        flags_are_automatic(
            self.form_key.as_deref().unwrap_or("door without form key"),
            self.flags.as_deref(),
        )
    }

    pub fn get(form_key: &str, conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        Ok(conn.query_row(
            "SELECT form_key, editor_id, name, model_path,
                    open_sound, close_sound, flags, is_marker
             FROM doors WHERE form_key = ?",
            rusqlite::params![form_key],
            row_to_door,
        )?)
    }

    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, editor_id, name, model_path,
                    open_sound, close_sound, flags, is_marker
             FROM doors",
        )?;

        Ok(stmt
            .query_map([], row_to_door)?
            .collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_is_bit_two_of_the_decimal_flags() {
        assert!(flags_are_automatic("d", Some("2")).unwrap());
        assert!(flags_are_automatic("d", Some("66")).unwrap());
        assert!(!flags_are_automatic("d", Some("64")).unwrap());
        assert!(!flags_are_automatic("d", Some("0")).unwrap());
    }

    #[test]
    fn missing_or_malformed_flags_are_errors_naming_the_subject() {
        let missing = flags_are_automatic("door A", None).unwrap_err();
        assert!(missing.to_string().contains("door A"), "{missing}");
        let malformed = flags_are_automatic("door B", Some("two")).unwrap_err();
        assert!(malformed.to_string().contains("door B"), "{malformed}");
    }
}
