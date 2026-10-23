use super::cell_lighting::ResolvedLighting;
use super::interior_catalog::InteriorLightingCatalog;

#[derive(Debug, Clone, PartialEq)]
pub enum NeutralReason {
    NotInCatalog,
    Unresolvable { label: &'static str, detail: String },
}

impl NeutralReason {
    pub fn label(&self) -> &'static str {
        match self {
            Self::NotInCatalog => "cell not in lighting catalog",
            Self::Unresolvable { label, .. } => label,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InteriorLighting {
    Cell(ResolvedLighting),
    Neutral(NeutralReason),
    CatalogUnavailable,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "Environment is a single long-lived value; boxing the interior payload would change the public variant shape"
)]
pub enum Environment {
    Exterior,
    Interior {
        cell: String,
        lighting: InteriorLighting,
    },
}

impl Environment {
    pub fn cell_key(&self) -> Option<&str> {
        match self {
            Self::Exterior => None,
            Self::Interior { cell, .. } => Some(cell),
        }
    }

    pub fn derive(interior: Option<&str>, catalog: Option<&InteriorLightingCatalog>) -> Self {
        let Some(cell) = interior else {
            return Self::Exterior;
        };
        let lighting = match catalog.map(|catalog| catalog.get(cell)) {
            None => InteriorLighting::CatalogUnavailable,
            Some(None) => InteriorLighting::Neutral(NeutralReason::NotInCatalog),
            Some(Some(Ok(resolved))) => InteriorLighting::Cell(resolved.clone()),
            Some(Some(Err(error))) => InteriorLighting::Neutral(NeutralReason::Unresolvable {
                label: error.label(),
                detail: error.to_string(),
            }),
        };
        Self::Interior {
            cell: cell.to_string(),
            lighting,
        }
    }

    pub fn log_entered(&self) {
        match self {
            Self::Exterior => tracing::info!("lighting environment: exterior"),
            Self::Interior { cell, lighting } => match lighting {
                InteriorLighting::Cell(resolved) => tracing::info!(
                    "lighting environment: interior {cell} ({})",
                    resolved.template.label()
                ),
                InteriorLighting::Neutral(NeutralReason::NotInCatalog) => tracing::warn!(
                    "lighting environment: interior {cell} is not in the lighting catalog; neutral fallback lighting"
                ),
                InteriorLighting::Neutral(NeutralReason::Unresolvable { label, detail }) => {
                    tracing::warn!(
                        "lighting environment: interior {cell} lighting unusable ({label}: {detail}); neutral fallback lighting"
                    );
                }
                InteriorLighting::CatalogUnavailable => tracing::warn!(
                    "lighting environment: interior {cell} has no lighting catalog; exterior lighting"
                ),
            },
        }
    }
}
