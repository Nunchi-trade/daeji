//! Behavioral state machine — six-state FSM with hysteresis.

// ─── States ─────────────────────────────────────────────────────────────────

/// Six behavioral states for the cognitive agent FSM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BehavioralState {
    /// Broad search for new opportunities.
    Explore,
    /// Focused execution of a known opportunity.
    Exploit,
    /// Offline knowledge consolidation (dream cycle).
    Consolidate,
    /// Defensive mode triggered by drawdown or catastrophic loss.
    Emergency,
    /// Post-emergency stabilization before returning to normal operation.
    Recovery,
    /// Guarded re-entry after recovery or alarming consolidation.
    Cautious,
}

// ─── State Modifiers ────────────────────────────────────────────────────────

/// Numeric operational parameters that vary by behavioral state.
#[derive(Clone, Debug)]
pub struct StateModifiers {
    /// Maximum number of entries to retrieve per query.
    pub retrieval_top_k: usize,
    /// Minimum similarity threshold for retrieval.
    pub retrieval_sim_threshold: f64,
    /// Minimum confidence to publish an insight.
    pub publication_conf_threshold: f64,
    /// Risk budget scaling factor.
    pub risk_budget_multiplier: f64,
    /// Minimum trust score required for interaction.
    pub trust_floor: f64,
    /// Position sizing multiplier (0.0 = no positions).
    pub position_size_multiplier: f64,
    /// Cap on somatic bias weight contribution.
    pub somatic_bias_weight_cap: f64,
    /// Fraction of actions devoted to exploration vs exploitation.
    pub exploration_rate: f64,
}

impl StateModifiers {
    /// Get the parameter set for a given behavioral state.
    pub const fn for_state(state: &BehavioralState) -> Self {
        match state {
            BehavioralState::Explore => Self {
                retrieval_top_k: 20,
                retrieval_sim_threshold: 0.55,
                publication_conf_threshold: 0.50,
                risk_budget_multiplier: 1.5,
                trust_floor: 0.15,
                position_size_multiplier: 1.2,
                somatic_bias_weight_cap: 0.30,
                exploration_rate: 0.30,
            },
            BehavioralState::Exploit => Self {
                retrieval_top_k: 5,
                retrieval_sim_threshold: 0.75,
                publication_conf_threshold: 0.70,
                risk_budget_multiplier: 1.0,
                trust_floor: 0.40,
                position_size_multiplier: 1.0,
                somatic_bias_weight_cap: 0.15,
                exploration_rate: 0.05,
            },
            BehavioralState::Consolidate => Self {
                retrieval_top_k: 0,
                retrieval_sim_threshold: 1.0,
                publication_conf_threshold: 1.0,
                risk_budget_multiplier: 0.0,
                trust_floor: 0.25,
                position_size_multiplier: 0.0,
                somatic_bias_weight_cap: 0.05,
                exploration_rate: 0.00,
            },
            BehavioralState::Emergency => Self {
                retrieval_top_k: 3,
                retrieval_sim_threshold: 0.85,
                publication_conf_threshold: 1.0,
                risk_budget_multiplier: 0.1,
                trust_floor: 0.60,
                position_size_multiplier: 0.0,
                somatic_bias_weight_cap: 0.05,
                exploration_rate: 0.00,
            },
            BehavioralState::Recovery => Self {
                retrieval_top_k: 10,
                retrieval_sim_threshold: 0.65,
                publication_conf_threshold: 0.85,
                risk_budget_multiplier: 0.3,
                trust_floor: 0.30,
                position_size_multiplier: 0.2,
                somatic_bias_weight_cap: 0.10,
                exploration_rate: 0.10,
            },
            BehavioralState::Cautious => Self {
                retrieval_top_k: 10,
                retrieval_sim_threshold: 0.70,
                publication_conf_threshold: 0.80,
                risk_budget_multiplier: 0.5,
                trust_floor: 0.55,
                position_size_multiplier: 0.5,
                somatic_bias_weight_cap: 0.10,
                exploration_rate: 0.10,
            },
        }
    }
}

// ─── Tick Metrics ───────────────────────────────────────────────────────────

/// Metrics snapshot used for state transition evaluation.
#[derive(Debug)]
pub struct TickMetrics {
    /// Whether a tradeable opportunity was detected this tick.
    pub opportunity_found: bool,
    /// Confidence level of the detected opportunity.
    pub opportunity_confidence: f64,
    /// Average return over the recent window.
    pub return_avg: f64,
    /// Whether the dream/consolidation cycle completed.
    pub dream_complete: bool,
    /// Current portfolio drawdown from peak.
    pub drawdown: f64,
    /// Loss incurred in the single most recent tick.
    pub single_tick_loss: f64,
    /// Whether the portfolio has stopped losing value.
    pub bleeding_stopped: bool,
    /// Whether the post-emergency postmortem analysis is done.
    pub postmortem_done: bool,
    /// Current mood pleasure dimension for state transition decisions.
    pub mood_pleasure: f64,
    /// Current knowledge base size (entry count).
    pub kb_size: usize,
    /// Ticks elapsed since last consolidation cycle.
    pub ticks_since_consolidation: u64,
    /// Fraction of knowledge entries classified as anti-knowledge.
    pub anti_knowledge_ratio: f64,
    /// Brier score measuring prediction calibration.
    pub brier_score: f64,
    /// Number of strategies demoted during consolidation.
    pub strategies_demoted: u32,
}

// ─── Transition Thresholds ──────────────────────────────────────────────────

const MAX_DRAWDOWN: f64 = 0.10;
const CATASTROPHIC_LOSS: f64 = 0.05;
const EXPLOITATION_RETURN_THRESHOLD: f64 = -0.02;
const CONSOLIDATION_KB_TRIGGER: usize = 10_000;
const CONSOLIDATION_TICK_TRIGGER: u64 = 500;
const EXPLORE_RETURN_THRESHOLD: f64 = 0.01;
const CAUTIOUS_TO_EXPLOIT_CONFIDENCE: f64 = 0.9;

// ─── State Context ──────────────────────────────────────────────────────────

/// Hysteresis-tracking context for the behavioral state machine.
#[derive(Debug)]
pub struct StateContext {
    /// The current behavioral state.
    pub current: BehavioralState,
    /// Counter for hysteresis-based transition gating.
    pub trigger_ticks: u32,
    last_emergency_exit_tick: u64,
    /// The current simulation tick.
    pub current_tick: u64,
}

impl StateContext {
    /// Create a new state context starting in Explore.
    pub const fn new() -> Self {
        Self {
            current: BehavioralState::Explore,
            trigger_ticks: 0,
            last_emergency_exit_tick: 0,
            current_tick: 0,
        }
    }

    /// Evaluate tick metrics and return the (possibly new) behavioral state.
    pub fn evaluate(&mut self, m: &TickMetrics) -> BehavioralState {
        // EMERGENCY is unconditional and immediate (0-tick hysteresis).
        if m.drawdown >= MAX_DRAWDOWN || m.single_tick_loss >= CATASTROPHIC_LOSS {
            self.trigger_ticks = 0;
            self.current = BehavioralState::Emergency;
            return self.current.clone();
        }

        let next = match &self.current {
            BehavioralState::Explore => {
                if m.opportunity_found {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 2 {
                        BehavioralState::Exploit
                    } else {
                        BehavioralState::Explore
                    }
                } else {
                    self.trigger_ticks = 0;
                    BehavioralState::Explore
                }
            }

            BehavioralState::Exploit => {
                let plateau = m.return_avg < EXPLOITATION_RETURN_THRESHOLD;
                let kb_full = m.kb_size > CONSOLIDATION_KB_TRIGGER
                    && m.ticks_since_consolidation > CONSOLIDATION_TICK_TRIGGER;

                if plateau || kb_full {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 5 {
                        BehavioralState::Consolidate
                    } else {
                        BehavioralState::Exploit
                    }
                } else if m.return_avg < EXPLORE_RETURN_THRESHOLD
                    && self.current_tick.saturating_sub(self.last_emergency_exit_tick) >= 20
                {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 5 {
                        BehavioralState::Explore
                    } else {
                        BehavioralState::Exploit
                    }
                } else {
                    self.trigger_ticks = 0;
                    BehavioralState::Exploit
                }
            }

            BehavioralState::Consolidate => {
                if m.dream_complete {
                    if m.anti_knowledge_ratio > 0.3
                        || m.brier_score > 0.4
                        || m.strategies_demoted > 2
                    {
                        BehavioralState::Cautious
                    } else {
                        BehavioralState::Explore
                    }
                } else {
                    BehavioralState::Consolidate
                }
            }

            // PROHIBITED: EMERGENCY -> EXPLORE. Must go through RECOVERY.
            BehavioralState::Emergency => {
                if m.bleeding_stopped {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 3 {
                        self.last_emergency_exit_tick = self.current_tick;
                        BehavioralState::Recovery
                    } else {
                        BehavioralState::Emergency
                    }
                } else {
                    self.trigger_ticks = 0;
                    BehavioralState::Emergency
                }
            }

            BehavioralState::Recovery => {
                if m.postmortem_done {
                    BehavioralState::Cautious
                } else {
                    BehavioralState::Recovery
                }
            }

            BehavioralState::Cautious => {
                if m.opportunity_found && m.opportunity_confidence > CAUTIOUS_TO_EXPLOIT_CONFIDENCE
                {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 3 {
                        BehavioralState::Exploit
                    } else {
                        BehavioralState::Cautious
                    }
                } else if m.return_avg >= 0.0 && m.mood_pleasure >= -0.2 {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 10 {
                        BehavioralState::Explore
                    } else {
                        BehavioralState::Cautious
                    }
                } else {
                    self.trigger_ticks = 0;
                    BehavioralState::Cautious
                }
            }
        };

        if next != self.current {
            self.trigger_ticks = 0;
        }
        self.current = next.clone();
        next
    }
}

impl Default for StateContext {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_metrics() -> TickMetrics {
        TickMetrics {
            opportunity_found: false,
            opportunity_confidence: 0.0,
            return_avg: 0.0,
            dream_complete: false,
            drawdown: 0.0,
            single_tick_loss: 0.0,
            bleeding_stopped: false,
            postmortem_done: false,
            mood_pleasure: 0.0,
            kb_size: 0,
            ticks_since_consolidation: 0,
            anti_knowledge_ratio: 0.0,
            brier_score: 0.0,
            strategies_demoted: 0,
        }
    }

    #[test]
    fn starts_in_explore() {
        let ctx = StateContext::new();
        assert_eq!(ctx.current, BehavioralState::Explore);
    }

    #[test]
    fn emergency_is_immediate() {
        let mut ctx = StateContext::new();
        let m = TickMetrics { drawdown: 0.15, ..default_metrics() };
        assert_eq!(ctx.evaluate(&m), BehavioralState::Emergency);
    }

    #[test]
    fn emergency_from_catastrophic_loss() {
        let mut ctx = StateContext::new();
        let m = TickMetrics { single_tick_loss: 0.06, ..default_metrics() };
        assert_eq!(ctx.evaluate(&m), BehavioralState::Emergency);
    }

    #[test]
    fn explore_to_exploit_requires_2_ticks() {
        let mut ctx = StateContext::new();
        let m = TickMetrics { opportunity_found: true, ..default_metrics() };
        // Tick 1: hysteresis not met
        assert_eq!(ctx.evaluate(&m), BehavioralState::Explore);
        // Tick 2: transition fires
        assert_eq!(ctx.evaluate(&m), BehavioralState::Exploit);
    }

    #[test]
    fn hysteresis_resets_when_trigger_goes_inactive() {
        let mut ctx = StateContext::new();
        let m_opp = TickMetrics { opportunity_found: true, ..default_metrics() };
        let m_no_opp = TickMetrics { opportunity_found: false, ..default_metrics() };

        ctx.evaluate(&m_opp);
        assert_eq!(ctx.trigger_ticks, 1);

        ctx.evaluate(&m_no_opp);
        assert_eq!(ctx.trigger_ticks, 0);

        // Need 2 consecutive again
        ctx.evaluate(&m_opp);
        assert_eq!(ctx.evaluate(&m_opp), BehavioralState::Exploit);
    }

    #[test]
    fn emergency_cannot_go_directly_to_explore() {
        let mut ctx = StateContext::new();
        ctx.current = BehavioralState::Emergency;
        ctx.trigger_ticks = 0;

        let m = TickMetrics {
            bleeding_stopped: true,
            return_avg: 1.0,
            mood_pleasure: 1.0,
            ..default_metrics()
        };

        ctx.evaluate(&m);
        ctx.evaluate(&m);
        let state = ctx.evaluate(&m);
        assert_eq!(state, BehavioralState::Recovery);
    }

    #[test]
    fn full_recovery_path() {
        let mut ctx = StateContext::new();
        ctx.current = BehavioralState::Emergency;
        ctx.trigger_ticks = 0;

        // 3 ticks bleeding stopped -> Recovery
        let m_stable = TickMetrics { bleeding_stopped: true, ..default_metrics() };
        ctx.evaluate(&m_stable);
        ctx.evaluate(&m_stable);
        assert_eq!(ctx.evaluate(&m_stable), BehavioralState::Recovery);

        // Postmortem done -> Cautious
        let m_post = TickMetrics { postmortem_done: true, ..default_metrics() };
        assert_eq!(ctx.evaluate(&m_post), BehavioralState::Cautious);

        // 10 ticks normal performance -> Explore
        let m_normal = TickMetrics { return_avg: 0.01, mood_pleasure: 0.0, ..default_metrics() };
        for _ in 0..9 {
            assert_eq!(ctx.evaluate(&m_normal), BehavioralState::Cautious);
        }
        assert_eq!(ctx.evaluate(&m_normal), BehavioralState::Explore);
    }

    #[test]
    fn consolidate_normal_exit_to_explore() {
        let mut ctx = StateContext::new();
        ctx.current = BehavioralState::Consolidate;

        let m = TickMetrics {
            dream_complete: true,
            anti_knowledge_ratio: 0.1,
            brier_score: 0.2,
            strategies_demoted: 0,
            ..default_metrics()
        };
        assert_eq!(ctx.evaluate(&m), BehavioralState::Explore);
    }

    #[test]
    fn consolidate_alarming_to_cautious() {
        let mut ctx = StateContext::new();
        ctx.current = BehavioralState::Consolidate;

        let m =
            TickMetrics { dream_complete: true, anti_knowledge_ratio: 0.4, ..default_metrics() };
        assert_eq!(ctx.evaluate(&m), BehavioralState::Cautious);
    }

    #[test]
    fn cautious_to_exploit_requires_high_confidence_3_ticks() {
        let mut ctx = StateContext::new();
        ctx.current = BehavioralState::Cautious;
        ctx.trigger_ticks = 0;

        let m = TickMetrics {
            opportunity_found: true,
            opportunity_confidence: 0.95,
            ..default_metrics()
        };

        assert_eq!(ctx.evaluate(&m), BehavioralState::Cautious);
        assert_eq!(ctx.evaluate(&m), BehavioralState::Cautious);
        assert_eq!(ctx.evaluate(&m), BehavioralState::Exploit);
    }

    #[test]
    fn state_modifiers_explore() {
        let mods = StateModifiers::for_state(&BehavioralState::Explore);
        assert_eq!(mods.retrieval_top_k, 20);
        assert!((mods.exploration_rate - 0.30).abs() < 1e-9);
        assert!((mods.risk_budget_multiplier - 1.5).abs() < 1e-9);
    }

    #[test]
    fn state_modifiers_emergency_halts_publication() {
        let mods = StateModifiers::for_state(&BehavioralState::Emergency);
        assert!((mods.publication_conf_threshold - 1.0).abs() < 1e-9);
        assert!((mods.position_size_multiplier - 0.0).abs() < 1e-9);
    }

    #[test]
    fn emergency_from_any_state() {
        for start_state in [
            BehavioralState::Explore,
            BehavioralState::Exploit,
            BehavioralState::Consolidate,
            BehavioralState::Recovery,
            BehavioralState::Cautious,
        ] {
            let mut ctx = StateContext::new();
            ctx.current = start_state;
            ctx.trigger_ticks = 0;

            let m = TickMetrics { drawdown: 0.15, ..default_metrics() };
            assert_eq!(ctx.evaluate(&m), BehavioralState::Emergency);
        }
    }
}
