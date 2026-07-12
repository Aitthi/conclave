use crate::ledger::ConvState;
use crate::RE_EVAL_GROWTH;

pub enum Decision {
    Passthrough,
    ApplyFrozen,
    Reevaluate,
}

/// Cache-aware hysteresis (spec D5). Frozen elisions keep applying forever —
/// prefix stability. Caller contract on `Reevaluate`: run `analyze`, EXTEND
/// `frozen` (monotone — never remove), set `last_eval_est = est_tokens`.
pub fn decide(
    est_tokens: usize,
    window: usize,
    high_water_ratio: f32,
    conv: &ConvState,
) -> Decision {
    let high_water = (high_water_ratio * window as f32) as usize;
    if est_tokens < high_water {
        return if conv.frozen.is_empty() {
            Decision::Passthrough
        } else {
            Decision::ApplyFrozen
        };
    }
    if conv.last_eval_est == 0 || est_tokens as f32 >= conv.last_eval_est as f32 * RE_EVAL_GROWTH {
        Decision::Reevaluate
    } else {
        Decision::ApplyFrozen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::{Elision, ElisionReason};
    use crate::ledger::ConvState;

    fn conv(frozen: usize, last_eval_est: usize) -> ConvState {
        ConvState {
            msg_hashes: vec![1],
            frozen: (0..frozen)
                .map(|i| Elision {
                    tool_use_id: format!("tu_{i}"),
                    stub: "s".into(),
                    reason: ElisionReason::DuplicateResult {
                        tool: "Bash".into(),
                        kept_msg: 0,
                    },
                })
                .collect(),
            last_eval_est,
            last_input_tokens: None,
            checkpoint_boundary: None,
            plateau_turns: 0,
        }
    }

    #[test]
    fn below_water_passthrough_or_frozen() {
        assert!(matches!(
            decide(69_000, 100_000, 0.70, &conv(0, 0)),
            Decision::Passthrough
        ));
        assert!(matches!(
            decide(69_000, 100_000, 0.70, &conv(1, 0)),
            Decision::ApplyFrozen
        ));
    }

    #[test]
    fn first_crossing_reevaluates() {
        assert!(matches!(
            decide(70_000, 100_000, 0.70, &conv(0, 0)),
            Decision::Reevaluate
        ));
    }

    #[test]
    fn growth_hysteresis() {
        // evaluated at 80k: +5% → frozen only; +12% → re-evaluate
        assert!(matches!(
            decide(84_000, 100_000, 0.70, &conv(1, 80_000)),
            Decision::ApplyFrozen
        ));
        assert!(matches!(
            decide(89_600, 100_000, 0.70, &conv(1, 80_000)),
            Decision::Reevaluate
        ));
    }

    #[test]
    fn lower_ratio_makes_passthrough_reevaluate() {
        // 30k of a 100k window passes through at the 0.70 default…
        assert!(matches!(
            decide(30_000, 100_000, 0.70, &conv(0, 0)),
            Decision::Passthrough
        ));
        // …but crosses a 0.25 high-water and re-evaluates.
        assert!(matches!(
            decide(30_000, 100_000, 0.25, &conv(0, 0)),
            Decision::Reevaluate
        ));
    }
}
