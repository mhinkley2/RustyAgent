// CRUD Tauri commands for the stories table.

use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;
use db::timestamps::NOW_ISO8601;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Story {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub story_type: String,            // 'task' | 'human' | 'pipeline'
    pub status: String,                // see db::story_status::STORY_STATUSES
    pub priority: String,              // 'low' | 'medium' | 'high' | 'critical'
    pub assigned_agent_id: Option<String>,
    pub assigned_agent_name: Option<String>, // from LEFT JOIN agent_profiles
    pub requires_approval: bool,
    pub track_history: bool,
    pub labels: Vec<String>,           // stored as JSON text in DB
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
    /// The most recent run against this story, if it has ever been run.
    ///
    /// Joined in `SELECT_STORIES` rather than fetched per card: the board
    /// renders every story at once, and a query per card is a query per card.
    pub latest_run: Option<StoryLatestRun>,
}

/// What the board needs to know about a story's most recent run.
///
/// A narrow projection of `story_runs`, not the whole row — the card shows a
/// state, an age and a cost, and the run detail view is one click away for
/// anything more.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryLatestRun {
    pub id: String,
    /// `running` | `done` | `failed` | `cancelled`.
    ///
    /// The *run* vocabulary, which is not the story vocabulary and legitimately
    /// contains `failed` — see `db::story_status` for the other one. The type
    /// this replaced invented a third spelling, `success` / `failure`, which
    /// matched neither.
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    /// Iterations the run has entered, which is what a card can honestly show
    /// as progress. There is no total to count against — an agent loop runs
    /// until it is done or hits `max_iterations`.
    pub iteration_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Deserialize)]
pub struct StoryOrderUpdate {
    pub id: String,
    pub sort_order: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateStoryInput {
    pub title: String,
    pub description: Option<String>,
    pub story_type: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    /// Empty string or absent means no assignee.
    pub assigned_agent_id: Option<String>,
    pub requires_approval: Option<bool>,
    pub track_history: Option<bool>,
    pub labels: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStoryInput {
    pub title: Option<String>,
    pub description: Option<String>,
    pub story_type: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    /// `None` = keep current; `Some("")` = clear assignee; `Some(uuid)` = set assignee.
    pub assigned_agent_id: Option<String>,
    pub requires_approval: Option<bool>,
    pub track_history: Option<bool>,
    pub labels: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_labels(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

fn row_to_story(row: &sqlx::sqlite::SqliteRow) -> Story {
    let requires_approval: i64 = row.try_get("requires_approval").unwrap_or(0);
    let track_history: i64 = row.try_get("track_history").unwrap_or(1);
    let labels_json: String = row.try_get("labels").unwrap_or_else(|_| "[]".to_string());
    Story {
        id:                  row.try_get("id").unwrap_or_default(),
        title:               row.try_get("title").unwrap_or_default(),
        description:         row.try_get("description").ok().flatten(),
        story_type:          row.try_get("story_type").unwrap_or_else(|_| "task".to_string()),
        status:              row.try_get("status").unwrap_or_else(|_| "backlog".to_string()),
        priority:            row.try_get("priority").unwrap_or_else(|_| "medium".to_string()),
        assigned_agent_id:   row.try_get("assigned_agent_id").ok().flatten(),
        assigned_agent_name: row.try_get("agent_name").ok().flatten(),
        requires_approval:   requires_approval != 0,
        track_history:       track_history != 0,
        labels:              parse_labels(&labels_json),
        sort_order:          row.try_get("sort_order").unwrap_or(0),
        created_at:          row.try_get("created_at").unwrap_or_default(),
        updated_at:          row.try_get("updated_at").unwrap_or_default(),
        latest_run:          row_to_latest_run(row),
    }
}

/// The joined run columns, when the story has a run at all.
///
/// Keyed on the run's id being present: the join is a `LEFT JOIN`, so a story
/// nobody has run yet comes back with every run column NULL, and the card must
/// render exactly as it did before rather than showing an empty slot.
fn row_to_latest_run(row: &sqlx::sqlite::SqliteRow) -> Option<StoryLatestRun> {
    let id: Option<String> = row.try_get("run_id").ok().flatten();
    let id = id?;

    Some(StoryLatestRun {
        id,
        status: row
            .try_get("run_status")
            .ok()
            .flatten()
            .unwrap_or_else(|| "running".to_string()),
        started_at: row.try_get("run_started_at").ok().flatten().unwrap_or_default(),
        finished_at: row.try_get("run_finished_at").ok().flatten(),
        iteration_count: row.try_get("run_iteration_count").ok().flatten().unwrap_or(0),
        input_tokens: row.try_get("run_input_tokens").ok().flatten().unwrap_or(0),
        output_tokens: row.try_get("run_output_tokens").ok().flatten().unwrap_or(0),
        estimated_cost_usd: row
            .try_get("run_estimated_cost_usd")
            .ok()
            .flatten()
            .unwrap_or(0.0),
    })
}

/// The board's order, qualified for this query's `stories s` alias.
///
/// Shares `db::story_status::queue_order_sql` with the scheduler. Sorting the
/// board one way and picking another is the defect this replaces — deciding
/// priority outranks manual position and then not showing it that way would
/// recreate the same lie pointing the other direction.
fn board_order() -> String {
    db::story_status::queue_order_sql("s.")
}

/// The board's read, with each story's most recent run joined in.
///
/// Run timestamps are emitted as RFC 3339 rather than passed through.
/// `story_runs.started_at` is usually written with `CURRENT_TIMESTAMP`, whose
/// `YYYY-MM-DD HH:MM:SS` output JavaScript parses as *local* time — so a card
/// would show every elapsed time shifted by the reader's UTC offset. See
/// story `7b74f638` for fixing what is written; this fixes what is read.
///
/// One query for the whole board. The obvious alternative — fetch the stories,
/// then a run per card — is a query per card on a surface that renders every
/// story at once.
///
/// `ROW_NUMBER()` picks the latest run per story. The tiebreak on `rowid`
/// matters: `story_runs.started_at` is written with `CURRENT_TIMESTAMP`, whose
/// resolution is one second, so two runs started in the same second are not
/// ordered by their timestamps alone. Without it the "latest" run of a
/// fast-retried story would be arbitrary.
const SELECT_STORIES: &str = "
    SELECT s.id, s.title, s.description, s.story_type, s.status, s.priority,
           s.assigned_agent_id, a.name AS agent_name, s.requires_approval,
           s.track_history, s.labels, s.sort_order, s.created_at, s.updated_at,
           r.id                 AS run_id,
           r.status             AS run_status,
           strftime('%Y-%m-%dT%H:%M:%fZ', r.started_at)  AS run_started_at,
           strftime('%Y-%m-%dT%H:%M:%fZ', r.finished_at) AS run_finished_at,
           r.iteration_count    AS run_iteration_count,
           r.input_tokens       AS run_input_tokens,
           r.output_tokens      AS run_output_tokens,
           r.estimated_cost_usd AS run_estimated_cost_usd
    FROM stories s
    LEFT JOIN agent_profiles a ON a.id = s.assigned_agent_id
    LEFT JOIN (
        SELECT id, story_id, status, started_at, finished_at, iteration_count,
               input_tokens, output_tokens, estimated_cost_usd,
               ROW_NUMBER() OVER (
                   PARTITION BY story_id ORDER BY started_at DESC, rowid DESC
               ) AS rn
        FROM story_runs
    ) r ON r.story_id = s.id AND r.rn = 1";

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub async fn get_stories(
    db: &DbPool,
    workspace_id: Option<String>,
) -> Result<Vec<Story>, String> {
    let rows = match workspace_id {
        Some(ref ws_id) => {
            let sql = format!(
                "{} WHERE s.story_type != 'chat' AND (s.workspace_id = ? OR s.workspace_id IS NULL) ORDER BY {}",
                SELECT_STORIES,
                // The same ordering the scheduler picks by, so the top of the
                // Ready column is the story an agent takes next rather than
                // merely the one drawn first.
                board_order()
            );
            sqlx::query(&sql)
                .bind(ws_id)
                .fetch_all(db)
                .await
                .map_err(|e| format!("DB error: {e}"))?
        }
        None => {
            let sql = format!(
                "{} WHERE s.story_type != 'chat' AND s.workspace_id IS NULL ORDER BY {}",
                SELECT_STORIES,
                // The same ordering the scheduler picks by, so the top of the
                // Ready column is the story an agent takes next rather than
                // merely the one drawn first.
                board_order()
            );
            sqlx::query(&sql)
                .fetch_all(db)
                .await
                .map_err(|e| format!("DB error: {e}"))?
        }
    };
    Ok(rows.iter().map(row_to_story).collect())
}

pub async fn get_story(id: String, db: &DbPool) -> Result<Story, String> {
    let sql = format!("{} WHERE s.id = ?", SELECT_STORIES);
    let row = sqlx::query(&sql)
        .bind(&id)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("DB error: {e}"))?
        .ok_or_else(|| format!("Story '{id}' not found"))?;
    Ok(row_to_story(&row))
}

pub async fn create_story(
    input: CreateStoryInput,
    db: &DbPool,
    workspace_id: Option<String>,
) -> Result<Story, String> {
    let id = Uuid::new_v4().to_string();
    let story_type = input.story_type.unwrap_or_else(|| "task".to_string());
    let status     = input.status.unwrap_or_else(|| "backlog".to_string());
    // The UI sends a value from a typed union, so this cannot fire from the
    // board today. It is here because "the frontend would never" is exactly
    // the reasoning that let five vocabularies grow: this is a write path, and
    // a write path either enforces the vocabulary or it does not have one.
    db::story_status::validate_status(&status)?;
    let priority   = input.priority.unwrap_or_else(|| "medium".to_string());
    let requires_approval = input.requires_approval.unwrap_or(false);
    let track_history = input.track_history.unwrap_or(true);
    let labels_json = serde_json::to_string(&input.labels.unwrap_or_default())
        .unwrap_or_else(|_| "[]".to_string());
    // Treat empty string as no assignee.
    let assigned_agent_id = input
        .assigned_agent_id
        .filter(|s| !s.is_empty());

    // Assign sort_order = max + 1 within the same workspace so the new story goes to the bottom.
    let sort_order: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM stories WHERE workspace_id IS ?",
    )
    .bind(&workspace_id)
    .fetch_one(db)
    .await
    .unwrap_or(0);

    sqlx::query(
        "INSERT INTO stories
             (id, title, description, story_type, status, priority,
              assigned_agent_id, requires_approval, track_history, labels, sort_order, workspace_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.title)
    .bind(&input.description)
    .bind(&story_type)
    .bind(&status)
    .bind(&priority)
    .bind(&assigned_agent_id)
    .bind(requires_approval as i64)
    .bind(track_history as i64)
    .bind(&labels_json)
    .bind(sort_order)
    .bind(&workspace_id)
    .execute(db)
    .await
    .map_err(|e| format!("DB insert error: {e}"))?;

    get_story(id, db).await
}

pub async fn update_story(
    id: String,
    input: UpdateStoryInput,
    db: &DbPool,
    workspace_id: Option<String>,
) -> Result<Story, String> {
    let current = get_story(id.clone(), db).await?;
    let input_status_supplied = input.status.is_some();

    let title             = input.title.unwrap_or(current.title);
    let description       = input.description.or(current.description);
    let story_type        = input.story_type.unwrap_or(current.story_type);
    let status            = input.status.unwrap_or(current.status);
    // Only when the caller asked to change it — a row carrying a status from
    // before the vocabulary was settled must still accept a title edit.
    if input_status_supplied {
        db::story_status::validate_status(&status)?;
    }
    let priority          = input.priority.unwrap_or(current.priority);
    let requires_approval = input.requires_approval.unwrap_or(current.requires_approval);
    let track_history = input.track_history.unwrap_or(current.track_history);
    let labels_json = match input.labels {
        Some(l) => serde_json::to_string(&l).unwrap_or_else(|_| "[]".to_string()),
        None    => serde_json::to_string(&current.labels).unwrap_or_else(|_| "[]".to_string()),
    };
    // None → keep current; Some("") → clear; Some(id) → set.
    let assigned_agent_id: Option<String> = match input.assigned_agent_id.as_deref() {
        None     => current.assigned_agent_id,
        Some("") => None,
        Some(s)  => Some(s.to_string()),
    };

    sqlx::query(
        &format!("UPDATE stories SET
             title = ?, description = ?, story_type = ?, status = ?, priority = ?,
             assigned_agent_id = ?, requires_approval = ?, track_history = ?, labels = ?,
             workspace_id = CASE WHEN workspace_id IS NULL THEN ? ELSE workspace_id END,
             updated_at = {NOW_ISO8601}
         WHERE id = ?"),
    )
    .bind(&title)
    .bind(&description)
    .bind(&story_type)
    .bind(&status)
    .bind(&priority)
    .bind(&assigned_agent_id)
    .bind(requires_approval as i64)
    .bind(track_history as i64)
    .bind(&labels_json)
    .bind(&workspace_id)
    .bind(&id)
    .execute(db)
    .await
    .map_err(|e| format!("DB update error: {e}"))?;

    get_story(id, db).await
}

pub async fn delete_story(id: String, db: &DbPool) -> Result<(), String> {
    sqlx::query("DELETE FROM stories WHERE id = ?")
        .bind(&id)
        .execute(db)
        .await
        .map_err(|e| format!("DB delete error: {e}"))?;
    Ok(())
}

pub async fn batch_update_story_order(
    updates: Vec<StoryOrderUpdate>,
    db: &DbPool,
) -> Result<(), String> {
    let pool = db;
    let mut tx = pool.begin().await.map_err(|e| format!("DB error: {e}"))?;
    for u in &updates {
        sqlx::query("UPDATE stories SET sort_order = ? WHERE id = ?")
            .bind(u.sort_order)
            .bind(&u.id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
    }
    tx.commit().await.map_err(|e| format!("DB commit error: {e}"))?;
    Ok(())
}
