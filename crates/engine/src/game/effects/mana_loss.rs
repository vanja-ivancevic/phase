//! Instruction-driven loss of unspent mana (CR 106.4).
//!
//! This module is intentionally separate from `turns`: a spell such as Mana
//! Short causes mana to be lost during resolution, while `turns` owns the
//! automatic emptying that happens as steps and phases end. In particular,
//! `StepEndManaAction` replacements are not applicable to this instruction.

use crate::types::ability::{Effect, EffectError, EffectKind, ResolvedAbility};
use crate::types::events::GameEvent;
use crate::types::game_state::GameState;
use crate::types::mana::{apply_empty_mana_pool_decisions, UnitDecision, UnitDisposition};

/// Resolve `Effect::LoseAllUnspentMana`.
///
/// CR 106.4 calls this loss, rather than spending or destroying mana. The
/// shared disposition applier is the single authority for removing individual
/// pool units and producing `ManaPoolEmptied` events, so card instructions and
/// step-end draining cannot diverge on event attribution.
pub fn resolve(
    state: &mut GameState,
    ability: &ResolvedAbility,
    events: &mut Vec<GameEvent>,
) -> Result<(), EffectError> {
    let Effect::LoseAllUnspentMana { player } = &ability.effect else {
        return Err(EffectError::MissingParam("LoseAllUnspentMana".to_string()));
    };

    let player_id = super::resolve_player_for_context_ref(state, ability, player);
    let units: Vec<UnitDecision> = state
        .players
        .iter()
        .find(|candidate| candidate.id == player_id)
        .map(|candidate| {
            candidate
                .mana_pool
                .mana
                .iter()
                .enumerate()
                .map(|(pool_index, unit)| UnitDecision {
                    pool_index,
                    color: unit.color,
                    disposition: UnitDisposition::Drop,
                })
                .collect()
        })
        .unwrap_or_default();

    apply_empty_mana_pool_decisions(state, player_id, &units, events);
    events.push(GameEvent::EffectResolved {
        kind: EffectKind::LoseAllUnspentMana,
        source_id: ability.source_id,
        subject: None,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ability::{TargetFilter, TargetRef};
    use crate::types::identifiers::ObjectId;
    use crate::types::mana::{ManaType, ManaUnit};
    use crate::types::player::PlayerId;

    #[test]
    fn target_player_loses_every_color_of_unspent_mana() {
        let mut state = GameState::new_two_player(42);
        state.players[1]
            .mana_pool
            .add(ManaUnit::new(ManaType::Blue, ObjectId(10), false, Vec::new()));
        state.players[1]
            .mana_pool
            .add(ManaUnit::new(ManaType::Red, ObjectId(11), false, Vec::new()));
        let ability = ResolvedAbility::new(
            Effect::LoseAllUnspentMana {
                player: TargetFilter::Player,
            },
            vec![TargetRef::Player(PlayerId(1))],
            ObjectId(99),
            PlayerId(0),
        );
        let mut events = Vec::new();

        resolve(&mut state, &ability, &mut events).unwrap();

        assert!(state.players[0].mana_pool.mana.is_empty());
        assert!(state.players[1].mana_pool.mana.is_empty());
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, GameEvent::ManaPoolEmptied { player_id: PlayerId(1), .. }))
                .count(),
            2,
            "each lost mana unit must retain its ordinary loss event"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            GameEvent::EffectResolved {
                kind: EffectKind::LoseAllUnspentMana,
                source_id: ObjectId(99),
                ..
            }
        )));
    }

    /// CR 608.2c: a sequential "that player loses all unspent mana" clause
    /// inherits the preceding player target (Mana Short's current wording).
    #[test]
    fn inherited_player_target_loses_every_color_of_unspent_mana() {
        let mut state = GameState::new_two_player(42);
        state.players[1]
            .mana_pool
            .add(ManaUnit::new(ManaType::Blue, ObjectId(10), false, Vec::new()));
        state.players[1]
            .mana_pool
            .add(ManaUnit::new(ManaType::Red, ObjectId(11), false, Vec::new()));
        let ability = ResolvedAbility::new(
            Effect::LoseAllUnspentMana {
                player: TargetFilter::ParentTarget,
            },
            vec![TargetRef::Player(PlayerId(1))],
            ObjectId(99),
            PlayerId(0),
        );
        let mut events = Vec::new();

        resolve(&mut state, &ability, &mut events).unwrap();

        assert!(state.players[0].mana_pool.mana.is_empty());
        assert!(state.players[1].mana_pool.mana.is_empty());
        assert!(events.iter().any(|event| matches!(
            event,
            GameEvent::ManaPoolEmptied { player_id: PlayerId(1), .. }
        )));
    }
}
