pub mod agent;
pub mod artifact;
pub mod blackboard;
pub mod browser;
pub mod cli;
pub mod code;
pub mod design;
// The `draft.agents` wire contract, catalogue and validator land before
// their handler + router arm (plan tasks A3 → A4).
#[allow(dead_code)]
pub mod draft;
pub mod fusion;
pub mod instance;
pub mod memory;
#[cfg(test)]
mod memory_bench;
pub mod message;
pub mod orient;
pub mod provider;
pub mod role;
pub mod skill;
pub mod skill_draft;
pub mod snapshot;
pub mod task;
pub mod tool;
pub mod workspace;
