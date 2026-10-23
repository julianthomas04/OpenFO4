//! Every Pip-Boy screen, registered once so the shell can route to the active one.

pub(crate) mod data_quests;
pub(crate) mod data_stats;
pub(crate) mod data_workshops;
mod glyph;
pub(crate) mod inv;
mod map;
mod perk_chart;
mod perk_chart_layout;
pub(crate) mod radio;
mod radio_wave;
pub(crate) mod stat_perks;
pub(crate) mod stat_special;
pub(crate) mod stat_status;
#[cfg(test)]
mod test_harness;
pub mod vault_boy;

use super::nav::{SubTab, TabId};
use super::tab::Tab;
use crate::sim::pipboy::InventoryCategory;
use crate::ui_layout::{KeyGroup, UiLayout, UiLayoutError};

/// Every layout key the pages read, resolved at boot.
pub static LAYOUT_KEYS: KeyGroup = KeyGroup(&[
    &stat_special::LAYOUT_KEYS,
    &stat_perks::LAYOUT_KEYS,
    &inv::LAYOUT_KEYS,
    &radio::LAYOUT_KEYS,
    &data_workshops::LAYOUT_KEYS,
    &data_stats::LAYOUT_KEYS,
    &data_quests::LAYOUT_KEYS,
    &map::LAYOUT_KEYS,
    &map::PULSE_KEYS,
    &stat_status::LAYOUT_KEYS,
]);

pub use data_quests::{QuestsLayout, QuestsPage};
pub use data_stats::{StatsLayout, StatsPage};
pub use data_workshops::{WorkshopsLayout, WorkshopsPage};
pub use inv::{InventoryLayout, InventoryPage};
pub use map::{MapLayout, MapPage, PlayerPulse};
pub use perk_chart::{LAYOUT_KEYS as PERK_CHART_KEYS, MOVIE as PERK_CHART_MOVIE, PerkChartPage};
#[cfg(test)]
pub use perk_chart::{LEVEL_UP_TRANSLATIONS, level_up_menu_movie};
pub use radio::{RadioLayout, RadioPage};
pub use stat_perks::{PerksLayout, PerksPage};
pub use stat_special::{SpecialLayout, SpecialPage};
pub use stat_status::StatusPage;

/// All screens of the shell and the perk chart.
pub struct Pages {
    status: StatusPage,
    special: SpecialPage,
    perks: PerksPage,
    inventory: [InventoryPage; InventoryCategory::ALL.len()],
    quests: QuestsPage,
    workshops: WorkshopsPage,
    stats: StatsPage,
    map: MapPage,
    radio: RadioPage,
    perk_chart: PerkChartPage,
}

impl Pages {
    #[allow(
        clippy::default_constructed_unit_structs,
        reason = "the placeholder pages gain state when implemented"
    )]
    pub fn new(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let inventory_layout = std::sync::Arc::new(InventoryLayout::resolve(layout)?);
        Ok(Self {
            status: StatusPage::new(stat_status::StatusLayout::resolve(layout)?),
            special: SpecialPage::new(SpecialLayout::resolve(layout)?),
            perks: PerksPage::new(PerksLayout::resolve(layout)?),
            inventory: InventoryCategory::ALL.map(|category| {
                InventoryPage::new(category, std::sync::Arc::clone(&inventory_layout))
            }),
            quests: QuestsPage::new(QuestsLayout::resolve(layout)?),
            workshops: WorkshopsPage::new(WorkshopsLayout::resolve(layout)?),
            stats: StatsPage::new(StatsLayout::resolve(layout)?),
            map: MapPage::new(MapLayout::resolve(layout)?, PlayerPulse::resolve(layout)?),
            radio: RadioPage::new(RadioLayout::resolve(layout)?),
            perk_chart: PerkChartPage::load(layout)?,
        })
    }

    pub fn tab_mut(&mut self, tab: TabId) -> &mut dyn Tab {
        match tab {
            TabId::Sub(SubTab::Status) => &mut self.status,
            TabId::Sub(SubTab::Special) => &mut self.special,
            TabId::Sub(SubTab::Perks) => &mut self.perks,
            TabId::Sub(SubTab::Weapons) => &mut self.inventory[InventoryCategory::Weapons as usize],
            TabId::Sub(SubTab::Apparel) => &mut self.inventory[InventoryCategory::Apparel as usize],
            TabId::Sub(SubTab::Aid) => &mut self.inventory[InventoryCategory::Aid as usize],
            TabId::Sub(SubTab::Misc) => &mut self.inventory[InventoryCategory::Misc as usize],
            TabId::Sub(SubTab::Junk) => &mut self.inventory[InventoryCategory::Junk as usize],
            TabId::Sub(SubTab::Mods) => &mut self.inventory[InventoryCategory::Mods as usize],
            TabId::Sub(SubTab::Ammo) => &mut self.inventory[InventoryCategory::Ammo as usize],
            TabId::Sub(SubTab::Quests) => &mut self.quests,
            TabId::Sub(SubTab::Workshops) => &mut self.workshops,
            TabId::Sub(SubTab::Stats) => &mut self.stats,
            TabId::Map => &mut self.map,
            TabId::Radio => &mut self.radio,
        }
    }

    /// Centres the map on an engine-space position (the DATA "show on map" action).
    pub fn focus_map(&mut self, position: glam::Vec3) {
        self.map.focus_on(position);
    }

    /// Tells every screen the Pip-Boy just opened.
    pub fn on_open(&mut self) {
        let Self {
            status,
            special,
            perks,
            inventory,
            quests,
            workshops,
            stats,
            map,
            radio,
            perk_chart,
        } = self;
        let mut tabs: Vec<&mut dyn Tab> = vec![
            status, special, perks, quests, workshops, stats, map, radio, perk_chart,
        ];
        tabs.extend(inventory.iter_mut().map(|page| page as &mut dyn Tab));
        for tab in tabs {
            tab.on_open();
        }
    }

    pub fn perk_chart_mut(&mut self) -> &mut PerkChartPage {
        &mut self.perk_chart
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::pipboy::nav::Page;

    #[test]
    fn every_tab_of_every_page_resolves_to_a_screen() {
        let mut pages =
            Pages::new(&crate::rendering::pipboy::test_layout::pipboy_layout()).unwrap();
        for page in Page::ALL {
            match page.sub_tabs() {
                [] => {
                    pages.tab_mut(if page == Page::Map {
                        TabId::Map
                    } else {
                        TabId::Radio
                    });
                }
                subs => {
                    for sub in subs {
                        pages.tab_mut(TabId::Sub(*sub));
                    }
                }
            }
        }
        pages.perk_chart_mut();
    }

    #[test]
    fn the_inventory_sub_tabs_resolve_to_their_own_category_page() {
        let mut pages =
            Pages::new(&crate::rendering::pipboy::test_layout::pipboy_layout()).unwrap();
        for (sub, category) in [
            (SubTab::Weapons, InventoryCategory::Weapons),
            (SubTab::Junk, InventoryCategory::Junk),
            (SubTab::Ammo, InventoryCategory::Ammo),
        ] {
            let bar = pages.tab_mut(TabId::Sub(sub)).bottom_bar();
            assert_eq!(
                bar,
                crate::rendering::pipboy::tab::BottomBar::Inventory(category)
            );
        }
    }
}
