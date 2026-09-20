use crate::types::ability::{
    Effect, EffectError, EffectKind, ResolvedAbility, RuntimeHandler, TargetRef,
};
use crate::types::events::GameEvent;
use crate::types::game_state::GameState;

/// CR 701.20e: privately look at the identities of the resolved object target(s).
///
/// This is deliberately separate from `Reveal`: a face-down permanent or other
/// hidden object remains hidden from every player except the ability controller.
/// `resolved_targets` is the shared authority for explicit and anaphoric target
/// references, so this handler also works for a chained `ParentTarget` form.
pub fn resolve(
    state: &mut GameState,
    ability: &ResolvedAbility,
    events: &mut Vec<GameEvent>,
) -> Result<(), EffectError> {
    let Effect::RuntimeHandled {
        handler: RuntimeHandler::LookAtObject { target },
    } = &ability.effect
    else {
        return Ok(());
    };

    let object_ids: Vec<_> = crate::game::targeting::resolved_targets(ability, target, state)
        .into_iter()
        .filter_map(|target| match target {
            TargetRef::Object(object_id) if state.objects.contains_key(&object_id) => {
                Some(object_id)
            }
            TargetRef::Object(_) | TargetRef::Player(_) => None,
        })
        .collect();

    if !object_ids.is_empty() {
        state.remember_card_identities(
            crate::game::turn_control::decision_audience_for_player(state, ability.controller),
            &object_ids,
        );
        state.private_look_ids = object_ids;
        state.private_look_player = Some(ability.controller);
    }

    events.push(GameEvent::EffectResolved {
        kind: EffectKind::RuntimeHandled,
        source_id: ability.source_id,
        subject: None,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::zones::create_object;
    use crate::types::ability::{Effect, ResolvedAbility, RuntimeHandler, TargetFilter};
    use crate::types::identifiers::{CardId, ObjectId};
    use crate::types::player::PlayerId;
    use crate::types::zones::Zone;

    #[test]
    fn private_object_look_records_identity_for_controller_only() {
        let mut state = GameState::new_two_player(42);
        let object_id = create_object(
            &mut state,
            CardId(1),
            PlayerId(1),
            "Face-down creature".to_string(),
            Zone::Battlefield,
        );
        state.objects.get_mut(&object_id).unwrap().face_down = true;
        let ability = ResolvedAbility::new(
            Effect::RuntimeHandled {
                handler: RuntimeHandler::LookAtObject {
                    target: TargetFilter::ParentTarget,
                },
            },
            vec![TargetRef::Object(object_id)],
            ObjectId(100),
            PlayerId(0),
        );
        let mut events = Vec::new();

        resolve(&mut state, &ability, &mut events).unwrap();

        assert_eq!(state.private_look_ids.as_slice(), &[object_id]);
        assert_eq!(state.private_look_player, Some(PlayerId(0)));
        assert!(events.iter().any(|event| matches!(
            event,
            GameEvent::EffectResolved {
                kind: EffectKind::RuntimeHandled,
                ..
            }
        )));
    }
}
