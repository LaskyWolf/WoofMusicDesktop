// SwiftyTube/Sync/BrowseLens.swift: the browse descents the Trop parsers use.
// `accountNameRuns` and `signedInAccount` (session probing) come with R5.

use crate::networking::inner_tube_decode::{InnerTubeDecode, JsonDict, as_dict, as_dict_list};

/// Shared descents into InnerTube browse JSON.
pub struct BrowseLens;

impl BrowseLens {
    /// The `sectionListRenderer` dict of a browse response. Tolerates both
    /// single- and two-column layouts; `None` instead of a crash on drift.
    pub fn section_list(json: &JsonDict) -> Option<&JsonDict> {
        for layout in [
            "singleColumnBrowseResultsRenderer",
            "twoColumnBrowseResultsRenderer",
        ] {
            let Some(tabs) = InnerTubeDecode::array(&["contents", layout, "tabs"], json) else {
                continue;
            };
            let Some(first) = tabs.first() else {
                continue;
            };
            if let Some(sl) =
                InnerTubeDecode::dict(&["tabRenderer", "content", "sectionListRenderer"], first)
            {
                return Some(sl);
            }
        }
        None
    }

    /// The section array of a browse response.
    pub fn browse_sections(json: &JsonDict) -> Option<Vec<&JsonDict>> {
        as_dict_list(Self::section_list(json)?.get("contents"))
    }

    /// The first section of a browse response.
    pub fn first_browse_section(json: &JsonDict) -> Option<&JsonDict> {
        Self::browse_sections(json)?.into_iter().next()
    }

    /// First section's first item, tolerating single- and two-column
    /// layouts. Used by the album/podcast/playlist detail header parsers.
    pub fn first_section_item(json: &JsonDict) -> Option<&JsonDict> {
        let contents = as_dict(json.get("contents"))?;
        let tabs = as_dict(contents.get("twoColumnBrowseResultsRenderer"))
            .and_then(|r| as_dict_list(r.get("tabs")))
            .or_else(|| {
                as_dict(contents.get("singleColumnBrowseResultsRenderer"))
                    .and_then(|r| as_dict_list(r.get("tabs")))
            })?;
        let sections = InnerTubeDecode::array(
            &["tabRenderer", "content", "sectionListRenderer", "contents"],
            tabs.first()?,
        )?;
        sections.into_iter().next()
    }
}
