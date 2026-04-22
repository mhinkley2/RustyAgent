# RUSTYAGE-4: LLM provider layer (Anthropic, OpenRouter, Ollama)

- Story ID: 0c6a61bd-75a0-41b2-83fd-52ca3be27c56
- Story Type: Story
- Status: Done
- Priority: Critical
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, backend, llm
- Created At: 04/09/2026 20:03:48

## Description

Implement the LLM provider abstraction and clients for Anthropic, OpenRouter, and Ollama in the `api/` crate. All clients must support streaming. Includes a `MockLlmProvider` for deterministic testing.

**Acceptance Criteria:**
- [ ] `LlmProvider` trait defined in `api/` crate: `stream_completion(messages, tools, config) -> Stream<Token>`
- [ ] Anthropic client: Claude API, streaming SSE, tool_use block support
- [ ] OpenRouter client: OpenAI-compatible API, streaming, model passthrough
- [ ] Ollama client: local HTTP, streaming, auto-discover available models from `/api/tags`
- [ ] `MockLlmProvider` implements `LlmProvider` with scripted deterministic responses — usable in unit tests without real API calls
- [ ] API keys read from OS keychain (never from env vars or config files)
- [ ] Settings page: add/update API keys for Anthropic and OpenRouter
- [ ] Model selector in agent profile dynamically fetches available models per provider
- [ ] Error handling: API errors, network failures, rate limits surfaced to UI

**Technical Notes:**
- Lives in `crates/api/`; imported by `crates/runtime/`
- `MockLlmProvider::script(vec![Response::text(...), Response::tool_call(...)])` for test setup
- `reqwest` with streaming for all HTTP clients
