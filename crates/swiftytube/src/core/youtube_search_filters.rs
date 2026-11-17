// Opaque protobuf `params` for `/search`. Each value stands alone; combining
// filters would need a protobuf builder and is not covered here.

/// Result type filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YouTubeSearchFilter {
    Video,
    Channel,
    Playlist,
    Movie,
}

impl YouTubeSearchFilter {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Video => "EgIQAQ==",
            Self::Channel => "EgIQAg==",
            Self::Playlist => "EgIQAw==",
            Self::Movie => "EgIQBA==",
        }
    }
}

/// Upload-date recency filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YouTubeSearchUploadDate {
    LastHour,
    Today,
    ThisWeek,
    ThisMonth,
    ThisYear,
}

impl YouTubeSearchUploadDate {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::LastHour => "EgIIAQ==",
            Self::Today => "EgIIAg==",
            Self::ThisWeek => "EgIIAw==",
            Self::ThisMonth => "EgIIBA==",
            Self::ThisYear => "EgIIBQ==",
        }
    }
}

/// Result duration filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YouTubeSearchDuration {
    Short,
    Medium,
    Long,
}

impl YouTubeSearchDuration {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Short => "EgIYAQ==",
            Self::Medium => "EgIYAg==",
            Self::Long => "EgIYAw==",
        }
    }
}

/// Result sort order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YouTubeSearchSortOrder {
    Relevance,
    UploadDate,
    ViewCount,
    Rating,
}

impl YouTubeSearchSortOrder {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Relevance => "",
            Self::UploadDate => "CAI=",
            Self::ViewCount => "CAM=",
            Self::Rating => "CAE=",
        }
    }
}
