use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use serde_json::Value;

use crate::analyze::Elision;

pub struct ConvState {
    pub msg_hashes: Vec<u64>,
    pub frozen: Vec<Elision>,
    pub last_eval_est: usize,
    pub last_input_tokens: Option<u64>,
    pub checkpoint_boundary: Option<usize>,
    pub plateau_turns: u32,
}

/// In-memory LRU of conversation states, identity by hash-prefix match
/// (spec D10). Oldest at the front, most recently observed at the back.
pub struct Ledger {
    convs: Vec<ConvState>,
    cap: usize,
}

pub fn hash_messages(messages: &Value) -> Vec<u64> {
    let Some(msgs) = messages.as_array() else { return Vec::new() };
    msgs.iter()
        .map(|m| {
            let mut h = DefaultHasher::new();
            m.to_string().hash(&mut h);
            h.finish()
        })
        .collect()
}

impl Ledger {
    pub fn new(cap: usize) -> Self {
        Ledger { convs: Vec::new(), cap: cap.max(1) }
    }

    /// Match by (first-message hash equal AND shorter list is a prefix of
    /// longer); on match update stored hashes to incoming and move to LRU
    /// back; else insert (evicting front past cap). Returns index for conv_mut.
    pub fn observe(&mut self, messages: &Value) -> usize {
        let incoming = hash_messages(messages);
        let matched = self.convs.iter().position(|c| {
            match (c.msg_hashes.first(), incoming.first()) {
                (Some(a), Some(b)) if a == b => {
                    let n = c.msg_hashes.len().min(incoming.len());
                    c.msg_hashes[..n] == incoming[..n]
                }
                _ => false,
            }
        });
        match matched {
            Some(i) => {
                let mut conv = self.convs.remove(i);
                conv.msg_hashes = incoming;
                self.convs.push(conv);
            }
            None => {
                if self.convs.len() >= self.cap {
                    self.convs.remove(0);
                }
                self.convs.push(ConvState {
                    msg_hashes: incoming,
                    frozen: Vec::new(),
                    last_eval_est: 0,
                    last_input_tokens: None,
                    checkpoint_boundary: None,
                    plateau_turns: 0,
                });
            }
        }
        self.convs.len() - 1
    }

    pub fn conv_mut(&mut self, idx: usize) -> &mut ConvState {
        &mut self.convs[idx]
    }

    pub fn len(&self) -> usize {
        self.convs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.convs.is_empty()
    }
}

/// Fold a newly-observed projected boundary into a conversation's plateau count.
/// Same boundary as last eligible sample → +1; a new/changed boundary → reset to 0.
/// Returns the plateau count AFTER this observation.
pub fn record_plateau(conv: &mut ConvState, boundary: usize) -> u32 {
    if conv.checkpoint_boundary == Some(boundary) {
        conv.plateau_turns = conv.plateau_turns.saturating_add(1);
    } else {
        conv.checkpoint_boundary = Some(boundary);
        conv.plateau_turns = 0;
    }
    conv.plateau_turns
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conv(texts: &[&str]) -> serde_json::Value {
        serde_json::Value::Array(
            texts.iter().map(|t| json!({"role": "user", "content": [{"type": "text", "text": t}]})).collect(),
        )
    }

    #[test]
    fn grown_conversation_matches_same_slot() {
        let mut led = Ledger::new(8);
        let idx1 = led.observe(&conv(&["a", "b"]));
        let grown = conv(&["a", "b", "c", "d"]);
        let idx2 = led.observe(&grown);
        assert_eq!(idx1, idx2);
        assert_eq!(led.conv_mut(idx2).msg_hashes, hash_messages(&grown));
    }

    #[test]
    fn different_first_message_is_new_conversation() {
        let mut led = Ledger::new(8);
        let idx1 = led.observe(&conv(&["a", "b"]));
        let idx2 = led.observe(&conv(&["z", "b"]));
        assert_ne!(idx1, idx2);
    }

    #[test]
    fn plateau_increments_while_boundary_holds_and_resets_on_change() {
        let mut led = Ledger::new(4);
        let idx = led.observe(&conv(&["a"]));
        let c = led.conv_mut(idx);
        assert_eq!(record_plateau(c, 7), 0); // first sight of boundary 7
        assert_eq!(record_plateau(c, 7), 1); // held
        assert_eq!(record_plateau(c, 7), 2); // held
        assert_eq!(record_plateau(c, 9), 0); // boundary moved → reset
        assert_eq!(c.checkpoint_boundary, Some(9));
        assert_eq!(c.plateau_turns, 0);
    }

    #[test]
    fn cap_evicts_oldest() {
        let mut led = Ledger::new(2);
        led.observe(&conv(&["one"]));
        led.observe(&conv(&["two"]));
        let idx3 = led.observe(&conv(&["three"])); // evicts "one"
        led.conv_mut(idx3).last_eval_est = 42;
        assert_eq!(led.len(), 2);
        // "one" was evicted: re-observing it re-inserts a FRESH state.
        let idx = led.observe(&conv(&["one", "more"]));
        assert_eq!(led.conv_mut(idx).last_eval_est, 0);
        assert_eq!(led.len(), 2);
        // "three" survived with its state intact (matched, not re-inserted).
        let idx3b = led.observe(&conv(&["three", "x"]));
        assert_eq!(led.conv_mut(idx3b).last_eval_est, 42);
        assert_eq!(led.len(), 2);
    }
}
