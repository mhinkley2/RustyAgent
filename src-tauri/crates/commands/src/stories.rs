// CRUD Tauri commands for the stories table.

use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

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
    }
}

const SELECT_STORIES: &str = "
    SELECT s.id, s.title, s.description, s.story_type, s.status, s.priority,
           s.assigned_agent_id, a.name AS agent_name, s.requires_approval,
           s.track_history, s.labels, s.sort_order, s.created_at, s.updated_at
    FROM stories s
    LEFT JOIN agent_profiles a ON a.id = s.assigned_agent_id";

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
                "{} WHERE s.story_type != 'chat' AND (s.workspace_id = ? OR s.workspace_id IS NULL) ORDER BY s.sort_order ASC, s.created_at ASC",
                SELECT_STORIES
            );
            sqlx::query(&sql)
                .bind(ws_id)
                .fetch_all(db)
                .await
                .map_err(|e| format!("DB error: {e}"))?
        }
        None => {
            let sql = format!(
                "{} WHERE s.story_type != 'chat' AND s.workspace_id IS NULL ORDER BY s.sort_order ASC, s.created_at ASC",
                SELECT_STORIES
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
        "UPDATE stories SET
             title = ?, description = ?, story_type = ?, status = ?, priority = ?,
             assigned_agent_id = ?, requires_approval = ?, track_history = ?, labels = ?,
             workspace_id = CASE WHEN workspace_id IS NULL THEN ? ELSE workspace_id END,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
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
