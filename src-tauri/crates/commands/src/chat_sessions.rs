use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSessionSummary {
    pub id: String,
    pub title: String,
    pub agent_profile_id: Option<String>,
    pub agent_name: Option<String>,
    pub last_message_preview: Option<String>,
    pub last_updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSessionMessage {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub agent_profile_id: Option<String>,
    pub created_at: String,
}

pub async fn create_chat_session(
    workspace_id: Option<String>,
    title: Option<String>,
    db: State<'_, DbPool>,
) -> Result<ChatSessionSummary, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let title = title
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "New Chat".to_string());

    sqlx::query(
        "INSERT INTO stories (id, title, story_type, status, priority, workspace_id)
         VALUES (?, ?, 'chat', 'in_progress', 'medium', ?)",
    )
    .bind(&id)
    .bind(&title)
    .bind(&workspace_id)
    .execute(db.inner())
    .await
    .map_err(|e| format!("DB error creating chat session: {e}"))?;

    Ok(ChatSessionSummary {
        id,
        title,
        agent_profile_id: None,
        agent_name: None,
        last_message_preview: None,
        last_updated_at: "".to_string(),
    })
}

pub async fn list_chat_sessions(
    workspace_id: Option<String>,
    limit: Option<i64>,
    db: State<'_, DbPool>,
) -> Result<Vec<ChatSessionSummary>, String> {
    let mut conditions: Vec<String> = vec!["s.story_type = 'chat'".to_string()];
    let mut binds: Vec<String> = Vec::new();

    match workspace_id {
        Some(ws_id) => {
            // Include legacy/global rows alongside workspace-scoped sessions.
            conditions.push("(s.workspace_id = ? OR s.workspace_id IS NULL)".to_string());
            binds.push(ws_id);
        }
        None => {
            conditions.push("s.workspace_id IS NULL".to_string());
        }
    }

        let sql = format!(
                "SELECT s.id, s.title,
                                lm.agent_profile_id AS agent_profile_id,
                                ap.name AS agent_name,
                                lm.content AS last_message_preview,
                                COALESCE(lm.created_at, s.updated_at) AS last_updated_at
                 FROM stories s
                 LEFT JOIN chat_session_messages lm
                     ON lm.id = (
                                SELECT csm.id
                                FROM chat_session_messages csm
                                WHERE csm.session_id = s.id
                                ORDER BY csm.created_at DESC, csm.id DESC
                                LIMIT 1
                     )
                 LEFT JOIN agent_profiles ap ON ap.id = lm.agent_profile_id
                 WHERE {}
                 ORDER BY last_updated_at DESC
                 LIMIT ?",
                conditions.join(" AND ")
        );

    let mut q = sqlx::query(&sql);
    for b in &binds {
        q = q.bind(b);
    }
    q = q.bind(limit.unwrap_or(50));

    let rows = q
        .fetch_all(db.inner())
        .await
        .map_err(|e| format!("DB error listing chat sessions: {e}"))?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let preview: Option<String> = row.try_get("last_message_preview").ok().flatten();
            ChatSessionSummary {
                id: row.try_get("id").unwrap_or_default(),
                title: row.try_get("title").unwrap_or_else(|_| "Chat Session".to_string()),
                agent_profile_id: row.try_get("agent_profile_id").ok().flatten(),
                agent_name: row.try_get("agent_name").ok().flatten(),
                last_message_preview: preview.map(|p| {
                    if p.len() > 120 {
                        format!("{}...", &p[..120])
                    } else {
                        p
                    }
                }),
                last_updated_at: row
                    .try_get("last_updated_at")
                    .unwrap_or_else(|_| "".to_string()),
            }
        })
        .collect())
}

pub async fn get_chat_session_messages(
    session_id: String,
    db: State<'_, DbPool>,
) -> Result<Vec<ChatSessionMessage>, String> {
    let rows = sqlx::query(
        "SELECT id, session_id, role, content, agent_profile_id, created_at
         FROM chat_session_messages
         WHERE session_id = ?
         ORDER BY created_at ASC, id ASC",
    )
    .bind(&session_id)
    .fetch_all(db.inner())
    .await
    .map_err(|e| format!("DB error loading chat session messages: {e}"))?;

    Ok(rows
        .into_iter()
        .map(|row| ChatSessionMessage {
            id: row.try_get("id").unwrap_or_default(),
            session_id: row.try_get("session_id").unwrap_or_default(),
            role: row.try_get("role").unwrap_or_default(),
            content: row.try_get("content").unwrap_or_default(),
            agent_profile_id: row.try_get("agent_profile_id").ok().flatten(),
            created_at: row.try_get("created_at").unwrap_or_default(),
        })
        .collect())
}

pub async fn append_chat_session_message(
    session_id: String,
    role: String,
    content: String,
    agent_profile_id: Option<String>,
    db: State<'_, DbPool>,
) -> Result<(), String> {
    if content.trim().is_empty() {
        return Ok(());
    }

    let role = role.trim().to_lowercase();
    if role != "user" && role != "assistant" {
        return Err("role must be 'user' or 'assistant'".to_string());
    }

    let id = uuid::Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO chat_session_messages (id, session_id, role, content, agent_profile_id)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&session_id)
    .bind(&role)
    .bind(&content)
    .bind(agent_profile_id)
    .execute(db.inner())
    .await
    .map_err(|e| format!("DB error appending chat message: {e}"))?;

    sqlx::query("UPDATE stories SET updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(&session_id)
        .execute(db.inner())
        .await
        .map_err(|e| format!("DB error touching chat session timestamp: {e}"))?;

    Ok(())
}
