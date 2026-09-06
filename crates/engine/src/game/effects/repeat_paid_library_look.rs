//! Repeated paid private-library looks (Lim-Dûl's Vault's Oracle-text family).

use crate::types::ability::{Effect, EffectError, EffectKind, ResolvedAbility};
use crate::types::events::GameEvent;
use crate::types::game_state::{DeferredLifeCostResume, GameState, WaitingFor};
use crate::types::identifiers::ObjectId;
use crate::types::player::PlayerId;

/// Start the first private look. The printed form reads "your library", so
/// the resolving controller both looks and owns the library.
pub fn resolve(
    state: &mut GameState,
    ability: &ResolvedAbility,
    _events: &mut Vec<GameEvent>,
) -> Result<(), EffectError> {
    let (count, life_payment) = match &ability.effect {
        Effect::RepeatPaidLibraryLook => (5, 1),
        _ => return Ok(()),
    };
    offer_payment_for_top_cards(
        state,
        ability.controller,
        ability.source_id,
        count,
        life_payment,
    );
    Ok(())
}

fn offer_payment_for_top_cards(
    state: &mut GameState,
    player: PlayerId,
    source_id: ObjectId,
    count: usize,
    life_payment: u32,
) {
    let cards: Vec<_> = state
        .players
        .iter()
        .find(|candidate| candidate.id == player)
        .expect("resolving player exists")
        .library
        .iter()
        .take(count)
        .copied()
        .collect();
    state.remember_card_identities(
        crate::game::turn_control::decision_audience_for_player(state, player),
        &cards,
    );
    state.private_look_ids = cards.clone();
    state.private_look_player = Some(player);
    state.waiting_for = WaitingFor::RepeatPaidLibraryLookPayment {
        player,
        source_id,
        cards,
        life_payment,
    };
}

/// Handle the paid-loop's optional decision. The cost travels through the
/// normal life-cost authority so replacement effects and payment prohibitions
/// retain their ordinary semantics.
pub(crate) fn handle_payment(
    state: &mut GameState,
    player: PlayerId,
    source_id: ObjectId,
    cards: Vec<ObjectId>,
    life_payment: u32,
    accept: bool,
    events: &mut Vec<GameEvent>,
) -> Result<WaitingFor, crate::game::engine::EngineError> {
    if !accept {
        return offer_final_top_choice(state, player, source_id, cards, events);
    }
    let resume_at_resolution_depth = state.resolution_stack.len();
    match crate::game::life_costs::pay_life_as_cost(state, player, life_payment, events) {
        crate::game::life_costs::PayLifeCostResult::Paid { .. } => {
            Ok(resume_after_paid_life(state, player, source_id, cards))
        }
        crate::game::life_costs::PayLifeCostResult::PaidWithDeferredSubstitution { .. }
        | crate::game::life_costs::PayLifeCostResult::DeferredReplacementChoice { .. } => {
            state.pending_deferred_life_cost_resume =
                Some(DeferredLifeCostResume::RepeatPaidLibraryLook {
                    player,
                    source_id,
                    cards,
                    resume_at_resolution_depth,
                });
            Ok(state.waiting_for.clone())
        }
        crate::game::life_costs::PayLifeCostResult::InsufficientLife
        | crate::game::life_costs::PayLifeCostResult::Prohibited => {
            Err(crate::game::engine::EngineError::InvalidAction(
                "player cannot pay the offered life cost".to_string(),
            ))
        }
    }
}

/// Called after the standard replacement pipeline has settled. The cost is
/// already paid, so this must only expose the ordered bottom choice.
pub(crate) fn resume_after_paid_life(
    state: &mut GameState,
    player: PlayerId,
    source_id: ObjectId,
    cards: Vec<ObjectId>,
) -> WaitingFor {
    state.waiting_for = WaitingFor::ReorderLibraryChoice {
        player,
        cards,
        top: false,
        source_id: Some(source_id),
    };
    state.waiting_for.clone()
}

/// Validate and commit one full permutation of the looked-at group. A bottom
/// order starts the next look; a top order completes the resolving effect.
pub(crate) fn handle_reorder(
    state: &mut GameState,
    player: PlayerId,
    cards: Vec<ObjectId>,
    top: bool,
    source_id: Option<ObjectId>,
    ordered: Vec<ObjectId>,
    events: &mut Vec<GameEvent>,
) -> Result<WaitingFor, crate::game::engine::EngineError> {
    if !is_full_permutation(&ordered, &cards) {
        return Err(crate::game::engine::EngineError::InvalidAction(
            "library reorder must contain every looked-at card exactly once".to_string(),
        ));
    }
    crate::game::zones::reorder_within_library(state, player, &ordered, top.then_some(0));
    if top {
        events.push(GameEvent::EffectResolved {
            kind: EffectKind::RepeatPaidLibraryLook,
            source_id: source_id.unwrap_or(ObjectId(0)),
            subject: None,
        });
        return Ok(
            crate::game::engine_resolution_choices::finish_with_continuation(state, player, events),
        );
    }
    let source_id = source_id.expect("paid library look bottom ordering retains source");
    // This structural Oracle form fixes both values at five / one. A short
    // library is still handled naturally by the look helper's `take(5)`.
    offer_payment_for_top_cards(state, player, source_id, 5, 1);
    Ok(state.waiting_for.clone())
}

fn offer_final_top_choice(
    state: &mut GameState,
    player: PlayerId,
    source_id: ObjectId,
    cards: Vec<ObjectId>,
    events: &mut Vec<GameEvent>,
) -> Result<WaitingFor, crate::game::engine::EngineError> {
    crate::game::effects::change_zone::shuffle_library(state, player, events);
    state.waiting_for = WaitingFor::ReorderLibraryChoice {
        player,
        cards,
        top: true,
        source_id: Some(source_id),
    };
    Ok(state.waiting_for.clone())
}

fn is_full_permutation(ordered: &[ObjectId], cards: &[ObjectId]) -> bool {
    ordered.len() == cards.len()
        && ordered.iter().all(|id| cards.contains(id))
        && ordered
            .iter()
            .enumerate()
            .all(|(index, id)| !ordered[..index].contains(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::zones::create_object;
    use crate::parser::oracle_effect::parse_effect_chain;
    use crate::types::ability::AbilityKind;
    use crate::types::identifiers::CardId;
    use crate::types::zones::Zone;

    const VAULT_TEXT: &str = "Look at the top five cards of your library. As many times as you choose, you may pay 1 life, put those cards on the bottom of your library in any order, then look at the top five cards of your library. Then shuffle and put the last cards you looked at this way on top in any order.";

    fn ability() -> ResolvedAbility {
        ResolvedAbility::new(
            Effect::RepeatPaidLibraryLook,
            vec![],
            ObjectId(900),
            PlayerId(0),
        )
    }

    fn add_library(state: &mut GameState, count: u64) -> Vec<ObjectId> {
        (0..count)
            .map(|index| {
                create_object(
                    state,
                    CardId(index + 1),
                    PlayerId(0),
                    format!("Card {index}"),
                    Zone::Library,
                )
            })
            .collect()
    }

    #[test]
    fn parses_the_complete_vault_grammar_as_one_semantic_effect() {
        let parsed = parse_effect_chain(VAULT_TEXT, AbilityKind::Spell);
        assert!(matches!(*parsed.effect, Effect::RepeatPaidLibraryLook));
        assert!(parsed.sub_ability.is_none());
        let near_miss = parse_effect_chain(
            "Look at the top four cards of your library. As many times as you choose, you may pay 1 life, put those cards on the bottom of your library in any order, then look at the top five cards of your library. Then shuffle and put the last cards you looked at this way on top in any order.",
            AbilityKind::Spell,
        );
        assert!(!matches!(*near_miss.effect, Effect::RepeatPaidLibraryLook));
    }

    #[test]
    fn decline_shuffles_then_requires_a_full_ordered_top_group() {
        let mut state = GameState::new_two_player(7);
        let initial = add_library(&mut state, 6);
        let mut events = Vec::new();
        resolve(&mut state, &ability(), &mut events).unwrap();
        let WaitingFor::RepeatPaidLibraryLookPayment { cards, .. } = &state.waiting_for else {
            panic!("expected paid-look prompt")
        };
        assert_eq!(cards, &initial[..5]);
        let top = vec![initial[4], initial[3], initial[2], initial[1], initial[0]];
        handle_payment(
            &mut state,
            PlayerId(0),
            ObjectId(900),
            initial[..5].to_vec(),
            1,
            false,
            &mut events,
        )
        .unwrap();
        assert!(matches!(
            state.waiting_for,
            WaitingFor::ReorderLibraryChoice { top: true, .. }
        ));
        handle_reorder(
            &mut state,
            PlayerId(0),
            initial[..5].to_vec(),
            true,
            Some(ObjectId(900)),
            top.clone(),
            &mut events,
        )
        .unwrap();
        let library: Vec<_> = state.players[0].library.iter().copied().collect();
        assert_eq!(&library[..5], top.as_slice());
        assert!(events.iter().any(|event| matches!(
            event,
            GameEvent::EffectResolved {
                kind: EffectKind::RepeatPaidLibraryLook,
                ..
            }
        )));
    }

    #[test]
    fn paid_iteration_loses_life_orders_bottom_then_looks_again() {
        let mut state = GameState::new_two_player(9);
        let initial = add_library(&mut state, 6);
        let mut events = Vec::new();
        resolve(&mut state, &ability(), &mut events).unwrap();
        let bottom = vec![initial[4], initial[3], initial[2], initial[1], initial[0]];
        handle_payment(
            &mut state,
            PlayerId(0),
            ObjectId(900),
            initial[..5].to_vec(),
            1,
            true,
            &mut events,
        )
        .unwrap();
        assert_eq!(state.players[0].life, 19);
        handle_reorder(
            &mut state,
            PlayerId(0),
            initial[..5].to_vec(),
            false,
            Some(ObjectId(900)),
            bottom.clone(),
            &mut events,
        )
        .unwrap();
        let WaitingFor::RepeatPaidLibraryLookPayment { cards, .. } = &state.waiting_for else {
            panic!("expected next paid-look prompt")
        };
        assert_eq!(
            cards,
            &vec![initial[5], initial[4], initial[3], initial[2], initial[1]]
        );
        let library: Vec<_> = state.players[0].library.iter().copied().collect();
        assert_eq!(&library[1..], bottom.as_slice());
    }

    #[test]
    fn reorder_rejects_duplicate_or_foreign_cards() {
        let mut state = GameState::new_two_player(11);
        let cards = add_library(&mut state, 3);
        let mut events = Vec::new();
        let error = handle_reorder(
            &mut state,
            PlayerId(0),
            cards.clone(),
            false,
            Some(ObjectId(900)),
            vec![cards[0], cards[0], cards[1]],
            &mut events,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            crate::game::engine::EngineError::InvalidAction(_)
        ));
    }
}
