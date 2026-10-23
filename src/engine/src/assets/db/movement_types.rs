//! MOVT movement types. The installer serializes the full SPED payload into
//! `movement_types.data` as JSON ({Left,Right,Forward,Back,Pitch,Roll,Yaw} ×
//! {Standing,Walk,Run,Sprint}, plus `FloatHeight` / `FlightAngleGain`). Speeds are
//! Bethesda units/s, rotation rates deg/s. Only the fields the sim consumes
//! are surfaced here; widen as needed.

pub struct MovementType {
    pub forward_walk: f32,
    pub forward_run: f32,
    pub forward_sprint: f32,
}

impl MovementType {
    pub fn get(editor_id: &str, conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        let data: String = conn.query_row(
            "SELECT data FROM movement_types WHERE editor_id = ?",
            rusqlite::params![editor_id],
            |row| row.get(0),
        )?;
        let v: serde_json::Value = serde_json::from_str(&data).map_err(|e| {
            anyhow::anyhow!("movement_types '{editor_id}': data is not valid JSON: {e}")
        })?;
        let f = |key: &str| -> anyhow::Result<f32> {
            v.get(key)
                .and_then(serde_json::Value::as_f64)
                .map(|x| x as f32)
                .ok_or_else(|| {
                    anyhow::anyhow!("movement_types '{editor_id}': data lacks field {key}")
                })
        };
        Ok(Self {
            forward_walk: f("ForwardWalk")?,
            forward_run: f("ForwardRun")?,
            forward_sprint: f("ForwardSprint")?,
        })
    }
}
