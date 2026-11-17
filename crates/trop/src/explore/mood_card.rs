// Logic half of Trop/Explore/MoodCard.swift: `MoodFamily` groups genres into
// families for Explore's chips. The tile view, its SF Symbols and colors are
// UI and are not translated.

use crate::networking::models::home_page::MoodItem;

/// One chip in Explore: a family with two or more genres, or a single mood.
#[derive(Clone, Debug, PartialEq)]
pub enum MoodFamilyEntry {
    Group {
        family: String,
        members: Vec<MoodItem>,
    },
    Mood(MoodItem),
}

impl MoodFamilyEntry {
    pub fn id(&self) -> String {
        match self {
            MoodFamilyEntry::Group { family, .. } => format!("group-{family}"),
            MoodFamilyEntry::Mood(mood) => format!("mood-{}", mood.id),
        }
    }

    pub fn title(&self) -> &str {
        match self {
            MoodFamilyEntry::Group { family, .. } => family,
            MoodFamilyEntry::Mood(mood) => &mood.title,
        }
    }
}

const FAMILIES: &[(&str, &[&str])] = &[
    ("Hip-Hop", &["hip-hop", "hip hop", "rap", "trap"]),
    ("Metal", &["metal"]),
    ("Rock", &["rock", "punk", "grunge"]),
    ("R&B & Soul", &["r&b", "soul"]),
    (
        "Dance & Electronic",
        &["dance", "electronic", "edm", "house", "techno"],
    ),
    ("Latin", &["latin", "reggaeton", "salsa", "bachata"]),
    ("Reggae", &["reggae", "dancehall"]),
    ("Jazz & Blues", &["jazz", "blues"]),
    ("Classical", &["classical"]),
    ("Country & Folk", &["country", "folk", "bluegrass"]),
    ("Pop", &["pop"]),
];

/// Genres that share a family (Hip-Hop, Brazilian Hip-Hop...) share a color.
/// A family with two or more genres becomes one chip in Explore; its genres
/// show as cards under the chips.
pub struct MoodFamily;

impl MoodFamily {
    pub fn names() -> Vec<&'static str> {
        FAMILIES.iter().map(|(name, _)| *name).collect()
    }

    pub fn family(title: &str) -> Option<&'static str> {
        let lower = title.to_lowercase();
        FAMILIES
            .iter()
            .find(|(_, words)| words.iter().any(|w| lower.contains(w)))
            .map(|(name, _)| *name)
    }

    /// The genre that is the family itself ("Pop" in Pop), when YouTube lists one.
    pub fn base<'m>(family: &str, members: &'m [MoodItem]) -> Option<&'m MoodItem> {
        let lower = family.to_lowercase();
        members.iter().find(|m| m.title.to_lowercase() == lower)
    }

    /// The family's genres other than the family itself.
    pub fn subgenres(family: &str, moods: &[MoodItem]) -> Vec<MoodItem> {
        let members: Vec<MoodItem> = moods
            .iter()
            .filter(|m| Self::family(&m.title) == Some(family))
            .cloned()
            .collect();
        let base_id = Self::base(family, &members).map(|b| b.id);
        members
            .into_iter()
            .filter(|m| Some(m.id) != base_id)
            .collect()
    }

    /// Families with two or more genres become one chip. Everything else keeps its place.
    pub fn entries(moods: &[MoodItem]) -> Vec<MoodFamilyEntry> {
        let members_of = |family: &str| -> Vec<MoodItem> {
            moods
                .iter()
                .filter(|m| Self::family(&m.title) == Some(family))
                .cloned()
                .collect()
        };
        let mut seen: Vec<&str> = Vec::new();
        let mut result = Vec::new();
        for mood in moods {
            let family = Self::family(&mood.title);
            let group = family.map(members_of).unwrap_or_default();
            match family {
                Some(f) if group.len() > 1 => {
                    if !seen.contains(&f) {
                        seen.push(f);
                        result.push(MoodFamilyEntry::Group {
                            family: f.to_string(),
                            members: group,
                        });
                    }
                }
                _ => result.push(MoodFamilyEntry::Mood(mood.clone())),
            }
        }
        result
    }
}
