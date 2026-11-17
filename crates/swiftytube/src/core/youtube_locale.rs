/// Region + language.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct YouTubeLocale {
    pub gl: String,
    pub hl: String,
}

impl YouTubeLocale {
    pub fn new(gl: impl Into<String>, hl: impl Into<String>) -> Self {
        Self {
            gl: gl.into(),
            hl: hl.into(),
        }
    }
}

/// Swift `YouTubeLocale.default`.
impl Default for YouTubeLocale {
    fn default() -> Self {
        Self::new("US", "en")
    }
}
