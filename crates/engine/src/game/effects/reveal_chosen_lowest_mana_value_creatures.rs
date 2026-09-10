//! CR 101.4 + CR 701.20 + CR 608.2c: reveal per-player hidden choices, then
//! put every tied lowest-mana-value chosen creature card onto the battlefield.
//!
//! This consumes the fresh tracked set produced by `ChooseFromZone`'s
//! per-player hidden-zone selection flow. It deliberately models a reusable
//! semantic consequence rather than a card-name-specific resolver.

use crate::game::zone_pipeline::{self, BatchMoveResult, ZoneMoveRequest};
use crate::types::ability::{Effect, EffectError, EffectKind, ResolvedAbility};
use crate::types::card_type::CoreType;
use crate::types::events::GameEvent;
use crate::types::game_state::GameState;
use crate::types::identifiers::ObjectId;
use crate::types::zones::Zone;

pub fn resolve(
    state: &mut GameState,
    ability: &ResolvedAbility,
    events: &mut Vec<GameEvent>,
) -> Result<(), EffectError> {
    if !matches!(ability.effect, Effect::RevealChosenLowestManaValueCreatures) {
        return Err(EffectError::MissingParam(
            "RevealChosenLowestManaValueCreatures".to_string(),
        ));
    }

    // A per-player choice always publishes a fresh set, even when no player
    // can choose. Do not fall back to an unrelated earlier selection.
    let chosen: Vec<ObjectId> = state
        .chain_tracked_set_id
        .and_then(|id| state.tracked_object_sets.get(&id).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|id| {
            state
                .objects
                .get(id)
                .is_some_and(|object| object.zone == Zone::Hand)
        })
        .collect();

    // Choices remain private until every player has committed. Reveal all of
    // them before testing creature quality or moving any winner.
    state.last_revealed_ids = chosen.clone();
    for &id in &chosen {
        state.revealed_cards.insert(id);
    }
    for player in crate::game::players::apnap_order(state) {
        let card_ids: Vec<ObjectId> = chosen
            .iter()
            .copied()
            .filter(|id| {
                state
                    .objects
                    .get(id)
                    .is_some_and(|object| object.owner == player)
            })
            .collect();
        if card_ids.is_empty() {
            continue;
        }
        let card_names = card_ids
            .iter()
            .filter_map(|id| state.objects.get(id).map(|object| object.name.clone()))
            .collect();
        events.push(GameEvent::CardsRevealed {
            player,
            card_ids,
            card_names,
        });
    }

    let creatures: Vec<ObjectId> = chosen
        .into_iter()
        .filter(|id| {
            state
                .objects
                .get(id)
                .is_some_and(|object| object.card_types.core_types.contains(&CoreType::Creature))
        })
        .collect();
    let Some(lowest_mana_value) = creatures
        .iter()
        .filter_map(|id| {
            state
                .objects
                .get(id)
                .map(|object| object.effective_mana_value())
        })
        .min()
    else {
        state.last_effect_count = Some(0);
        events.push(GameEvent::EffectResolved {
            kind: EffectKind::RevealChosenLowestManaValueCreatures,
            source_id: ability.source_id,
            subject: None,
        });
        return Ok(());
    };

    let winners: Vec<ObjectId> = creatures
        .into_iter()
        .filter(|id| {
            state
                .objects
                .get(id)
                .is_some_and(|object| object.effective_mana_value() == lowest_mana_value)
        })
        .collect();
    state.last_effect_count = Some(winners.len() as i32);
    let requests = winners
        .into_iter()
        .map(|id| ZoneMoveRequest::effect(id, Zone::Battlefield, ability.source_id))
        .collect();
    if matches!(
        zone_pipeline::move_objects_simultaneously(state, requests, events),
        BatchMoveResult::NeedsChoice
    ) {
        return Ok(());
    }

    events.push(GameEvent::EffectResolved {
        kind: EffectKind::RevealChosenLowestManaValueCreatures,
        source_id: ability.source_id,
        subject: None,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::zones::create_object;
    use crate::types::identifiers::{CardId, TrackedSetId};
    use crate::types::mana::ManaCost;
    use crate::types::player::PlayerId;

    fn hand_card(
        state: &mut GameState,
        owner: PlayerId,
        name: &str,
        mana_value: u32,
        creature: bool,
    ) -> ObjectId {
        let id = create_object(
            state,
            CardId(state.objects.len() as u64 + 1),
            owner,
            name.to_string(),
            Zone::Hand,
        );
        let object = state.objects.get_mut(&id).expect("created card exists");
        object.mana_cost = ManaCost::generic(mana_value);
        if creature {
            object.card_types.core_types.push(CoreType::Creature);
        }
        id
    }

    #[test]
    fn reveals_all_choices_then_moves_every_tied_lowest_creature() {
        let mut state = GameState::new_two_player(77);
        let low_a = hand_card(&mut state, PlayerId(0), "Low A", 2, true);
        let low_b = hand_card(&mut state, PlayerId(1), "Low B", 2, true);
        let higher = hand_card(&mut state, PlayerId(0), "Higher", 4, true);
        let noncreature = hand_card(&mut state, PlayerId(1), "Spell", 0, false);
        let selected = vec![low_a, low_b, higher, noncreature];
        state.chain_tracked_set_id = Some(TrackedSetId(1));
        state
            .tracked_object_sets
            .insert(TrackedSetId(1), selected.clone());
        let ability = ResolvedAbility::new(
            Effect::RevealChosenLowestManaValueCreatures,
            vec![],
            ObjectId(900),
            PlayerId(0),
        );
        let mut events = Vec::new();

        resolve(&mut state, &ability, &mut events).expect("resolution succeeds");

        assert!(state.battlefield.contains(&low_a));
        assert!(state.battlefield.contains(&low_b));
        assert!(state.players[0].hand.contains(&higher));
        assert!(state.players[1].hand.contains(&noncreature));
        assert_eq!(state.last_revealed_ids, selected);
        let revealed: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                GameEvent::CardsRevealed { card_ids, .. } => Some(card_ids.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(revealed.len(), 2);
        let revealed_ids: Vec<ObjectId> = revealed.into_iter().flatten().collect();
        assert_eq!(revealed_ids.len(), selected.len());
        assert!(selected.iter().all(|id| revealed_ids.contains(id)));
    }
}
