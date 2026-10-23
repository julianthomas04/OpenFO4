//! Chooses the playing music type among the applicable ones.
//!
//! Pure priority stack: the lowest music type priority number plays and the more specific source
//! (cell, then location, then region, then worldspace) breaks ties. The vanilla precedence between
//! the sources is undocumented, so this is a chosen rule.

use super::place::Candidate;

/// Declaration order is specificity order: the derived `Ord` ranks the most specific source lowest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlaceSource {
    Cell,
    Location,
    Region,
    Worldspace,
}

pub fn pick(candidates: &[Candidate]) -> Option<Candidate> {
    candidates
        .iter()
        .min_by_key(|candidate| (candidate.priority, candidate.source))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(music: &str, source: PlaceSource, priority: u8) -> Candidate {
        Candidate {
            music_type: music.to_string(),
            source,
            priority,
        }
    }

    #[test]
    fn lowest_priority_number_wins_over_specificity() {
        let candidates = [
            candidate("MUSzExplore", PlaceSource::Worldspace, 90),
            candidate("FoothillsSanctuary", PlaceSource::Region, 80),
            candidate("MUSzPublic", PlaceSource::Cell, 100),
        ];
        assert_eq!(pick(&candidates).unwrap().music_type, "FoothillsSanctuary");
    }

    #[test]
    fn equal_priority_goes_to_the_more_specific_source() {
        let candidates = [
            candidate("MUSzExplore", PlaceSource::Worldspace, 80),
            candidate("MUSDungeonA", PlaceSource::Location, 80),
            candidate("FoothillsSanctuary", PlaceSource::Region, 80),
        ];
        assert_eq!(pick(&candidates).unwrap().music_type, "MUSDungeonA");
    }

    #[test]
    fn no_candidates_means_no_pick() {
        assert_eq!(pick(&[]), None);
    }
}
