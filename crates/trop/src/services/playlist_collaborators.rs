//! Trop/Services/PlaylistCollaborators.swift.

use serde_json::{Value, json};
use swiftytube::networking::inner_tube_client::InnerTubeClient;
use swiftytube::networking::inner_tube_decode::{InnerTubeDecode, JsonDict, as_dict, as_string};
use swiftytube::networking::retry_policy::RetryPolicy;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlaylistCollaborator {
    pub name: String,
    pub avatar_url: Option<String>,
    pub is_owner: bool,
    /// The person's channel, for their profile page; `None` when the list has no link.
    pub channel_id: Option<String>,
}

pub struct PlaylistCollaborators;

impl PlaylistCollaborators {
    /// The people on a shared playlist, owner first. `params` comes from the
    /// playlist header's facepile; an empty list means the request failed.
    pub async fn fetch(client: &InnerTubeClient, params: &str) -> Vec<PlaylistCollaborator> {
        let mut body = client.body(None, None);
        body.insert("panelId".into(), "PAplaylist_collaborate".into());
        body.insert("params".into(), params.into());
        match client
            .post("get_panel", body, None, RetryPolicy::INNER_TUBE)
            .await
        {
            Ok(json) => Self::parse(&json),
            Err(_) => Vec::new(),
        }
    }

    pub fn parse(json: &JsonDict) -> Vec<PlaylistCollaborator> {
        let people = InnerTubeDecode::array(
            &[
                "content",
                "engagementPanelSectionListRenderer",
                "content",
                "playlistCollaborationViewModel",
                "playlistCollaborators",
            ],
            json,
        )
        .unwrap_or_default();
        people
            .into_iter()
            .enumerate()
            .filter_map(|(index, person)| {
                let item = as_dict(person.get("contentListItemViewModel"))?;
                let name = InnerTubeDecode::string(&["title", "content"], item)?;
                let avatar_url = InnerTubeDecode::array(
                    &["avatar", "avatarViewModel", "image", "sources"],
                    item,
                )
                .and_then(|sources| as_string(sources.first()?.get("url")))
                .map(str::to_owned);
                Some(PlaylistCollaborator {
                    name: name.to_owned(),
                    avatar_url,
                    // YouTube lists the owner first.
                    is_owner: index == 0,
                    channel_id: Self::channel_id(&json!(item)),
                })
            })
            .collect()
    }

    /// The first channel id (UC…) anywhere under `value`: the row's tap command.
    fn channel_id(value: &Value) -> Option<String> {
        match value {
            Value::Object(dict) => match dict.get("browseId").and_then(Value::as_str) {
                Some(id) if id.starts_with("UC") => Some(id.to_owned()),
                _ => dict.values().find_map(Self::channel_id),
            },
            Value::Array(array) => array.iter().find_map(Self::channel_id),
            _ => None,
        }
    }
}
