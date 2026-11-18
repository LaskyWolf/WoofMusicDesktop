// SwiftyTube/Sync/BrowseLens.swift.

use serde_json::Value;

use crate::networking::inner_tube_decode::{
    InnerTubeDecode, JsonDict, as_dict, as_dict_list, as_string,
};
use crate::networking::inner_tube_json::InnerTubeJSON;

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

    /// Account-name runs from an `account/account_menu` response, used to
    /// probe session validity.
    pub fn account_name_runs(json: &JsonDict) -> Option<Vec<&JsonDict>> {
        InnerTubeDecode::array(
            &[
                "header",
                "musicAccountHeaderRenderer",
                "accountName",
                "runs",
            ],
            json,
        )
    }

    /// The signed-in identity anywhere in an account menu: the first
    /// `activeAccountHeaderRenderer` / `musicAccountHeaderRenderer` with a
    /// name or a photo.
    pub fn signed_in_account(json: &JsonDict) -> Option<SignedInAccount> {
        fn scalar_text(node: Option<&Value>) -> Option<String> {
            let dict = as_dict(node)?;
            if let Some(simple) = as_string(dict.get("simpleText")).filter(|s| !s.is_empty()) {
                return Some(simple.to_string());
            }
            InnerTubeJSON::runs_text(Some(dict))
        }

        let mut stack: Vec<&Value> = Vec::new();
        let mut next = Some(json);
        loop {
            if let Some(dict) = next.take() {
                for key in ["activeAccountHeaderRenderer", "musicAccountHeaderRenderer"] {
                    let Some(renderer) = as_dict(dict.get(key)) else {
                        continue;
                    };
                    let name = scalar_text(renderer.get("accountName"));
                    let email = scalar_text(renderer.get("email"));
                    let has_photo = as_dict(renderer.get("accountPhoto"))
                        .is_some_and(|p| p.get("thumbnails").is_some_and(|t| !t.is_null()));
                    if has_photo || name.as_deref().is_some_and(|n| !n.is_empty()) {
                        return Some(SignedInAccount {
                            name,
                            email,
                            has_photo,
                        });
                    }
                }
                stack.extend(dict.values());
            }
            match stack.pop()? {
                Value::Object(dict) => next = Some(dict),
                Value::Array(list) => stack.extend(list),
                _ => {}
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedInAccount {
    pub name: Option<String>,
    pub email: Option<String>,
    pub has_photo: bool,
}
