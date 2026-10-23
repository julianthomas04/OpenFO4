//! Where the Pip-Boy screen is: page, sub-tab, and the full-screen perk chart.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Stat,
    Inv,
    Data,
    Map,
    Radio,
}

impl Page {
    pub const ALL: [Page; 5] = [Page::Stat, Page::Inv, Page::Data, Page::Map, Page::Radio];

    pub fn label(self) -> &'static str {
        match self {
            Page::Stat => "STAT",
            Page::Inv => "INV",
            Page::Data => "DATA",
            Page::Map => "MAP",
            Page::Radio => "RADIO",
        }
    }

    pub fn sub_tabs(self) -> &'static [SubTab] {
        match self {
            Page::Stat => &[SubTab::Status, SubTab::Special, SubTab::Perks],
            Page::Inv => &[
                SubTab::Weapons,
                SubTab::Apparel,
                SubTab::Aid,
                SubTab::Misc,
                SubTab::Junk,
                SubTab::Mods,
                SubTab::Ammo,
            ],
            Page::Data => &[SubTab::Quests, SubTab::Workshops, SubTab::Stats],
            Page::Map | Page::Radio => &[],
        }
    }

    fn name(self) -> &'static str {
        match self {
            Page::Stat => "stat",
            Page::Inv => "inv",
            Page::Data => "data",
            Page::Map => "map",
            Page::Radio => "radio",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubTab {
    Status,
    Special,
    Perks,
    Weapons,
    Apparel,
    Aid,
    Misc,
    Junk,
    Mods,
    Ammo,
    Quests,
    Workshops,
    Stats,
}

impl SubTab {
    pub fn label(self) -> &'static str {
        match self {
            SubTab::Status => "STATUS",
            SubTab::Special => "SPECIAL",
            SubTab::Perks => "PERKS",
            SubTab::Weapons => "WEAPONS",
            SubTab::Apparel => "APPAREL",
            SubTab::Aid => "AID",
            SubTab::Misc => "MISC",
            SubTab::Junk => "JUNK",
            SubTab::Mods => "MODS",
            SubTab::Ammo => "AMMO",
            SubTab::Quests => "QUESTS",
            SubTab::Workshops => "WORKSHOPS",
            SubTab::Stats => "STATS",
        }
    }
}

/// One screen of the shell. MAP and RADIO have no sub-tab row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TabId {
    Sub(SubTab),
    Map,
    Radio,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Screen {
    Shell(TabId),
    PerkChart,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "'{0}' is not a Pip-Boy screen; expected <page>[/<subtab>] (e.g. stat/perks, inv/aid, map, radio) or perkchart"
)]
pub struct UnknownScreen(pub String);

/// Parses an `OPENFO4_PIPBOY_OPEN` value: `stat/status`, `inv/junk`, `data/quests`, `map`, `radio`,
/// `perkchart`; a bare page opens its first sub-tab. Case-insensitive.
pub fn parse_screen(text: &str) -> Result<Screen, UnknownScreen> {
    let unknown = || UnknownScreen(text.to_owned());
    let lowered = text.trim().to_ascii_lowercase();
    if lowered == "perkchart" {
        return Ok(Screen::PerkChart);
    }
    let (page_name, sub_name) = match lowered.split_once('/') {
        Some((page, sub)) => (page, Some(sub)),
        None => (lowered.as_str(), None),
    };
    let page = Page::ALL
        .into_iter()
        .find(|page| page.name() == page_name)
        .ok_or_else(unknown)?;
    let tab = match (page.sub_tabs(), sub_name) {
        (_, None) => page_tab(page, 0),
        ([], Some(_)) => return Err(unknown()),
        (subs, Some(name)) => {
            let index = subs
                .iter()
                .position(|sub| sub.label().eq_ignore_ascii_case(name))
                .ok_or_else(unknown)?;
            page_tab(page, index)
        }
    };
    Ok(Screen::Shell(tab))
}

fn page_tab(page: Page, sub_index: usize) -> TabId {
    match page {
        Page::Map => TabId::Map,
        Page::Radio => TabId::Radio,
        Page::Stat | Page::Inv | Page::Data => TabId::Sub(page.sub_tabs()[sub_index]),
    }
}

/// Which way the sub-tab or page selection moved, for the strip's slide animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Previous,
    Next,
}

/// The navigation state machine: current page, the remembered sub-tab of every page, and whether
/// the perk chart covers the shell. Sub-tab and page selection clamp at their ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nav {
    page: Page,
    sub_index: [usize; Page::ALL.len()],
    perk_chart: bool,
}

impl Default for Nav {
    fn default() -> Self {
        Self {
            page: Page::Stat,
            sub_index: [0; Page::ALL.len()],
            perk_chart: false,
        }
    }
}

impl Nav {
    pub fn page(&self) -> Page {
        self.page
    }

    pub fn sub_index(&self) -> usize {
        self.sub_index[Self::page_slot(self.page)]
    }

    pub fn tab(&self) -> TabId {
        page_tab(self.page, self.sub_index())
    }

    pub fn screen(&self) -> Screen {
        if self.perk_chart {
            Screen::PerkChart
        } else {
            Screen::Shell(self.tab())
        }
    }

    pub fn open(&mut self, screen: Screen) {
        match screen {
            Screen::PerkChart => self.perk_chart = true,
            Screen::Shell(tab) => {
                self.perk_chart = false;
                self.show_tab(tab);
            }
        }
    }

    pub fn open_perk_chart(&mut self) {
        self.perk_chart = true;
    }

    pub fn close_perk_chart(&mut self) {
        self.perk_chart = false;
    }

    pub fn select_page(&mut self, page: Page) -> bool {
        let changed = page != self.page;
        self.page = page;
        changed
    }

    /// Moves one page toward `step`; `None` at the end of the row.
    pub fn step_page(&mut self, step: Step) -> Option<Step> {
        let index = Page::ALL.iter().position(|p| *p == self.page)?;
        let target = step_index(index, Page::ALL.len(), step)?;
        self.page = Page::ALL[target];
        Some(step)
    }

    pub fn select_sub(&mut self, index: usize) -> bool {
        let count = self.page.sub_tabs().len();
        let slot = Self::page_slot(self.page);
        let changed = index < count && self.sub_index[slot] != index;
        if changed {
            self.sub_index[slot] = index;
        }
        changed
    }

    /// Moves one sub-tab toward `step`; `None` at the end of the row or on a page without sub-tabs.
    pub fn step_sub(&mut self, step: Step) -> Option<Step> {
        let count = self.page.sub_tabs().len();
        let target = step_index(self.sub_index(), count, step)?;
        let slot = Self::page_slot(self.page);
        self.sub_index[slot] = target;
        Some(step)
    }

    fn show_tab(&mut self, tab: TabId) {
        let page = match tab {
            TabId::Map => Page::Map,
            TabId::Radio => Page::Radio,
            TabId::Sub(sub) => Page::ALL
                .into_iter()
                .find(|page| page.sub_tabs().contains(&sub))
                .expect("every sub-tab belongs to a page"),
        };
        self.page = page;
        if let TabId::Sub(sub) = tab {
            let index = page.sub_tabs().iter().position(|s| *s == sub);
            self.sub_index[Self::page_slot(page)] =
                index.expect("the sub-tab is listed by its page");
        }
    }

    fn page_slot(page: Page) -> usize {
        Page::ALL
            .iter()
            .position(|p| *p == page)
            .expect("every page is listed")
    }
}

fn step_index(index: usize, count: usize, step: Step) -> Option<usize> {
    match step {
        Step::Previous => index.checked_sub(1),
        Step::Next => (index + 1 < count).then_some(index + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_documented_form_parses() {
        let cases = [
            ("stat/status", Screen::Shell(TabId::Sub(SubTab::Status))),
            ("stat", Screen::Shell(TabId::Sub(SubTab::Status))),
            ("INV/Aid", Screen::Shell(TabId::Sub(SubTab::Aid))),
            ("inv", Screen::Shell(TabId::Sub(SubTab::Weapons))),
            ("inv/junk", Screen::Shell(TabId::Sub(SubTab::Junk))),
            (
                "data/workshops",
                Screen::Shell(TabId::Sub(SubTab::Workshops)),
            ),
            ("data", Screen::Shell(TabId::Sub(SubTab::Quests))),
            ("map", Screen::Shell(TabId::Map)),
            ("radio", Screen::Shell(TabId::Radio)),
            ("perkchart", Screen::PerkChart),
            ("  Stat/Perks ", Screen::Shell(TabId::Sub(SubTab::Perks))),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_screen(text), Ok(expected), "{text}");
        }
    }

    #[test]
    fn unknown_pages_sub_tabs_and_sub_tabs_of_page_with_none_are_errors() {
        for text in [
            "",
            "stats",
            "stat/aid",
            "map/world",
            "radio/x",
            "perkchart/1",
            "/",
        ] {
            assert_eq!(
                parse_screen(text),
                Err(UnknownScreen(text.to_owned())),
                "{text}"
            );
        }
    }

    #[test]
    fn the_error_names_the_offending_text() {
        let message = parse_screen("bogus").unwrap_err().to_string();
        assert!(message.contains("'bogus'"));
    }

    #[test]
    fn each_page_remembers_its_own_sub_tab() {
        let mut nav = Nav::default();
        nav.step_sub(Step::Next);
        nav.select_page(Page::Inv);
        assert_eq!(nav.tab(), TabId::Sub(SubTab::Weapons));
        nav.step_sub(Step::Next);
        nav.step_sub(Step::Next);
        nav.select_page(Page::Stat);
        assert_eq!(nav.tab(), TabId::Sub(SubTab::Special));
        nav.select_page(Page::Inv);
        assert_eq!(nav.tab(), TabId::Sub(SubTab::Aid));
    }

    #[test]
    fn sub_tab_stepping_clamps_at_both_ends_and_reports_no_move() {
        let mut nav = Nav::default();
        assert_eq!(nav.step_sub(Step::Previous), None);
        assert_eq!(nav.step_sub(Step::Next), Some(Step::Next));
        assert_eq!(nav.step_sub(Step::Next), Some(Step::Next));
        assert_eq!(nav.step_sub(Step::Next), None);
        assert_eq!(nav.tab(), TabId::Sub(SubTab::Perks));
    }

    #[test]
    fn pages_without_sub_tabs_do_not_step_sub_tabs() {
        let mut nav = Nav::default();
        nav.select_page(Page::Map);
        assert_eq!(nav.step_sub(Step::Next), None);
        assert_eq!(nav.tab(), TabId::Map);
    }

    #[test]
    fn page_stepping_clamps_at_both_ends() {
        let mut nav = Nav::default();
        assert_eq!(nav.step_page(Step::Previous), None);
        for _ in 0..4 {
            assert_eq!(nav.step_page(Step::Next), Some(Step::Next));
        }
        assert_eq!(nav.page(), Page::Radio);
        assert_eq!(nav.step_page(Step::Next), None);
    }

    #[test]
    fn selecting_a_sub_tab_reports_change_and_rejects_out_of_range() {
        let mut nav = Nav::default();
        assert!(nav.select_sub(2));
        assert!(!nav.select_sub(2));
        assert!(!nav.select_sub(3));
        assert_eq!(nav.tab(), TabId::Sub(SubTab::Perks));
    }

    #[test]
    fn opening_a_screen_switches_page_and_sub_tab_and_the_perk_chart_covers_the_shell() {
        let mut nav = Nav::default();
        nav.open(Screen::Shell(TabId::Sub(SubTab::Ammo)));
        assert_eq!((nav.page(), nav.sub_index()), (Page::Inv, 6));
        nav.open_perk_chart();
        assert_eq!(nav.screen(), Screen::PerkChart);
        nav.close_perk_chart();
        assert_eq!(nav.screen(), Screen::Shell(TabId::Sub(SubTab::Ammo)));
        nav.open(Screen::Shell(TabId::Radio));
        assert_eq!(nav.screen(), Screen::Shell(TabId::Radio));
    }
}
