//! Trop/Audio/Lyrics/LyricsManager.swift.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use swiftytube::networking::http_client::Session;
use tokio::task::JoinSet;

use super::lyrics_service::LyricLine;
use super::providers::genius_provider::GeniusProvider;
use super::providers::kugou_provider::KugouProvider;
use super::providers::lrclib_provider::{LRCLIBProvider, LyricsError};
use super::providers::lyrics_provider::{LyricsParsing, LyricsProvider, LyricsQuery};
use super::providers::musixmatch_provider::MusixmatchProvider;
use super::providers::netease_provider::NeteaseProvider;
use crate::networking::models::home_page::next_id;

/// Swift `LyricsSettings` plus `SettingsStore.disabledLyricsProviders`;
/// saved by `settings::settings_store::SettingsStore`.
#[derive(Clone, Debug, PartialEq)]
pub struct LyricsSettings {
    /// Ordered provider ids.
    pub provider_order: Vec<String>,
    pub disabled_providers: HashSet<String>,
}

impl Default for LyricsSettings {
    fn default() -> Self {
        Self {
            provider_order: LyricsProviderRegistry::DEFAULT_ORDER
                .map(String::from)
                .to_vec(),
            disabled_providers: HashSet::new(),
        }
    }
}

impl LyricsSettings {
    /// A saved order, with providers it does not name appended in the default order.
    pub fn from_saved(saved_order: Vec<String>, disabled_providers: HashSet<String>) -> Self {
        let mut provider_order = saved_order;
        for id in LyricsProviderRegistry::DEFAULT_ORDER {
            if !provider_order.iter().any(|saved| saved == id) {
                provider_order.push(id.to_string());
            }
        }
        Self {
            provider_order,
            disabled_providers,
        }
    }
}

/// Registry of all available providers.
pub struct LyricsProviderRegistry;

impl LyricsProviderRegistry {
    pub const DEFAULT_ORDER: [&str; 5] = ["musixmatch", "lrclib", "netease", "kugou", "genius"];

    pub fn all() -> Vec<Arc<dyn LyricsProvider>> {
        vec![
            Arc::new(LRCLIBProvider),
            Arc::new(MusixmatchProvider),
            Arc::new(NeteaseProvider),
            Arc::new(KugouProvider),
            Arc::new(GeniusProvider),
        ]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LyricSearchResult {
    /// Swift `UUID()`.
    pub id: u64,
    pub lines: Vec<LyricLine>,
    pub provider_name: String,
    pub sort_order: usize,
}

impl LyricSearchResult {
    pub fn preview_text(&self) -> String {
        self.lines
            .iter()
            .take(2)
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn is_synced(&self) -> bool {
        self.lines.iter().any(|line| line.start_time.is_some())
    }
}

struct ProviderHit {
    order: usize,
    lines: Vec<LyricLine>,
    provider_name: String,
}

impl ProviderHit {
    const PLAIN_TEXT_PENALTY: usize = 1_000;

    /// Lower wins. Line-timed lyrics beat plain text from any provider:
    /// the player's lyric line and the synced view need timestamps.
    fn rank(&self) -> usize {
        if self.lines.iter().any(|line| line.start_time.is_some()) {
            self.order
        } else {
            self.order + Self::PLAIN_TEXT_PENALTY
        }
    }
}

pub struct LyricsManager {
    providers: Vec<Arc<dyn LyricsProvider>>,
    session: Session,
    pub settings: Mutex<LyricsSettings>,
}

impl LyricsManager {
    const PROVIDER_TIMEOUT: Duration = Duration::from_secs(10);

    /// Every provider in the registry, over `session` (`Session::shared()` for the network).
    pub fn new(session: Session) -> Self {
        Self::with_providers(LyricsProviderRegistry::all(), session)
    }

    pub fn with_providers(providers: Vec<Arc<dyn LyricsProvider>>, session: Session) -> Self {
        Self {
            providers,
            session,
            settings: Mutex::new(LyricsSettings::default()),
        }
    }

    pub async fn fetch_lyrics(&self, query: &LyricsQuery) -> Result<Vec<LyricLine>, LyricsError> {
        Ok(self.fetch_lyrics_returning_provider(query).await?.0)
    }

    /// The enabled providers in order, with their position in the order;
    /// `excluding` names a provider to leave out.
    fn candidates(&self, excluding: Option<&str>) -> Vec<(usize, Arc<dyn LyricsProvider>)> {
        let settings = self.settings.lock().unwrap();
        settings
            .provider_order
            .iter()
            .enumerate()
            .filter(|(_, id)| !settings.disabled_providers.contains(*id))
            .filter_map(|(index, id)| {
                let provider = self.providers.iter().find(|p| p.id() == id)?;
                (Some(provider.name()) != excluding).then(|| (index, provider.clone()))
            })
            .collect()
    }

    /// Queries every enabled provider concurrently and returns all matches.
    pub async fn search_all(&self, query: &LyricsQuery) -> Vec<LyricSearchResult> {
        let mut set = JoinSet::new();
        for (index, (_, provider)) in self.candidates(None).into_iter().enumerate() {
            let (query, session) = (query.clone(), self.session.clone());
            set.spawn(async move {
                let fetched = provider.fetch(&query, &session).await.ok()?;
                let lines =
                    LyricsParsing::dropping_placeholder(LyricsParsing::dropping_credits(fetched));
                (!lines.is_empty()).then(|| LyricSearchResult {
                    id: next_id(),
                    lines,
                    provider_name: provider.name().to_string(),
                    sort_order: index,
                })
            });
        }
        let mut results: Vec<LyricSearchResult> =
            set.join_all().await.into_iter().flatten().collect();
        results.sort_by_key(|r| r.sort_order);
        results
    }

    /// Fails with `NotFound` only when a provider answered without lyrics;
    /// when none could be reached, with the network error.
    pub async fn fetch_lyrics_returning_provider(
        &self,
        query: &LyricsQuery,
    ) -> Result<(Vec<LyricLine>, String), LyricsError> {
        let best = self
            .race(query, None, false)
            .await?
            .ok_or(LyricsError::NotFound)?;
        Ok((best.lines, best.provider_name))
    }

    /// The highest-priority result with word timing, for karaoke when the
    /// chosen lyrics are line-only. `excluding` is the provider that gave them.
    pub async fn fetch_word_timed(
        &self,
        query: &LyricsQuery,
        excluding: Option<&str>,
    ) -> Option<(Vec<LyricLine>, String)> {
        let best = self.race(query, excluding, true).await.ok()??;
        Some((best.lines, best.provider_name))
    }

    /// Asks the enabled providers concurrently and returns the highest-priority
    /// hit; with `word_timed_only`, results without word timing do not count.
    /// Fails with a network error when no provider answered at all.
    async fn race(
        &self,
        query: &LyricsQuery,
        excluding: Option<&str>,
        word_timed_only: bool,
    ) -> Result<Option<ProviderHit>, LyricsError> {
        let candidates = self.candidates(excluding);
        let mut set = JoinSet::new();
        for (index, provider) in &candidates {
            let (index, provider) = (*index, provider.clone());
            let (query, session) = (query.clone(), self.session.clone());
            set.spawn(async move {
                let result =
                    tokio::time::timeout(Self::PROVIDER_TIMEOUT, provider.fetch(&query, &session))
                        .await
                        .unwrap_or(Err(LyricsError::TimedOut));
                (index, provider.name(), result)
            });
        }

        // Returns as soon as no higher-priority provider is still pending,
        // instead of waiting for every provider (each up to 10 s).
        let mut pending: Vec<usize> = candidates.iter().map(|(index, _)| *index).collect();
        let mut winner: Option<ProviderHit> = None;
        let mut any_answered = false;
        let mut last_error = None;
        while let Some(joined) = set.join_next().await {
            let Ok((index, name, result)) = joined else {
                continue;
            };
            pending.retain(|&i| i != index);
            match result {
                Ok(fetched) => {
                    any_answered = true;
                    let lines = LyricsParsing::dropping_placeholder(
                        LyricsParsing::dropping_credits(fetched),
                    );
                    let counts = !lines.is_empty()
                        && (!word_timed_only || lines.iter().any(|l| l.words.is_some()));
                    let hit = ProviderHit {
                        order: index,
                        lines,
                        provider_name: name.to_string(),
                    };
                    if counts && hit.rank() < winner.as_ref().map_or(usize::MAX, ProviderHit::rank)
                    {
                        winner = Some(hit);
                    }
                }
                // A `LyricsError` answer: no match.
                Err(error) if error.is_answer() => any_answered = true,
                Err(error) => last_error = Some(error),
            }
            // A pending provider's best possible rank is its order.
            if let Some(found) = &winner
                && !pending.iter().any(|&i| i < found.rank())
            {
                break;
            }
        }
        // Dropping `set` aborts the providers still running.
        match (winner, last_error) {
            (None, Some(error)) if !any_answered => Err(error),
            (winner, _) => Ok(winner),
        }
    }
}
