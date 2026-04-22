# RUSTYAGE-12: Three-tier memory system (SQLite episodic + LanceDB semantic)

- Story ID: ba9ce697-2390-48c8-b03e-043dfb6a6463
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, backend, memory
- Created At: 04/09/2026 20:22:27

## Description

Implement the three-tier memory system in the `memory/` crate. Agents access a unified memory interface that routes to the correct backing store — SQLite for episodic (structured) memory, LanceDB for semantic (vector) memory.

**Acceptance Criteria:**
- [ ] `Memory` trait defined with: `write_episodic(key, value)`, `read_episodic(key)`, `write_semantic(content)`, `search_semantic(query, top_k) -> Vec<MemoryResult>`
- [ ] SQLite episodic store: key-value and freeform entries in `agent_memory` table; scoped by profile_id and scope (session | persistent | shared_scratchpad)
- [ ] LanceDB semantic store: embeddings stored locally alongside SQLite DB file; no daemon required
- [ ] `fastembed-rs` generates embeddings locally using ONNX model; falls back to Ollama `/api/embeddings` if configured
- [ ] Semantic search returns top-K results with similarity scores
- [ ] Memory retrieval at run start: top-K semantically relevant past run summaries injected into system prompt by `ConversationRuntime`
- [ ] Memory write at run end: run summary embedded and stored in LanceDB automatically by `ConversationRuntime`
- [ ] Built-in `memory_read` and `memory_write` agent tools backed by this crate
- [ ] Shared scratchpad scope: agents in the same pipeline run share a namespaced memory space

**Technical Notes:**
- Lives in `crates/memory/`; depends on `db/`
- LanceDB stores data in `{app_data_dir}/memory/` alongside the SQLite file
- fastembed-rs model downloaded on first run and cached locally
- The `memory/` crate is a dependency of `runtime/` and `tools/`
