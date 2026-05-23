// Per-exploration agent loop, conversation, registry.
//
// Specs:
//   spec/components/core/agent-loop.md
//   spec/components/core/conversation.md
//   spec/components/core/exploration.md
//   spec/components/core/tool-registry.md
//   spec/contracts/tool.md

pub mod agent_loop;
pub mod conversation;
pub mod exploration;
pub mod message;
pub mod registry;

pub use agent_loop::{AgentLoopConfig, AgentLoopOutcome, build_request, run};
pub use conversation::Conversation;
pub use exploration::{Exploration, ExplorationId, ExplorationKind, ExplorationSummary, LspHandle};
pub use message::{ContentBlock, ImageSource, Message, ToolResultContent};
pub use registry::{Tool, ToolCategory, ToolContext, ToolRegistry, ToolResult};
