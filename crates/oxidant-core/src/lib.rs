// Per-exploration agent loop, conversation, registry.
//
// Specs:
//   spec/components/core/agent-loop.md
//   spec/components/core/conversation.md
//   spec/components/core/exploration.md
//   spec/components/core/tool-registry.md
//   spec/contracts/tool.md

pub mod registry;

pub use registry::{Tool, ToolCategory, ToolContext, ToolRegistry, ToolResult};
