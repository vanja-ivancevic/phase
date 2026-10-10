use super::*;
use crate::game::{engine, zones};
use crate::types::ability::{Effect, ResolvedAbility, TargetFilter, TypeFilter, TypedFilter};
use crate::types::actions::GameAction;
use crate::types::card_type::CoreType;
use crate::types::identifiers::CardId;
use crate::types::phase::Phase;

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);

fn card(state: &mut GameState, owner: PlayerId, name: &str, zone: Zone) -> ObjectId {
    let card_id = CardId(state.next_object_id);
    let id = zones::create_object(state, card_id, owner, name.into(), zone);
    if zone == Zone::Battlefield {
        let object = state.objects.get_mut(&id).unwrap();
        object.card_types.core_types = vec![CoreType::Land];
        object.base_card_types = object.card_types.clone();
    }
    id
}

fn state() -> GameState {
    let mut state = GameState::new_two_player(42);
    state.phase = Phase::PreCombatMain;
    state.turn_number = 2;
    state.waiting_for = WaitingFor::Priority { player: P0 };
    state
}

fn begin(state: &mut GameState, cards: &[ObjectId], destination: Zone) -> Vec<GameEvent> {
    let mut events = Vec::new();
    super::super::move_objects_simultaneously(
        state,
        cards
            .iter()
            .map(|id| ZoneMoveRequest::effect(*id, destination, ObjectId(900)))
            .collect(),
        &mut events,
    );
    events
}

#[test]
fn owner_not_controller_chooses_before_any_arrival_and_preserves_existing_graveyard() {
    let mut state = state();
    let old = card(&mut state, P0, "Older corpse", Zone::Graveyard);
    let forest = card(&mut state, P0, "Forest", Zone::Battlefield);
    let plains = card(&mut state, P0, "Plains", Zone::Battlefield);
    state.objects.get_mut(&forest).unwrap().controller = P1;
    state.objects.get_mut(&plains).unwrap().controller = P1;
    let events = begin(&mut state, &[forest, plains], Zone::Graveyard);
    assert_eq!(state.battlefield, crate::im::vector![forest, plains]);
    assert_eq!(state.players[0].graveyard, crate::im::vector![old]);
    assert!(!events
        .iter()
        .any(|event| matches!(event, GameEvent::ZoneChanged { .. })));
    assert!(matches!(state.waiting_for,
        WaitingFor::GraveyardOrderChoice { player: P0, ref cards, .. }
            if cards == &[forest, plains]));
    let unchanged = state.clone();
    assert!(engine::apply(
        &mut state,
        P1,
        GameAction::SelectCards {
            cards: vec![plains, forest]
        }
    )
    .is_err());
    assert_eq!(state, unchanged);
    engine::apply(
        &mut state,
        P0,
        GameAction::SelectCards {
            cards: vec![plains, forest],
        },
    )
    .unwrap();
    assert_eq!(
        state.players[0].graveyard,
        crate::im::vector![old, plains, forest]
    );
    assert!(state.battlefield.is_empty());
    assert!(state.pending_graveyard_order_choice.is_none());
}

#[test]
fn apnap_owner_choices_survive_save_restore_and_all_finish_before_delivery() {
    let mut state = state();
    state.active_player = P1;
    state.priority_player = P1;
    state.waiting_for = WaitingFor::Priority { player: P1 };
    let a = card(&mut state, P0, "Forest", Zone::Battlefield);
    let b = card(&mut state, P1, "Swamp", Zone::Battlefield);
    let c = card(&mut state, P0, "Plains", Zone::Battlefield);
    let d = card(&mut state, P1, "Island", Zone::Battlefield);
    begin(&mut state, &[a, b, c, d], Zone::Graveyard);
    assert!(matches!(
        state.waiting_for,
        WaitingFor::GraveyardOrderChoice { player: P1, .. }
    ));
    engine::apply(
        &mut state,
        P1,
        GameAction::SelectCards { cards: vec![d, b] },
    )
    .unwrap();
    assert_eq!(state.battlefield, crate::im::vector![a, b, c, d]);
    assert!(state
        .players
        .iter()
        .all(|player| player.graveyard.is_empty()));
    assert!(matches!(
        state.waiting_for,
        WaitingFor::GraveyardOrderChoice { player: P0, .. }
    ));
    let bytes = serde_json::to_vec(&state).unwrap();
    let mut restored: GameState = serde_json::from_slice(&bytes).unwrap();
    engine::apply(
        &mut restored,
        P0,
        GameAction::SelectCards { cards: vec![c, a] },
    )
    .unwrap();
    assert_eq!(restored.players[0].graveyard, crate::im::vector![c, a]);
    assert_eq!(restored.players[1].graveyard, crate::im::vector![d, b]);
    assert_eq!(restored.priority_player, P1);
    assert!(matches!(
        restored.waiting_for,
        WaitingFor::Priority { player: P1 }
    ));
    assert!(restored.pending_graveyard_order_choice.is_none());
}

#[test]
fn incomplete_duplicate_foreign_and_stale_orders_are_rejected_unchanged() {
    let mut state = state();
    let a = card(&mut state, P0, "Forest", Zone::Battlefield);
    let b = card(&mut state, P0, "Plains", Zone::Battlefield);
    let foreign = card(&mut state, P1, "Swamp", Zone::Battlefield);
    begin(&mut state, &[a, b], Zone::Graveyard);
    for chosen in [vec![a], vec![a, a], vec![a, foreign]] {
        let unchanged = state.clone();
        assert!(engine::apply(&mut state, P0, GameAction::SelectCards { cards: chosen }).is_err());
        assert_eq!(state, unchanged);
    }
    let original = state.objects[&a].incarnation;
    zones::move_to_zone(&mut state, a, Zone::Hand, &mut Vec::new());
    zones::move_to_zone(&mut state, a, Zone::Battlefield, &mut Vec::new());
    assert_ne!(state.objects[&a].incarnation, original);
    let unchanged = state.clone();
    assert!(engine::apply(
        &mut state,
        P0,
        GameAction::SelectCards { cards: vec![b, a] }
    )
    .is_err());
    assert_eq!(state, unchanged);
}

#[test]
fn singleton_owners_tokens_and_face_up_exile_do_not_get_graveyard_order_choices() {
    let mut state = state();
    let a = card(&mut state, P0, "Forest", Zone::Battlefield);
    let b = card(&mut state, P1, "Swamp", Zone::Battlefield);
    let token = card(&mut state, P0, "Land token", Zone::Battlefield);
    state.objects.get_mut(&token).unwrap().is_token = true;
    begin(&mut state, &[a, b, token], Zone::Graveyard);
    assert_eq!(state.players[0].graveyard, crate::im::vector![a, token]);
    assert_eq!(state.players[1].graveyard, crate::im::vector![b]);
    assert!(matches!(state.waiting_for, WaitingFor::Priority { .. }));
    let x = card(&mut state, P0, "Exiled forest", Zone::Battlefield);
    let y = card(&mut state, P0, "Exiled plains", Zone::Battlefield);
    begin(&mut state, &[x, y], Zone::Exile);
    assert_eq!(state.exile, crate::im::vector![x, y]);
    assert!(matches!(state.waiting_for, WaitingFor::Priority { .. }));
    assert!(state.pending_graveyard_order_choice.is_none());
}

#[test]
fn destroy_all_lands_defers_moves_and_resolved_event_until_owner_permutation() {
    let mut state = state();
    let forest = card(&mut state, P0, "Forest", Zone::Battlefield);
    let plains = card(&mut state, P0, "Plains", Zone::Battlefield);
    let swamp = card(&mut state, P1, "Swamp", Zone::Battlefield);
    let ability = ResolvedAbility::new(
        Effect::DestroyAll {
            target: TargetFilter::Typed(TypedFilter {
                type_filters: vec![TypeFilter::Land],
                controller: None,
                properties: vec![],
            }),
            cant_regenerate: false,
        },
        vec![],
        ObjectId(900),
        P0,
    );
    let mut events = Vec::new();
    crate::game::effects::destroy::resolve_all(&mut state, &ability, &mut events).unwrap();
    assert_eq!(state.battlefield, crate::im::vector![forest, plains, swamp]);
    assert!(!events.iter().any(|event| matches!(
        event,
        GameEvent::ZoneChanged { .. } | GameEvent::EffectResolved { .. }
    )));
    let result = engine::apply(
        &mut state,
        P0,
        GameAction::SelectCards {
            cards: vec![plains, forest],
        },
    )
    .unwrap();
    assert_eq!(
        state.players[0].graveyard,
        crate::im::vector![plains, forest]
    );
    assert_eq!(state.players[1].graveyard, crate::im::vector![swamp]);
    assert_eq!(
        result
            .events
            .iter()
            .filter(|event| matches!(
                event,
                GameEvent::EffectResolved {
                    kind: crate::types::ability::EffectKind::DestroyAll,
                    ..
                }
            ))
            .count(),
        1
    );
    assert_eq!(
        result
            .events
            .iter()
            .filter(|event| matches!(event, GameEvent::CreatureDestroyed { .. }))
            .count(),
        3
    );
}

#[test]
fn conceding_during_owner_order_ends_duel_without_delivering_the_pending_instruction() {
    let mut state = state();
    let a = card(&mut state, P0, "Forest", Zone::Battlefield);
    let b = card(&mut state, P0, "Plains", Zone::Battlefield);
    begin(&mut state, &[a, b], Zone::Graveyard);
    engine::apply(&mut state, P0, GameAction::Concede { player_id: P0 }).unwrap();
    assert!(matches!(
        state.waiting_for,
        WaitingFor::GameOver { winner: Some(P1) }
    ));
    assert_eq!(state.battlefield, crate::im::vector![a, b]);
    assert!(state.players[0].graveyard.is_empty());
    assert!(state.pending_graveyard_order_choice.is_none());
}

#[test]
fn changed_singleton_member_invalidates_the_frozen_multi_owner_instruction() {
    let mut state = state();
    let a = card(&mut state, P0, "Forest", Zone::Battlefield);
    let b = card(&mut state, P0, "Plains", Zone::Battlefield);
    let singleton = card(&mut state, P1, "Swamp", Zone::Battlefield);
    begin(&mut state, &[a, b, singleton], Zone::Graveyard);
    zones::move_to_zone(&mut state, singleton, Zone::Hand, &mut Vec::new());
    let unchanged = state.clone();
    assert!(engine::apply(
        &mut state,
        P0,
        GameAction::SelectCards { cards: vec![b, a] }
    )
    .is_err());
    assert_eq!(state, unchanged);
    assert_eq!(state.battlefield, crate::im::vector![a, b]);
    assert!(state
        .players
        .iter()
        .all(|player| player.graveyard.is_empty()));
}

#[test]
fn departing_owner_is_pruned_without_losing_surviving_owner_order_or_arrivals() {
    let mut state = GameState::new(state().format_config.clone(), 3, 42);
    state.phase = Phase::PreCombatMain;
    state.turn_number = 2;
    state.waiting_for = WaitingFor::Priority { player: P0 };
    let a = card(&mut state, P0, "Departing forest", Zone::Battlefield);
    let b = card(&mut state, P0, "Departing plains", Zone::Battlefield);
    let c = card(&mut state, P1, "Surviving swamp", Zone::Battlefield);
    let d = card(&mut state, P1, "Surviving island", Zone::Battlefield);
    let e = card(
        &mut state,
        PlayerId(2),
        "Surviving mountain",
        Zone::Battlefield,
    );
    begin(&mut state, &[a, b, c, d, e], Zone::Graveyard);
    engine::apply(&mut state, P0, GameAction::Concede { player_id: P0 }).unwrap();
    assert!(matches!(
        state.waiting_for,
        WaitingFor::GraveyardOrderChoice { player: P1, .. }
    ));
    assert_eq!(state.battlefield, crate::im::vector![c, d, e]);
    engine::apply(
        &mut state,
        P1,
        GameAction::SelectCards { cards: vec![d, c] },
    )
    .unwrap();
    assert_eq!(state.players[1].graveyard, crate::im::vector![d, c]);
    assert_eq!(state.players[2].graveyard, crate::im::vector![e]);
    assert!(state.battlefield.is_empty());
    assert!(state.pending_graveyard_order_choice.is_none());
}

#[test]
fn viewer_projections_drop_delivery_authority_and_do_not_reveal_future_owner_library() {
    let mut state = state();
    let a = card(&mut state, P0, "Own forest", Zone::Library);
    let b = card(&mut state, P0, "Own plains", Zone::Library);
    let c = card(&mut state, P1, "Secret swamp", Zone::Library);
    let d = card(&mut state, P1, "Secret island", Zone::Library);
    begin(&mut state, &[a, b, c, d], Zone::Graveyard);
    for projected in [
        crate::game::visibility::filter_state_for_viewer(&state, P0),
        crate::game::visibility::filter_state_for_unseated_viewer(&state),
    ] {
        assert!(projected.pending_graveyard_order_choice.is_none());
        assert_ne!(projected.objects[&c].name, state.objects[&c].name);
        assert_ne!(projected.objects[&d].name, state.objects[&d].name);
    }
    assert!(state.pending_graveyard_order_choice.is_some());
    assert_eq!(state.objects[&c].zone, Zone::Library);
}

#[test]
fn mass_hand_to_graveyard_keeps_owner_order_and_completes_instruction_once() {
    let mut state = state();
    let a = card(&mut state, P0, "First card", Zone::Hand);
    let b = card(&mut state, P0, "Second card", Zone::Hand);
    let ability = ResolvedAbility::new(
        Effect::ChangeZoneAll {
            origin: Some(Zone::Hand),
            destination: Zone::Graveyard,
            target: TargetFilter::None,
            enters_under: None,
            enter_tapped: crate::types::zones::EtbTapState::Unspecified,
            enters_attacking: false,
            enter_with_counters: vec![],
            face_down_profile: None,
            library_position: None,
            library_shuffle: crate::types::ability::MassLibraryShuffleMode::PerObject,
            random_order: false,
        },
        vec![],
        ObjectId(900),
        P0,
    );
    let mut events = Vec::new();
    crate::game::effects::change_zone::resolve_all(&mut state, &ability, &mut events).unwrap();
    assert_eq!(state.players[0].hand, crate::im::vector![a, b]);
    assert!(!events.iter().any(|event| matches!(
        event,
        GameEvent::ZoneChanged { .. } | GameEvent::EffectResolved { .. }
    )));
    let result = engine::apply(
        &mut state,
        P0,
        GameAction::SelectCards { cards: vec![b, a] },
    )
    .unwrap();
    assert_eq!(state.players[0].graveyard, crate::im::vector![b, a]);
    assert!(state.players[0].hand.is_empty());
    assert_eq!(state.last_effect_count, Some(2));
    assert_eq!(
        result
            .events
            .iter()
            .filter(|event| matches!(
                event,
                GameEvent::EffectResolved {
                    kind: crate::types::ability::EffectKind::ChangeZoneAll,
                    ..
                }
            ))
            .count(),
        1
    );
}

#[test]
fn destroy_owner_permutation_survives_two_inner_replacement_parks_and_finishes_once() {
    use crate::types::ability::{
        AbilityDefinition, AbilityKind, ReplacementDefinition, ReplacementMode,
    };
    use crate::types::replacements::ReplacementEvent;
    use crate::types::zones::EtbTapState;
    let mut state = state();
    let a = card(&mut state, P0, "Forest", Zone::Battlefield);
    let b = card(&mut state, P0, "Plains", Zone::Battlefield);
    let redirect = zones::create_object(
        &mut state,
        CardId(900),
        P0,
        "Optional redirect".into(),
        Zone::Battlefield,
    );
    state
        .objects
        .get_mut(&redirect)
        .unwrap()
        .replacement_definitions
        .push(
            ReplacementDefinition::new(ReplacementEvent::Moved)
                .mode(ReplacementMode::Optional { decline: None })
                .execute(AbilityDefinition::new(
                    AbilityKind::Spell,
                    Effect::ChangeZone {
                        origin: None,
                        destination: Zone::Exile,
                        target: TargetFilter::SelfRef,
                        owner_library: false,
                        enter_transformed: false,
                        enters_under: None,
                        enter_tapped: EtbTapState::Unspecified,
                        enters_attacking: false,
                        up_to: false,
                        enter_with_counters: vec![],
                        conditional_enter_with_counters: vec![],
                        face_down_profile: None,
                        enters_modified_if: None,
                    },
                ))
                .destination_zone(Zone::Graveyard),
        );
    let ability = ResolvedAbility::new(
        Effect::DestroyAll {
            target: TargetFilter::Typed(TypedFilter {
                type_filters: vec![TypeFilter::Land],
                controller: None,
                properties: vec![],
            }),
            cant_regenerate: false,
        },
        vec![],
        redirect,
        P0,
    );
    crate::game::effects::destroy::resolve_all(&mut state, &ability, &mut Vec::new()).unwrap();
    let first = engine::apply(
        &mut state,
        P0,
        GameAction::SelectCards { cards: vec![b, a] },
    )
    .unwrap();
    assert!(matches!(
        state.waiting_for,
        WaitingFor::ReplacementChoice { .. }
    ));
    assert_eq!(state.objects[&a].zone, Zone::Battlefield);
    assert_eq!(state.objects[&b].zone, Zone::Battlefield);
    assert!(!first
        .events
        .iter()
        .any(|event| matches!(event, GameEvent::EffectResolved { .. })));
    let middle = engine::apply(&mut state, P0, GameAction::ChooseReplacement { index: 1 }).unwrap();
    assert!(matches!(
        state.waiting_for,
        WaitingFor::ReplacementChoice { .. }
    ));
    assert_eq!(state.objects[&b].zone, Zone::Graveyard);
    assert_eq!(state.objects[&a].zone, Zone::Battlefield);
    assert!(!middle
        .events
        .iter()
        .any(|event| matches!(event, GameEvent::EffectResolved { .. })));
    let last = engine::apply(&mut state, P0, GameAction::ChooseReplacement { index: 1 }).unwrap();
    assert_eq!(state.players[0].graveyard, crate::im::vector![b, a]);
    assert_eq!(
        last.events
            .iter()
            .filter(|event| matches!(
                event,
                GameEvent::EffectResolved {
                    kind: crate::types::ability::EffectKind::DestroyAll,
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(state.active_batch_delivery().is_none());
}
