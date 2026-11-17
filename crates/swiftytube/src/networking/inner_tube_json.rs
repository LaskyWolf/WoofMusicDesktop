use super::inner_tube_decode::{InnerTubeDecode, JsonDict, as_dict_list, as_string};

/// Tolerant readers for browse JSON (runs text, thumbnails).
pub struct InnerTubeJSON;

impl InnerTubeJSON {
    /// Joined run texts, else simpleText/text.
    pub fn runs_text(dict: Option<&JsonDict>) -> Option<String> {
        let dict = dict?;
        if let Some(runs) = as_dict_list(dict.get("runs")).filter(|r| !r.is_empty()) {
            let joined: String = runs
                .iter()
                .filter_map(|r| as_string(r.get("text")))
                .collect();
            if !joined.is_empty() {
                return Some(joined);
            }
        }
        InnerTubeDecode::runs_text_tolerant_dict(dict)
    }

    /// All non-blank run texts, dropping `" • "` separators.
    pub fn runs_texts(dict: Option<&JsonDict>) -> Vec<String> {
        Self::raw_runs(dict)
            .into_iter()
            .filter_map(|r| as_string(r.get("text")))
            .filter(|t| *t != " • " && !t.trim().is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Raw runs array (empty when absent).
    pub fn raw_runs(dict: Option<&JsonDict>) -> Vec<&JsonDict> {
        as_dict_list(dict.and_then(|d| d.get("runs"))).unwrap_or_default()
    }

    /// Last URL of a `thumbnails` array (largest variant).
    pub fn last_thumbnail_url(thumbnails: Option<&[&JsonDict]>) -> Option<String> {
        as_string(thumbnails?.last()?.get("url")).map(str::to_string)
    }

    /// Last URL of `{thumbnail: {thumbnails: [...]}}`.
    pub fn nested_thumbnail_url(dict: Option<&JsonDict>) -> Option<String> {
        let thumbnails = InnerTubeDecode::array(&["thumbnail", "thumbnails"], dict?);
        Self::last_thumbnail_url(thumbnails.as_deref())
    }

    /// Largest thumbnail in any known envelope.
    pub fn music_thumbnail_url(dict: &JsonDict) -> Option<String> {
        let url = InnerTubeDecode::thumbnail_url_tolerant(Some(dict));
        if url.is_none() {
            let mut keys: Vec<&String> = dict.keys().collect();
            keys.sort();
            InnerTubeDecode::warn_once(&format!(
                "musicThumbnailURL: unknown thumbnail envelope keys={keys:?}"
            ));
        }
        url
    }
}
