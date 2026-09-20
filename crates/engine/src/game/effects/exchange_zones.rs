use crate::types::ability::{Effect, EffectError, EffectKind, ResolvedAbility};
use crate::types::events::GameEvent;
use crate::types::game_state::GameState;
use crate::types::identifiers::ObjectId;
use crate::types::player::PlayerId;
use crate::types::zones::Zone;

/// CR 701.12a: Exchange a player's graveyard and library.
///
/// Morality Shift is not equivalent to two sequential `ChangeZoneAll`
/// effects: after the first move, the original library would be mixed into
/// the destination zone scanned by the second move. Capture both populations
/// first, then route each card through the normal zone-change authority. This
/// preserves CR 400.7 object incarnation changes, replacement effects, zone
/// change records, and graveyard-arrival bookkeeping. The chained Shuffle
/// effect randomizes the resulting library after this effect resolves.
pub fn resolve(
    state: &mut GameState,
    ability: &ResolvedAbility,
    events: &mut Vec<GameEvent>,
) -> Result<(), EffectError> {
    let Effect::RuntimeHandled { handler } = &ability.effect else {
        return Ok(());
    };
    if !matches!(
        handler,
        crate::types::ability::RuntimeHandler::MoralityShift
    ) {
        return Ok(());
    }

    let player = ability.controller;
    let (library, graveyard) = player_zone_members(state, player)?;

    // CR 603.10a + CR 614.1 + CR 616.1: announce both directions as one
    // heterogeneous batch. The populations were snapshotted before any move,
    // and the pipeline keeps that one logical zone-change group intact while
    // still consulting per-card replacement effects and pausing safely for a
    // replacement-choice prompt.
    let requests = library
        .into_iter()
        .map(|object_id| {
            crate::game::zone_pipeline::ZoneMoveRequest::effect(
                object_id,
                Zone::Graveyard,
                ability.source_id,
            )
        })
        .chain(graveyard.into_iter().map(|object_id| {
            crate::game::zone_pipeline::ZoneMoveRequest::effect(
                object_id,
                Zone::Library,
                ability.source_id,
            )
        }))
        .collect();

    match crate::game::zone_pipeline::move_objects_simultaneously(state, requests, events) {
        crate::game::zone_pipeline::BatchMoveResult::Done => {
            events.push(GameEvent::EffectResolved {
                kind: EffectKind::RuntimeHandled,
                source_id: ability.source_id,
                subject: None,
            });
        }
        crate::game::zone_pipeline::BatchMoveResult::NeedsChoice => {
            // The batch drain owns the paused effect until the replacement
            // choice settles. Re-emit the ordinary completion marker only
            // after the final card of the exchange has been delivered.
            crate::game::zone_pipeline::defer_completion_on_pause(
                state,
                crate::types::game_state::BatchCompletion::EmitEffectResolved {
                    kind: EffectKind::RuntimeHandled,
                    source_id: ability.source_id,
                },
            );
        }
    }
    Ok(())
}

fn player_zone_members(
    state: &GameState,
    player: PlayerId,
) -> Result<(Vec<ObjectId>, Vec<ObjectId>), EffectError> {
    let player_state = state
        .players
        .iter()
        .find(|candidate| candidate.id == player)
        .ok_or(EffectError::PlayerNotFound)?;
    Ok((
        player_state.library.iter().copied().collect(),
        player_state.graveyard.iter().copied().collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::zones::create_object;
    use crate::types::ability::{Effect, ResolvedAbility, RuntimeHandler};
    use crate::types::identifiers::CardId;

    #[test]
    fn morality_shift_swaps_library_and_graveyard_memberships() {
        let mut state = GameState::new_two_player(42);
        let library_card = create_object(
            &mut state,
            CardId(1),
            PlayerId(0),
            "Library card".to_string(),
            Zone::Library,
        );
        let graveyard_card = create_object(
            &mut state,
            CardId(2),
            PlayerId(0),
            "Graveyard card".to_string(),
            Zone::Graveyard,
        );
        let ability = ResolvedAbility::new(
            Effect::RuntimeHandled {
                handler: RuntimeHandler::MoralityShift,
            },
            vec![],
            ObjectId(100),
            PlayerId(0),
        );
        let mut events = Vec::new();

        resolve(&mut state, &ability, &mut events).unwrap();

        assert_eq!(state.objects[&library_card].zone, Zone::Graveyard);
        assert_eq!(state.objects[&graveyard_card].zone, Zone::Library);
        assert!(state.players[0].library.contains(&graveyard_card));
        assert!(state.players[0].graveyard.contains(&library_card));
        assert!(events.iter().any(|event| matches!(
            event,
            GameEvent::EffectResolved {
                kind: EffectKind::RuntimeHandled,
                ..
            }
        )));
    }
}
