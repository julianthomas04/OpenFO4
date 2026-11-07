//! A worldspace's parent chain and the `PNAM` bits that say which of its data
//! the worldspace takes from the parent instead of its own records.

use std::collections::HashSet;

/// `PNAM` bits, by the data they defer to the parent.
pub mod parent_use {
    /// LAND records and the `DNAM` default land and water heights.
    pub const LAND_DATA: u16 = 0x01;
    /// Distant LOD data: the LOD water form (`NAM3`) and the LOD terrain tiles.
    pub const LOD_DATA: u16 = 0x02;
    /// The water form (`NAM2`).
    pub const WATER_DATA: u16 = 0x08;
}

/// A worldspace parent chain this long is a cycle in the data.
const MAX_WORLDSPACE_DEPTH: usize = 8;

/// One worldspace of a parent chain.
pub trait WorldspaceLink {
    fn parent(&self) -> Option<&str>;
    /// The `PNAM` bits.
    fn parent_flags(&self) -> u16;
}

/// The bare link facts of a worldspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentLink {
    pub form_key: String,
    pub parent: Option<String>,
    pub parent_flags: u16,
}

impl WorldspaceLink for ParentLink {
    fn parent(&self) -> Option<&str> {
        self.parent.as_deref()
    }

    fn parent_flags(&self) -> u16 {
        self.parent_flags
    }
}

/// The worldspace and its parents, nearest first, each fetched with `fetch`.
/// The worldspace itself must exist; a parent that cannot be fetched only ends
/// the chain, logged once per worldspace and parent. A chain that loops or runs deeper than the depth cap is an error.
pub fn walk_parents<L: WorldspaceLink>(
    worldspace: &str,
    mut fetch: impl FnMut(&str) -> anyhow::Result<L>,
) -> anyhow::Result<Vec<L>> {
    let mut links: Vec<L> = Vec::new();
    let mut seen = HashSet::new();
    let mut next = Some(worldspace.to_owned());
    while let Some(key) = next.take() {
        anyhow::ensure!(
            seen.insert(key.clone()) && seen.len() <= MAX_WORLDSPACE_DEPTH,
            "worldspace parent chain of {worldspace} loops at {key}"
        );
        match fetch(&key) {
            Ok(link) => {
                next = link.parent().map(str::to_owned);
                links.push(link);
            }
            Err(error) if !links.is_empty() => {
                crate::utils::warn_once(&format!("worldspace-parent:{worldspace}:{key}"), || {
                    format!("worldspace lineage: parent {key} of {worldspace}: {error:#}")
                });
            }
            Err(error) => return Err(error),
        }
    }
    Ok(links)
}

/// Index of the link that keeps its own data for `bit`: the first one whose
/// `PNAM` does not defer to its parent, or the last link when the chain ends
/// while every link still defers.
pub fn owner_index<L: WorldspaceLink>(links: &[L], bit: u16) -> usize {
    let deferring = links
        .iter()
        .take_while(|link| link.parent_flags() & bit != 0)
        .count();
    deferring.min(links.len().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(name: &str, parent: Option<&str>, flags: u16) -> ParentLink {
        ParentLink {
            form_key: name.to_owned(),
            parent: parent.map(str::to_owned),
            parent_flags: flags,
        }
    }

    fn fetch_from(links: Vec<ParentLink>) -> impl FnMut(&str) -> anyhow::Result<ParentLink> {
        move |key| {
            links
                .iter()
                .find(|link| link.form_key == key)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no worldspace {key}"))
        }
    }

    fn names(links: &[ParentLink]) -> Vec<&str> {
        links.iter().map(|link| link.form_key.as_str()).collect()
    }

    #[test]
    fn the_walk_lists_the_worldspace_then_its_parents_nearest_first() {
        let links = walk_parents(
            "a",
            fetch_from(vec![
                link("c", None, 0),
                link("a", Some("b"), 0),
                link("b", Some("c"), 0),
            ]),
        )
        .unwrap();
        assert_eq!(names(&links), ["a", "b", "c"]);
    }

    #[test]
    fn a_worldspace_without_parent_is_a_chain_of_one() {
        let links = walk_parents("a", fetch_from(vec![link("a", None, 0)])).unwrap();
        assert_eq!(names(&links), ["a"]);
    }

    #[test]
    fn a_missing_parent_only_ends_the_chain() {
        let links = walk_parents("a", fetch_from(vec![link("a", Some("gone"), 0xFF)])).unwrap();
        assert_eq!(names(&links), ["a"]);
    }

    #[test]
    fn a_missing_worldspace_is_an_error() {
        let error = walk_parents("a", fetch_from(Vec::new())).unwrap_err();
        assert!(error.to_string().contains("no worldspace a"), "{error}");
    }

    #[test]
    fn a_parent_loop_is_an_error_naming_where_it_loops() {
        let error = walk_parents(
            "a",
            fetch_from(vec![link("a", Some("b"), 0), link("b", Some("a"), 0)]),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("loops at a"), "{error}");
    }

    #[test]
    fn a_chain_deeper_than_the_cap_is_an_error() {
        let chain = |length: usize| {
            (0..length)
                .map(|index| {
                    let parent = (index + 1 < length).then(|| format!("w{}", index + 1));
                    link(&format!("w{index}"), parent.as_deref(), 0)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            walk_parents("w0", fetch_from(chain(MAX_WORLDSPACE_DEPTH)))
                .unwrap()
                .len(),
            MAX_WORLDSPACE_DEPTH
        );
        let error = walk_parents("w0", fetch_from(chain(MAX_WORLDSPACE_DEPTH + 1)))
            .unwrap_err()
            .to_string();
        assert!(error.contains("loops at"), "{error}");
    }

    #[test]
    fn the_owner_is_the_first_link_that_does_not_defer_for_the_bit() {
        let links = [
            link("a", Some("b"), parent_use::LAND_DATA),
            link("b", Some("c"), parent_use::LAND_DATA),
            link("c", None, 0),
        ];
        assert_eq!(owner_index(&links, parent_use::LAND_DATA), 2);
        let links = [
            link("a", Some("b"), parent_use::LAND_DATA),
            link("b", None, 0),
        ];
        assert_eq!(owner_index(&links, parent_use::LAND_DATA), 1);
    }

    #[test]
    fn a_link_without_the_bit_owns_its_data_even_when_other_bits_are_set() {
        let links = [
            link("a", Some("b"), parent_use::WATER_DATA),
            link("b", None, 0),
        ];
        assert_eq!(owner_index(&links, parent_use::LAND_DATA), 0);
        assert_eq!(owner_index(&links, parent_use::WATER_DATA), 1);
    }

    #[test]
    fn a_chain_that_ends_while_still_deferring_is_owned_by_its_last_link() {
        let links = [link("only", Some("missing"), 0xFF)];
        assert_eq!(owner_index(&links, parent_use::LOD_DATA), 0);
        let links = [link("a", Some("b"), 0xFF), link("b", Some("gone"), 0xFF)];
        assert_eq!(owner_index(&links, parent_use::LOD_DATA), 1);
    }
}
