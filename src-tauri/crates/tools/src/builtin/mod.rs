pub mod story;
pub mod memory;
pub mod notify;
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
    registry.register(Box::new(file::FileReadTool));
    registry.register(Box::new(file::FileWriteTool));
    registry.register(Box::new(file::FileListTool));
}
