// Three-tier memory system: SQLite episodic store + fastembed-powered semantic store.
// See RUSTYAGE-12 for implementation details.
//
// Episodic: key-value entries in `agent_memory` table, scoped per agent + scope type.
// Semantic: run summaries embedded via fastembed AllMiniLML6V2 and stored in
//           `memory_semantic`; searched via cosine similarity at run start.

use std::sync::Arc;

use anyhow::{Context, Result};
use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tracing::{info, warn};
use uuid::Uuid;
use db::timestamps::NOW_ISO8601;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Episodic memory scope — controls where a key-value entry is accessible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// Lives for the duration of a single session; cleared on next run.
    Session,
    /// Survives across runs for the same agent profile.
    Persistent,
    /// Shared across all agents within the same pipeline run.
    SharedScratchpad,
}

impl MemoryScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Persistent => "persistent",
            Self::SharedScratchpad => "shared_scratchpad",
        }
    }
}

impl std::str::FromStr for MemoryScope {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "session" => Ok(Self::Session),
            "persistent" => Ok(Self::Persistent),
            "shared_scratchpad" => Ok(Self::SharedScratchpad),
            other => anyhow::bail!("Unknown MemoryScope: '{other}'"),
        }
    }
}

/// A result from a semantic memory search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryResult {
    pub content: String,
    pub similarity: f32,
}

// ---------------------------------------------------------------------------
// EpisodicStore — SQLite key-value store
// ---------------------------------------------------------------------------

/// Key-value episodic store backed by the `agent_memory` SQLite table.
#[derive(Clone)]
pub struct EpisodicStore {
    db: DbPool,
    agent_profile_id: String,
}

impl EpisodicStore {
    pub fn new(db: DbPool, agent_profile_id: impl Into<String>) -> Self {
        Self { db, agent_profile_id: agent_profile_id.into() }
    }

    /// Upsert a key-value entry in the given scope.
    pub async fn write(&self, scope: MemoryScope, key: &str, value: &str) -> Result<()> {
        // SQLite's UNIQUE constraint treats NULL != NULL so `ON CONFLICT` never fires
        // when pipeline_run_id IS NULL.  Use explicit UPDATE-then-INSERT instead.
        let updated = sqlx::query(
            &format!("UPDATE agent_memory \
             SET value = ?, updated_at = {NOW_ISO8601} \
             WHERE agent_profile_id = ? AND scope = ? AND key = ? AND pipeline_run_id IS NULL"),
        )
        .bind(value)
        .bind(&self.agent_profile_id)
        .bind(scope.as_str())
        .bind(key)
        .execute(&self.db)
        .await
        .context("EpisodicStore::write UPDATE")?;

        if updated.rows_affected() == 0 {
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO agent_memory \
                 (id, agent_profile_id, scope, key, value, pipeline_run_id) \
                 VALUES (?, ?, ?, ?, ?, NULL)",
            )
            .bind(&id)
            .bind(&self.agent_profile_id)
            .bind(scope.as_str())
            .bind(key)
            .bind(value)
            .execute(&self.db)
            .await
            .context("EpisodicStore::write INSERT")?;
        }

        Ok(())
    }

    /// Read a value by scope and key. Returns `None` if no entry exists.
    pub async fn read(&self, scope: MemoryScope, key: &str) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT value FROM agent_memory \
             WHERE agent_profile_id = ? AND scope = ? AND key = ? AND pipeline_run_id IS NULL",
        )
        .bind(&self.agent_profile_id)
        .bind(scope.as_str())
        .bind(key)
        .fetch_optional(&self.db)
        .await
        .context("EpisodicStore::read")?;

        Ok(row.map(|r| r.try_get::<String, _>("value").unwrap_or_default()))
    }
}

// ---------------------------------------------------------------------------
// SemanticStore — fastembed + SQLite vector store
// ---------------------------------------------------------------------------

/// Semantic memory backed by fastembed (AllMiniLML6V2, 384-dim) embeddings stored in SQLite.
///
/// The model is loaded asynchronously during construction. If it fails (e.g. no internet
/// on first use, before the model file is cached), semantic operations silently no-op so
/// the agent run continues unaffected.
#[derive(Clone)]
pub struct SemanticStore {
    db: DbPool,
    /// None when the model failed to load; all ops are no-ops in that case.
    model: Option<Arc<fastembed::TextEmbedding>>,
}

impl SemanticStore {
    /// Attempt to load the embedding model in a blocking thread, then return.
    pub async fn new(db: DbPool) -> Self {
        let model = tokio::task::spawn_blocking(|| {
            use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
            TextEmbedding::try_new(
                InitOptions::new(EmbeddingModel::AllMiniLML6V2)
                    .with_show_download_progress(true),
            )
        })
        .await
        .ok()
        .and_then(|r| r.ok())
        .map(Arc::new);

        if model.is_some() {
            info!("Semantic memory: AllMiniLML6V2 model ready");
        } else {
            warn!("Semantic memory: embedding model unavailable — semantic features disabled");
        }

        Self { db, model }
    }

    /// Returns true if the embedding model was loaded successfully.
    pub fn is_ready(&self) -> bool {
        self.model.is_some()
    }

    /// Embed text into a float vector, running on a blocking thread.
    async fn embed(&self, text: String) -> Result<Vec<f32>> {
        let model = match &self.model {
            Some(m) => Arc::clone(m),
            None => anyhow::bail!("Embedding model not available"),
        };
        tokio::task::spawn_blocking(move || {
            let mut embeddings = model.embed(vec![text], None)?;
            Ok::<Vec<f32>, anyhow::Error>(embeddings.pop().unwrap_or_default())
        })
        .await
        .context("spawn_blocking panicked")?
    }

    /// Embed and persist a piece of text for the given agent.
    pub async fn write(&self, agent_profile_id: &str, content: &str) -> Result<()> {
        if self.model.is_none() {
            return Ok(());
        }
        let embedding = self.embed(content.to_string()).await?;
        let embedding_json = serde_json::to_string(&embedding)?;
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO memory_semantic (id, agent_profile_id, content, embedding) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(agent_profile_id)
        .bind(content)
        .bind(&embedding_json)
        .execute(&self.db)
        .await
        .context("SemanticStore::write INSERT")?;
        Ok(())
    }

    /// Return the top-K most similar memories for the given agent and query.
    pub async fn search(
        &self,
        agent_profile_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<MemoryResult>> {
        if self.model.is_none() || top_k == 0 {
            return Ok(vec![]);
        }

        let query_emb = self.embed(query.to_string()).await?;

        // For a desktop app with O(100–1000) entries, loading all and doing dot-product in
        // Rust is simpler and fast enough — no need for an external vector database.
        let rows = sqlx::query(
            "SELECT content, embedding FROM memory_semantic WHERE agent_profile_id = ?",
        )
        .bind(agent_profile_id)
        .fetch_all(&self.db)
        .await
        .context("SemanticStore::search fetch")?;

        let mut scored: Vec<MemoryResult> = rows
            .into_iter()
            .filter_map(|row| {
                let content: String = row.try_get("content").ok()?;
                let emb_json: String = row.try_get("embedding").ok()?;
                let emb: Vec<f32> = serde_json::from_str(&emb_json).ok()?;
                let similarity = cosine_similarity(&query_emb, &emb);
                Some(MemoryResult { content, similarity })
            })
            .collect();

        scored.sort_by(|a, b| {
            b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(top_k);
        Ok(scored)
    }
}

/// Cosine similarity between two equal-length float vectors.
/// AllMiniLML6V2 outputs are already L2-normalised, so this is just a dot product,
/// but we compute the full formula for correctness with other models.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

// ---------------------------------------------------------------------------
// MemoryStore — unified interface
// ---------------------------------------------------------------------------

/// Combines episodic and semantic memory for a single agent profile.
#[derive(Clone)]
pub struct MemoryStore {
    pub episodic: EpisodicStore,
    pub semantic: SemanticStore,
    pub agent_profile_id: String,
}

impl MemoryStore {
    /// Create a new MemoryStore. This is async because it loads the fastembed model.
    pub async fn new(db: DbPool, agent_profile_id: impl Into<String>) -> Self {
        let agent_profile_id = agent_profile_id.into();
        let semantic = SemanticStore::new(db.clone()).await;
        let episodic = EpisodicStore::new(db, agent_profile_id.clone());
        Self { episodic, semantic, agent_profile_id }
    }

    // Convenience delegates matching the Memory trait in the story spec.

    pub async fn write_episodic(&self, scope: MemoryScope, key: &str, value: &str) -> Result<()> {
        self.episodic.write(scope, key, value).await
    }

    pub async fn read_episodic(&self, scope: MemoryScope, key: &str) -> Result<Option<String>> {
        self.episodic.read(scope, key).await
    }

    pub async fn write_semantic(&self, content: &str) -> Result<()> {
        self.semantic.write(&self.agent_profile_id, content).await
    }

    pub async fn search_semantic(&self, query: &str, top_k: usize) -> Result<Vec<MemoryResult>> {
        self.semantic.search(&self.agent_profile_id, query, top_k).await
    }
}
