use std::collections::{BTreeMap, HashMap};

use anyhow::Context;

use super::cell_lighting::{
    CellLightingError, RawLighting, ResolvedLighting, TemplateGap, TemplateSource, resolve,
};
use crate::assets::db::DbClient;
use crate::assets::db::cells::InteriorLightingRow;

pub type InteriorLightingEntry = Result<ResolvedLighting, CellLightingError>;

type TemplateEntry = Result<RawLighting, TemplateGap>;

/// Resolved lighting of every interior cell, keyed by cell form key (the key
/// `WorldStreamer::current_interior` returns).
pub struct InteriorLightingCatalog {
    entries: HashMap<String, InteriorLightingEntry>,
}

impl InteriorLightingCatalog {
    /// Boot-time entry point: a failure is logged and leaves interiors without
    /// a catalog, which [`Environment::derive`](super::environment::Environment::derive)
    /// answers with the exterior lighting.
    pub fn load_or_report(db_path: &str) -> Option<Self> {
        let db = match DbClient::new(db_path) {
            Ok(db) => db,
            Err(error) => {
                tracing::error!(
                    "interior lighting catalog not loaded, database open failed: {error:#}; interiors use exterior lighting"
                );
                return None;
            }
        };
        match Self::load(&db) {
            Ok(catalog) => Some(catalog),
            Err(error) => {
                tracing::error!(
                    "interior lighting catalog not loaded: {error:#}; interiors use exterior lighting"
                );
                None
            }
        }
    }

    pub fn load(db: &DbClient) -> anyhow::Result<Self> {
        let cells = db
            .get_interior_lighting()
            .context("reading interior cell lighting")?;
        if !cells.template_column_present {
            tracing::warn!(
                "cells.lighting_template column missing (database predates lighting template conversion); interior lighting uses cell values only"
            );
        }
        let templates = load_templates(db)?;

        let entries: HashMap<String, InteriorLightingEntry> = cells
            .rows
            .into_iter()
            .map(|row| {
                let entry = resolve_cell(&row, cells.template_column_present, &templates);
                (row.form_key, entry)
            })
            .collect();

        log_summary(&entries);
        Ok(Self { entries })
    }

    pub fn get(&self, form_key: &str) -> Option<&InteriorLightingEntry> {
        self.entries.get(form_key)
    }
}

fn load_templates(db: &DbClient) -> anyhow::Result<HashMap<String, TemplateEntry>> {
    let rows = db
        .get_lighting_templates()
        .context("reading lighting templates")?;
    let templates: HashMap<String, TemplateEntry> = rows
        .into_iter()
        .map(|row| {
            let entry = match RawLighting::parse(row.data.as_deref()) {
                Ok(raw) => Ok(raw),
                Err(CellLightingError::Absent) => Err(TemplateGap::NoData),
                Err(error) => {
                    tracing::warn!("lighting template {} unusable: {error}", row.form_key);
                    Err(TemplateGap::Malformed)
                }
            };
            (row.form_key, entry)
        })
        .collect();

    let usable = templates.values().filter(|entry| entry.is_ok()).count();
    let without_data = templates
        .values()
        .filter(|entry| matches!(entry, Err(TemplateGap::NoData)))
        .count();
    let malformed = templates
        .values()
        .filter(|entry| matches!(entry, Err(TemplateGap::Malformed)))
        .count();
    tracing::info!(
        "lighting templates: {usable} usable, {without_data} without data, {malformed} malformed"
    );
    Ok(templates)
}

fn resolve_cell(
    row: &InteriorLightingRow,
    template_column_present: bool,
    templates: &HashMap<String, TemplateEntry>,
) -> InteriorLightingEntry {
    let cell = RawLighting::parse(row.lighting.as_deref())?;
    let source = match (template_column_present, row.lighting_template.as_deref()) {
        (false, _) => TemplateSource::LinkUnknown,
        (true, None) => TemplateSource::Unlinked,
        (true, Some(form_key)) => match templates.get(form_key) {
            Some(Ok(raw)) => TemplateSource::Available(raw),
            Some(Err(gap)) => TemplateSource::Unavailable {
                form_key,
                gap: *gap,
            },
            None => TemplateSource::Unavailable {
                form_key,
                gap: TemplateGap::UnknownTemplate,
            },
        },
    };
    resolve(&cell, source)
}

struct OutcomeCount<'a> {
    cells: usize,
    first_form_key: &'a str,
}

fn log_summary(entries: &HashMap<String, InteriorLightingEntry>) {
    let mut outcomes: BTreeMap<&'static str, OutcomeCount> = BTreeMap::new();
    let mut failures = 0;
    for (form_key, entry) in entries {
        let label = match entry {
            Ok(resolved) => resolved.template.label(),
            Err(error) => {
                failures += 1;
                error.label()
            }
        };
        outcomes
            .entry(label)
            .and_modify(|count| count.cells += 1)
            .or_insert(OutcomeCount {
                cells: 1,
                first_form_key: form_key,
            });
    }

    let breakdown = outcomes
        .iter()
        .map(|(label, count)| format!("{label}: {} (e.g. {})", count.cells, count.first_form_key))
        .collect::<Vec<_>>()
        .join("; ");
    let message = format!(
        "interior lighting: {} cells, {failures} failed; {breakdown}",
        entries.len()
    );
    if failures > 0 {
        tracing::warn!("{message}");
    } else {
        tracing::info!("{message}");
    }
}
