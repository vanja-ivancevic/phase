//! Consumer regressions for source-linked battlefield returns (CR 607.2c).

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::TargetRef;
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::{GameState, StackEntryKind, WaitingFor};
use engine::types::identifiers::{ObjectId, ObjectIncarnationRef};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const DIABOLIC_SERVITUDE: &str = concat!(
    "When this enchantment enters, return target creature card from your graveyard to the battlefield.\n",
    "When the creature put onto the battlefield with this enchantment dies, exile it and return this enchantment to its owner's hand.\n",
    "When this enchantment leaves the battlefield, exile the creature put onto the battlefield with this enchantment."
);

struct Fixture {
    runner: GameRunner,
    source: ObjectId,
    creature: ObjectId,
    other_grave: ObjectId,
    bystander: ObjectId,
    kill: ObjectId,
    second_kill: ObjectId,
    remove_source: ObjectId,
    blink: ObjectId,
    reanimate: ObjectId,
    exile_grave: ObjectId,
    thief: ObjectId,
}

fn add_servitude(scenario: &mut GameScenario, name: &str) -> ObjectId {
    scenario
        .add_spell_to_hand(P0, name, false)
        .as_enchantment()
        .from_oracle_text(DIABOLIC_SERVITUDE)
        .id()
}

fn fixture(with_target: bool) -> Fixture {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let source = add_servitude(&mut scenario, "Diabolic Servitude");
    let creature = if with_target {
        scenario
            .add_creature_to_graveyard(P0, "Returned Grizzly Bears", 2, 2)
            .id()
    } else {
        scenario
            .add_creature_to_exile(P0, "Unreturned Grizzly Bears", 2, 2)
            .id()
    };
    let other_grave = if with_target {
        scenario
            .add_creature_to_graveyard(P0, "Other Grizzly Bears", 2, 2)
            .id()
    } else {
        scenario
            .add_creature_to_exile(P0, "Other Grizzly Bears", 2, 2)
            .id()
    };
    let bystander = scenario.add_creature(P0, "Unlinked Bear", 2, 2).id();
    let kill = scenario
        .add_spell_to_hand_from_oracle(P0, "Death Probe", true, "Destroy target creature.")
        .id();
    let second_kill = scenario
        .add_spell_to_hand_from_oracle(P0, "Second Death Probe", true, "Destroy target creature.")
        .id();
    let remove_source = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Source Departure Probe",
            true,
            "Destroy target enchantment.",
        )
        .id();
    let blink = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Incarnation Probe",
            true,
            "Exile target permanent, then return it to the battlefield under its owner's control.",
        )
        .id();
    let reanimate = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Independent Return Probe",
            true,
            "Return target creature card from your graveyard to the battlefield.",
        )
        .id();
    let exile_grave = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Graveyard Departure Probe",
            true,
            "Exile target card from a graveyard.",
        )
        .id();
    let thief = scenario
        .add_creature_from_oracle(
            P1,
            "Control Probe",
            1,
            1,
            "{0}: Gain control of target enchantment.",
        )
        .id();
    Fixture {
        runner: scenario.build(),
        source,
        creature,
        other_grave,
        bystander,
        kill,
        second_kill,
        remove_source,
        blink,
        reanimate,
        exile_grave,
        thief,
    }
}

fn identity(state: &GameState, id: ObjectId) -> ObjectIncarnationRef {
    ObjectIncarnationRef::from_object(&state.objects[&id])
}

fn assert_zone(state: &GameState, id: ObjectId, zone: Zone) {
    let object = &state.objects[&id];
    assert_eq!(object.zone, zone, "object {id:?}");
    let player = &state.players[object.owner.0 as usize];
    let members = match zone {
        Zone::Battlefield => &state.battlefield,
        Zone::Graveyard => &player.graveyard,
        Zone::Hand => &player.hand,
        Zone::Exile => &state.exile,
        _ => panic!("unsupported assertion zone {zone:?}"),
    };
    assert!(
        members.contains(&id),
        "{id:?} must inhabit the concrete {zone:?} container"
    );
}

fn pass_one_resolution(runner: &mut GameRunner) {
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::Priority { .. }
    ));
    runner
        .act(GameAction::PassPriority)
        .expect("first player passes priority");
    runner
        .act(GameAction::PassPriority)
        .expect("second player passes priority");
}

fn return_creature(fixture: &mut Fixture) {
    fixture
        .runner
        .cast(fixture.source)
        .target_object(fixture.creature)
        .resolve();
    let state = fixture.runner.state();
    assert!(state.stack.is_empty(), "ETB resolution must finish");
    assert_zone(state, fixture.source, Zone::Battlefield);
    assert_zone(state, fixture.creature, Zone::Battlefield);
    assert_zone(state, fixture.other_grave, Zone::Graveyard);
    assert_zone(state, fixture.bystander, Zone::Battlefield);
    let source = identity(state, fixture.source);
    let recipient = identity(state, fixture.creature);
    assert_eq!(state.battlefield_return_links.len(), 1);
    assert_eq!(state.battlefield_return_links[0].source, source);
    assert_eq!(state.battlefield_return_links[0].recipient, recipient);
}

fn assert_departure_trigger(
    state: &GameState,
    source: ObjectId,
    departing: ObjectIncarnationRef,
    destination: Zone,
) {
    assert_eq!(
        state.stack.len(),
        1,
        "exactly one source-linked trigger must be pending"
    );
    let entry = &state.stack[0];
    assert_eq!(entry.source_id, source);
    let StackEntryKind::TriggeredAbility {
        trigger_event:
            Some(GameEvent::ZoneChanged {
                object_id,
                from,
                to,
                record,
            }),
        ..
    } = &entry.kind
    else {
        panic!(
            "expected an actual zone-change triggered ability, got {:?}",
            entry.kind
        );
    };
    assert_eq!(*object_id, departing.object_id);
    assert_eq!(*from, Some(Zone::Battlefield));
    assert_eq!(*to, destination);
    assert_eq!(
        record
            .trigger_source_context()
            .expect("departure carries LKI")
            .identity
            .reference,
        departing,
    );
    assert!(matches!(state.waiting_for, WaitingFor::Priority { .. }));
}

#[test]
fn authentic_servitude_death_exiles_successor_and_bounces_to_owner_not_controller() {
    let mut f = fixture(true);
    return_creature(&mut f);
    f.runner
        .act(GameAction::PassPriority)
        .expect("P0 gives P1 priority");
    f.runner
        .activate(f.thief, 0)
        .target_object(f.source)
        .resolve();
    assert_eq!(f.runner.state().objects[&f.source].controller, P1);
    assert_eq!(f.runner.state().objects[&f.source].owner, P0);
    let returned = identity(f.runner.state(), f.creature);
    f.runner.cast(f.kill).target_object(f.creature).commit();
    pass_one_resolution(&mut f.runner);
    assert_zone(f.runner.state(), f.creature, Zone::Graveyard);
    assert_zone(f.runner.state(), f.source, Zone::Battlefield);
    assert_departure_trigger(f.runner.state(), f.source, returned, Zone::Graveyard);
    assert_eq!(f.runner.state().stack[0].controller, P1);
    pass_one_resolution(&mut f.runner);
    f.runner.advance_until_stack_empty();
    assert_zone(f.runner.state(), f.creature, Zone::Exile);
    assert_zone(f.runner.state(), f.source, Zone::Hand);
    assert!(!f.runner.state().players[1].hand.contains(&f.source));
    assert_zone(f.runner.state(), f.other_grave, Zone::Graveyard);
    assert_zone(f.runner.state(), f.bystander, Zone::Battlefield);
    assert!(f.runner.state().battlefield_return_links.is_empty());
}

#[test]
fn authentic_servitude_source_departure_exiles_only_its_returned_incarnation() {
    let mut f = fixture(true);
    return_creature(&mut f);
    let source = identity(f.runner.state(), f.source);
    let returned = identity(f.runner.state(), f.creature);
    f.runner
        .cast(f.remove_source)
        .target_object(f.source)
        .commit();
    pass_one_resolution(&mut f.runner);
    assert_zone(f.runner.state(), f.source, Zone::Graveyard);
    assert_zone(f.runner.state(), f.creature, Zone::Battlefield);
    assert_departure_trigger(f.runner.state(), f.source, source, Zone::Graveyard);
    pass_one_resolution(&mut f.runner);
    assert!(f.runner.state().stack.is_empty());
    assert_zone(f.runner.state(), f.creature, Zone::Exile);
    assert_ne!(identity(f.runner.state(), f.creature), returned);
    assert_zone(f.runner.state(), f.source, Zone::Graveyard);
    assert_zone(f.runner.state(), f.bystander, Zone::Battlefield);
    assert_zone(f.runner.state(), f.other_grave, Zone::Graveyard);
}

#[test]
fn creature_blink_breaks_old_link_for_both_death_and_source_departure() {
    let mut f = fixture(true);
    return_creature(&mut f);
    let old = identity(f.runner.state(), f.creature);
    f.runner.cast(f.blink).target_object(f.creature).resolve();
    let replacement = identity(f.runner.state(), f.creature);
    assert_ne!(old, replacement);
    assert_zone(f.runner.state(), f.creature, Zone::Battlefield);
    f.runner.cast(f.kill).target_object(f.creature).commit();
    pass_one_resolution(&mut f.runner);
    assert!(
        f.runner.state().stack.is_empty(),
        "a new creature incarnation is not the linked creature"
    );
    assert_zone(f.runner.state(), f.creature, Zone::Graveyard);
    assert_zone(f.runner.state(), f.source, Zone::Battlefield);
    f.runner
        .cast(f.reanimate)
        .target_object(f.creature)
        .resolve();
    let independently_returned = identity(f.runner.state(), f.creature);
    f.runner
        .cast(f.remove_source)
        .target_object(f.source)
        .resolve();
    assert_zone(f.runner.state(), f.creature, Zone::Battlefield);
    assert_eq!(
        identity(f.runner.state(), f.creature),
        independently_returned
    );
    assert_zone(f.runner.state(), f.source, Zone::Graveyard);
}

#[test]
fn pending_death_exile_cannot_hit_a_later_graveyard_incarnation() {
    let mut f = fixture(true);
    return_creature(&mut f);
    let returned = identity(f.runner.state(), f.creature);
    f.runner.cast(f.kill).target_object(f.creature).commit();
    pass_one_resolution(&mut f.runner);
    assert_departure_trigger(f.runner.state(), f.source, returned, Zone::Graveyard);
    let original_graveyard_successor = identity(f.runner.state(), f.creature);
    f.runner
        .cast(f.reanimate)
        .target_object(f.creature)
        .commit();
    pass_one_resolution(&mut f.runner);
    assert_zone(f.runner.state(), f.creature, Zone::Battlefield);
    f.runner
        .cast(f.second_kill)
        .target_object(f.creature)
        .commit();
    pass_one_resolution(&mut f.runner);
    assert_departure_trigger(f.runner.state(), f.source, returned, Zone::Graveyard);
    let later_graveyard_successor = identity(f.runner.state(), f.creature);
    assert_ne!(original_graveyard_successor, later_graveyard_successor);
    pass_one_resolution(&mut f.runner);
    f.runner.advance_until_stack_empty();
    assert_zone(f.runner.state(), f.creature, Zone::Graveyard);
    assert_eq!(
        identity(f.runner.state(), f.creature),
        later_graveyard_successor
    );
    assert_zone(f.runner.state(), f.source, Zone::Hand);
}

#[test]
fn pending_source_leave_exile_cannot_hit_a_blinked_battlefield_incarnation() {
    let mut f = fixture(true);
    return_creature(&mut f);
    let source = identity(f.runner.state(), f.source);
    let old = identity(f.runner.state(), f.creature);
    f.runner
        .cast(f.remove_source)
        .target_object(f.source)
        .commit();
    pass_one_resolution(&mut f.runner);
    assert_departure_trigger(f.runner.state(), f.source, source, Zone::Graveyard);
    f.runner.cast(f.blink).target_object(f.creature).commit();
    pass_one_resolution(&mut f.runner);
    let replacement = identity(f.runner.state(), f.creature);
    assert_ne!(replacement, old);
    assert_departure_trigger(f.runner.state(), f.source, source, Zone::Graveyard);
    pass_one_resolution(&mut f.runner);
    assert_zone(f.runner.state(), f.creature, Zone::Battlefield);
    assert_eq!(identity(f.runner.state(), f.creature), replacement);
    assert_zone(f.runner.state(), f.source, Zone::Graveyard);
    assert!(f.runner.state().stack.is_empty());
}

#[test]
fn source_blink_creates_a_new_link_and_old_death_cannot_bounce_new_source() {
    let mut f = fixture(true);
    return_creature(&mut f);
    let old_source = identity(f.runner.state(), f.source);
    let returned = identity(f.runner.state(), f.creature);
    f.runner.cast(f.kill).target_object(f.creature).commit();
    pass_one_resolution(&mut f.runner);
    assert_departure_trigger(f.runner.state(), f.source, returned, Zone::Graveyard);
    f.runner
        .cast(f.blink)
        .target_objects(&[f.source, f.other_grave])
        .resolve();
    let new_source = identity(f.runner.state(), f.source);
    assert_ne!(new_source, old_source);
    assert_zone(f.runner.state(), f.source, Zone::Battlefield);
    assert_zone(f.runner.state(), f.creature, Zone::Exile);
    assert_zone(f.runner.state(), f.other_grave, Zone::Battlefield);
    assert_eq!(f.runner.state().battlefield_return_links.len(), 1);
    assert_eq!(
        f.runner.state().battlefield_return_links[0].source,
        new_source
    );
    assert_eq!(
        f.runner.state().battlefield_return_links[0].recipient,
        identity(f.runner.state(), f.other_grave),
    );
    f.runner
        .cast(f.second_kill)
        .target_object(f.other_grave)
        .resolve();
    assert_zone(f.runner.state(), f.other_grave, Zone::Exile);
    assert_zone(f.runner.state(), f.source, Zone::Hand);
    assert_zone(f.runner.state(), f.bystander, Zone::Battlefield);
}

#[test]
fn illegal_etb_target_never_links_another_graveyard_card_or_bystander() {
    let mut f = fixture(true);
    f.runner.cast(f.source).commit();
    pass_one_resolution(&mut f.runner);
    assert!(matches!(
        f.runner.state().waiting_for,
        WaitingFor::TriggerTargetSelection { .. }
    ));
    f.runner
        .act(GameAction::ChooseTarget {
            target: Some(TargetRef::Object(f.creature)),
        })
        .expect("choose the ETB's actual graveyard target");
    f.runner
        .cast(f.exile_grave)
        .target_object(f.creature)
        .resolve();
    assert_zone(f.runner.state(), f.creature, Zone::Exile);
    assert_zone(f.runner.state(), f.other_grave, Zone::Graveyard);
    assert_zone(f.runner.state(), f.bystander, Zone::Battlefield);
    assert_zone(f.runner.state(), f.source, Zone::Battlefield);
    assert!(f.runner.state().battlefield_return_links.is_empty());
    f.runner
        .cast(f.remove_source)
        .target_object(f.source)
        .resolve();
    assert_zone(f.runner.state(), f.source, Zone::Graveyard);
    assert_zone(f.runner.state(), f.other_grave, Zone::Graveyard);
    assert_zone(f.runner.state(), f.bystander, Zone::Battlefield);
}

#[test]
fn missing_etb_target_leaves_an_empty_link_not_a_source_fallback() {
    let mut f = fixture(false);
    f.runner.cast(f.source).resolve();
    assert_zone(f.runner.state(), f.source, Zone::Battlefield);
    assert!(f.runner.state().battlefield_return_links.is_empty());
    f.runner
        .cast(f.remove_source)
        .target_object(f.source)
        .resolve();
    assert_zone(f.runner.state(), f.source, Zone::Graveyard);
    assert_zone(f.runner.state(), f.bystander, Zone::Battlefield);
    assert_zone(f.runner.state(), f.creature, Zone::Exile);
    assert!(f.runner.state().stack.is_empty());
}

#[test]
fn generic_renamed_sources_have_separate_linked_recipient_sets() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let first = add_servitude(&mut scenario, "First Linked Return Enchantment");
    let second = add_servitude(&mut scenario, "Second Linked Return Enchantment");
    let bear = scenario.add_creature_to_graveyard(P0, "Bear", 2, 2).id();
    let wolf = scenario.add_creature_to_graveyard(P0, "Wolf", 2, 2).id();
    let removal = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Source Departure Probe",
            true,
            "Destroy target enchantment.",
        )
        .id();
    let mut runner = scenario.build();
    runner.cast(first).target_object(bear).resolve();
    runner.cast(second).target_object(wolf).resolve();
    assert_zone(runner.state(), bear, Zone::Battlefield);
    assert_zone(runner.state(), wolf, Zone::Battlefield);
    assert_eq!(runner.state().battlefield_return_links.len(), 2);
    let wolf_incarnation = identity(runner.state(), wolf);
    runner.cast(removal).target_object(first).resolve();
    assert_zone(runner.state(), first, Zone::Graveyard);
    assert_zone(runner.state(), bear, Zone::Exile);
    assert_zone(runner.state(), second, Zone::Battlefield);
    assert_zone(runner.state(), wolf, Zone::Battlefield);
    assert_eq!(identity(runner.state(), wolf), wolf_incarnation);
    assert_eq!(runner.state().battlefield_return_links.len(), 1);
    assert_eq!(
        runner.state().battlefield_return_links[0].source,
        identity(runner.state(), second)
    );
}

#[test]
fn unrelated_reanimation_does_not_install_source_links() {
    let mut f = fixture(true);
    f.runner
        .cast(f.reanimate)
        .target_object(f.creature)
        .resolve();
    assert_zone(f.runner.state(), f.creature, Zone::Battlefield);
    assert_zone(f.runner.state(), f.source, Zone::Hand);
    assert!(f.runner.state().battlefield_return_links.is_empty());
    f.runner.cast(f.kill).target_object(f.creature).resolve();
    assert_zone(f.runner.state(), f.creature, Zone::Graveyard);
    assert_zone(f.runner.state(), f.source, Zone::Hand);
    assert!(f.runner.state().stack.is_empty());
}

#[test]
fn pending_old_etb_return_cannot_install_a_link_on_a_blinked_source() {
    let mut f = fixture(true);
    f.runner.cast(f.source).commit();
    pass_one_resolution(&mut f.runner);
    assert!(matches!(
        f.runner.state().waiting_for,
        WaitingFor::TriggerTargetSelection { .. }
    ));
    f.runner
        .act(GameAction::ChooseTarget {
            target: Some(TargetRef::Object(f.creature)),
        })
        .expect("choose the old source's ETB target");
    let old_source = identity(f.runner.state(), f.source);
    f.runner
        .cast(f.blink)
        .target_objects(&[f.source, f.other_grave])
        .resolve();
    let new_source = identity(f.runner.state(), f.source);
    assert_ne!(old_source, new_source);
    assert_zone(f.runner.state(), f.creature, Zone::Battlefield);
    assert_zone(f.runner.state(), f.other_grave, Zone::Battlefield);
    assert_eq!(f.runner.state().battlefield_return_links.len(), 1);
    assert_eq!(
        f.runner.state().battlefield_return_links[0].source,
        new_source
    );
    assert_eq!(
        f.runner.state().battlefield_return_links[0].recipient,
        identity(f.runner.state(), f.other_grave),
    );
    let unlinked_return = identity(f.runner.state(), f.creature);
    f.runner
        .cast(f.remove_source)
        .target_object(f.source)
        .resolve();
    assert_zone(f.runner.state(), f.other_grave, Zone::Exile);
    assert_zone(f.runner.state(), f.creature, Zone::Battlefield);
    assert_eq!(identity(f.runner.state(), f.creature), unlinked_return);
    assert_zone(f.runner.state(), f.source, Zone::Graveyard);
}

#[test]
fn simultaneous_board_wipe_preserves_linked_incarnations_in_either_trigger_order() {
    for order in [[0, 1], [1, 0]] {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let source = add_servitude(&mut scenario, "Board Wipe Linked Return Enchantment");
        let creature = scenario
            .add_creature_to_graveyard(P0, "Returned Bear", 2, 2)
            .id();
        let bystander = scenario.add_creature(P0, "Unlinked Wolf", 2, 2).id();
        let wipe = scenario
            .add_spell_to_hand_from_oracle(
                P0,
                "Simultaneous Departure Probe",
                true,
                "Destroy all nonland permanents.",
            )
            .id();
        let mut runner = scenario.build();
        runner.cast(source).target_object(creature).resolve();
        let old_source = identity(runner.state(), source);
        let returned = identity(runner.state(), creature);
        runner.cast(wipe).commit();
        pass_one_resolution(&mut runner);
        assert_zone(runner.state(), source, Zone::Graveyard);
        assert_zone(runner.state(), creature, Zone::Graveyard);
        assert_zone(runner.state(), bystander, Zone::Graveyard);
        assert!(runner.state().battlefield_return_links.is_empty());
        let successor = identity(runner.state(), creature);
        assert_ne!(successor, returned);
        assert_ne!(identity(runner.state(), source), old_source);

        // Both triggers belong to P0; CR 603.3b permits either order.
        let WaitingFor::OrderTriggers { player, triggers } = &runner.state().waiting_for else {
            panic!("simultaneous linked departures must offer their controller a trigger order");
        };
        assert_eq!(*player, P0);
        assert_eq!(triggers.len(), 2);
        runner
            .act(GameAction::OrderTriggers {
                order: order.to_vec(),
            })
            .expect("accept either offered controller order");
        assert_eq!(runner.state().stack.len(), 2);
        let departure_first = matches!(
            &runner.state().stack.last().expect("two queued triggers").kind,
            StackEntryKind::TriggeredAbility {
                trigger_event: Some(GameEvent::ZoneChanged { object_id, .. }),
                ..
            } if *object_id == source
        );
        for entry in &runner.state().stack {
            assert_eq!(entry.controller, P0);
            assert_eq!(entry.source_id, source);
            let StackEntryKind::TriggeredAbility {
                trigger_event:
                    Some(GameEvent::ZoneChanged {
                        object_id,
                        from,
                        to,
                        record,
                    }),
                ..
            } = &entry.kind
            else {
                panic!("the wipe must queue actual zone-change triggers");
            };
            assert_eq!(*from, Some(Zone::Battlefield));
            assert_eq!(*to, Zone::Graveyard);
            let departed = record
                .trigger_source_context()
                .expect("departure LKI")
                .identity
                .reference;
            assert_eq!(
                departed,
                if *object_id == source {
                    old_source
                } else {
                    returned
                }
            );
            assert!(*object_id == source || *object_id == creature);
        }

        pass_one_resolution(&mut runner);
        assert_eq!(runner.state().stack.len(), 1);
        if departure_first {
            // The former battlefield referent cannot follow a zone change.
            assert_zone(runner.state(), creature, Zone::Graveyard);
            assert_eq!(identity(runner.state(), creature), successor);
        } else {
            // The death trigger may find only the immediate graveyard successor.
            assert_zone(runner.state(), creature, Zone::Exile);
        }
        pass_one_resolution(&mut runner);
        assert!(runner.state().stack.is_empty());
        assert_zone(runner.state(), creature, Zone::Exile);
        assert_ne!(identity(runner.state(), creature), successor);
        assert_zone(runner.state(), source, Zone::Graveyard);
        assert_zone(runner.state(), bystander, Zone::Graveyard);
    }
}
