use std::{ops::Deref, sync::Arc};

use async_trait::async_trait;
use atuin_client::{
    database::{Context, Database},
    history::History,
    settings::{FilterMode, SearchMode},
};
use eyre::Result;

use super::cursor::Cursor;

pub mod db;
pub mod skim;
pub mod tantivy;

pub fn engine(search_mode: SearchMode) -> Result<Box<dyn SearchEngine>> {
    Ok(match search_mode {
        SearchMode::Skim => Box::new(skim::Search::new()) as Box<_>,
        SearchMode::Tantivy => Box::new(tantivy::Search::new()?) as Box<_>,
        mode => Box::new(db::Search(mode)) as Box<_>,
    })
}

pub struct SearchState {
    pub input: Cursor,
    pub filter_mode: FilterMode,
    pub context: Context,
}

#[async_trait]
pub trait SearchEngine: Send + Sync + 'static {
    async fn full_query(
        &mut self,
        state: &SearchState,
        db: &mut dyn Database,
    ) -> Result<Vec<Arc<HistoryWrapper>>>;

    async fn query(
        &mut self,
        state: &SearchState,
        db: &mut dyn Database,
    ) -> Result<Vec<Arc<HistoryWrapper>>> {
        if state.input.as_str().is_empty() {
            Ok(db
                .list(state.filter_mode, &state.context, Some(200), true)
                .await?
                .into_iter()
                .map(|history| HistoryWrapper { history, count: 1 })
                .map(Arc::new)
                .collect::<Vec<_>>())
        } else {
            self.full_query(state, db).await
        }
    }
}

pub struct HistoryWrapper {
    pub history: History,
    pub count: i32,
}
impl Deref for HistoryWrapper {
    type Target = History;

    fn deref(&self) -> &Self::Target {
        &self.history
    }
}