//! `items` rows joined with their `weapons` / `armors` extension rows.

#[derive(Clone, Debug, PartialEq)]
pub struct WeaponRow {
    pub damage: Option<i64>,
    pub fire_rate: Option<f64>,
    pub range: Option<f64>,
    pub accuracy: Option<f64>,
    pub ammo_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArmorRow {
    pub armor_rating: Option<i64>,
    pub body_slot: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemRow {
    pub form_key: String,
    /// Record signature: `WEAP`, `ARMO`, `AMMO`, `MISC`, `BOOK`, `ALCH`, `KEYM`, ...
    pub item_type: String,
    pub editor_id: Option<String>,
    pub name: Option<String>,
    pub value: i64,
    pub weight: f64,
    pub weapon: Option<WeaponRow>,
    pub armor: Option<ArmorRow>,
}

impl ItemRow {
    pub fn list_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT i.form_key, i.item_type, i.editor_id, i.name, i.value, i.weight,
                    w.form_key IS NOT NULL, w.damage, w.fire_rate, w.range, w.accuracy, w.ammo_type,
                    a.form_key IS NOT NULL, a.armor_rating, a.body_slot
             FROM items i
             LEFT JOIN weapons w ON w.form_key = i.form_key
             LEFT JOIN armors a ON a.form_key = i.form_key
             ORDER BY i.form_key",
        )?;
        Ok(stmt
            .query_map([], |row| {
                let weapon = if row.get::<_, bool>(6)? {
                    Some(WeaponRow {
                        damage: row.get(7)?,
                        fire_rate: row.get(8)?,
                        range: row.get(9)?,
                        accuracy: row.get(10)?,
                        ammo_type: row.get(11)?,
                    })
                } else {
                    None
                };
                let armor = if row.get::<_, bool>(12)? {
                    Some(ArmorRow {
                        armor_rating: row.get(13)?,
                        body_slot: row.get(14)?,
                    })
                } else {
                    None
                };
                Ok(Self {
                    form_key: row.get(0)?,
                    item_type: row.get(1)?,
                    editor_id: row.get(2)?,
                    name: row.get(3)?,
                    value: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    weight: row.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                    weapon,
                    armor,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
