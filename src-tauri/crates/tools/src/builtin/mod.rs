pub mod story;
pub mod memory;
pub mod notify;
pub mod run;
#[cfg(test)]
mod run_tests;
pub mod subtask;
pub mod file;

use crate::ToolRegistry;
use db::DbPool;

/// Register all built-in tools into a registry.
pub fn register_builtins(registry: &mut ToolRegistry, _db: DbPool) {
    registry.register(Box::new(story::GetStoryTool));
    registry.register(Box::new(story::ListStoriesTool));
    registry.register(Box::new(story::CreateStoryTool));
    registry.register(Box::new(story::UpdateStoryTool));
    registry.register(Box::new(story::UpdateStoryStatusTool));
    registry.register(Box::new(story::DeleteStoryTool));
    registry.register(Box::new(memory::MemoryReadTool));
    registry.register(Box::new(memory::MemoryWriteTool));
    registry.register(Box::new(notify::SendNotificationTool));
    registry.register(Box::new(subtask::SpawnSubtaskTool));
    registry.register(Box::new(run::GetRunTool));
    registry.register(Box::new(file::FileReadTool));
    registry.register(Box::new(file::FileWriteTool));
    registry.register(Box::new(file::FileEditTool));
    registry.register(Box::new(file::FileListTool));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::make_test_pool;

    /// A tool the registry does not hand to the model does not exist as far as
    /// the agent is concerned, so registration is pinned separately from the
    /// tool's own behaviour.
    #[tokio::test]
    async fn register_builtins_exposes_the_file_tools_to_the_agent() {
        let mut registry = ToolRegistry::new();

        register_builtins(&mut registry, make_test_pool().await);

        let names: Vec<String> = registry
            .all_definitions()
            .into_iter()
            .map(|d| d.name)
            .collect();
        for expected in ["file_read", "file_write", "file_edit", "file_list"] {
            assert!(names.contains(&expected.to_string()), "{expected} missing from {names:?}");
        }
    }

    #[tokio::test]
    async fn the_file_edit_definition_advertises_its_parameters() {
        let mut registry = ToolRegistry::new();
        register_builtins(&mut registry, make_test_pool().await);

        let def = registry
            .all_definitions()
            .into_iter()
            .find(|d| d.name == "file_edit")
            .expect("file_edit should be registered");

        let required = def.input_schema["required"]
            .as_array()
            .expect("required list")
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();
        assert_eq!(required, vec!["path", "old_string", "new_string"]);
        assert!(def.input_schema["properties"]["replace_all"].is_object());
    }
}
