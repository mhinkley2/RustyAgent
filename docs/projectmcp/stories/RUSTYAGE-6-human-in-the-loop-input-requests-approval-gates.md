# RUSTYAGE-6: Human-in-the-loop: input requests & approval gates

- Story ID: aa708d79-8339-4934-9ba4-9581865df01e
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, frontend, backend, human-in-loop
- Created At: 04/09/2026 20:04:07

## Description

Implement the human-in-the-loop mechanism. When an agent creates a Human story or a run pauses for approval, the user is notified and can respond.

**Acceptance Criteria:**
- [ ] Agents can call built-in tool `request_human_input(question, context)` which creates a Human-type story
- [ ] Human stories appear prominently in the board (dedicated section or badge on Board page)
- [ ] Desktop notification sent when a Human story is created
- [ ] User can open the story, read the agent's question, and submit a text response
- [ ] On submission: run resumes with the human response injected as a user message
- [ ] Approval gate: runs with `requires_approval=true` pause before each tool execution; user sees tool name + inputs and can approve or reject
- [ ] Rejected tool call → agent receives rejection message and must continue without that tool
