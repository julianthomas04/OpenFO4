//! Play statistics shown on DATA/STATS.

#[derive(Clone, Debug, PartialEq)]
pub struct StatEntry {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatCategory {
    pub name: String,
    pub entries: Vec<StatEntry>,
}
