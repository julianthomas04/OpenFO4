#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Gait {
    Walk,
    Run,
    Sprint,
    Sneak,
    Swim,
}

impl Gait {
    pub fn parse(text: &str) -> Option<Gait> {
        match text {
            "walk" => Some(Gait::Walk),
            "run" => Some(Gait::Run),
            "sprint" => Some(Gait::Sprint),
            "sneak" => Some(Gait::Sneak),
            "swim" => Some(Gait::Swim),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FootTag {
    FootRight,
    FootLeft,
    FootSprintRight,
    FootSprintLeft,
    JumpUp,
    JumpDown,
}

impl FootTag {
    pub fn parse(text: &str) -> Option<FootTag> {
        match text {
            "FootRight" => Some(FootTag::FootRight),
            "FootLeft" => Some(FootTag::FootLeft),
            "FootSprintRight" => Some(FootTag::FootSprintRight),
            "FootSprintLeft" => Some(FootTag::FootSprintLeft),
            "JumpUp" => Some(FootTag::JumpUp),
            "JumpDown" => Some(FootTag::JumpDown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SurfaceMaterial {
    None,
    Hash(u32),
    Terrain { point: [f32; 3] },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FootstepEvent {
    pub tag: FootTag,
    pub gait: Gait,
    pub surface: SurfaceMaterial,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gait_parse_accepts_db_text() {
        assert_eq!(Gait::parse("walk"), Some(Gait::Walk));
        assert_eq!(Gait::parse("run"), Some(Gait::Run));
        assert_eq!(Gait::parse("sprint"), Some(Gait::Sprint));
        assert_eq!(Gait::parse("sneak"), Some(Gait::Sneak));
        assert_eq!(Gait::parse("swim"), Some(Gait::Swim));
    }

    #[test]
    fn gait_parse_rejects_unknown_and_wrong_case() {
        assert_eq!(Gait::parse("Run"), None);
        assert_eq!(Gait::parse(""), None);
        assert_eq!(Gait::parse("crawl"), None);
    }

    #[test]
    fn foot_tag_parse_accepts_exact_strings() {
        assert_eq!(FootTag::parse("FootRight"), Some(FootTag::FootRight));
        assert_eq!(FootTag::parse("FootLeft"), Some(FootTag::FootLeft));
        assert_eq!(
            FootTag::parse("FootSprintRight"),
            Some(FootTag::FootSprintRight)
        );
        assert_eq!(
            FootTag::parse("FootSprintLeft"),
            Some(FootTag::FootSprintLeft)
        );
        assert_eq!(FootTag::parse("JumpUp"), Some(FootTag::JumpUp));
        assert_eq!(FootTag::parse("JumpDown"), Some(FootTag::JumpDown));
    }

    #[test]
    fn foot_tag_parse_rejects_unknown_and_wrong_case() {
        assert_eq!(FootTag::parse("footright"), None);
        assert_eq!(FootTag::parse(""), None);
        assert_eq!(FootTag::parse("FootSprint"), None);
    }
}
