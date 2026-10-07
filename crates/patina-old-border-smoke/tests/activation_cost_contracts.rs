//! Consumer regressions for announced activation costs and their issued choices.

use std::collections::{BTreeMap, BTreeSet};

use engine::ai_support::AiDecisionContract;
use engine::game::casting::can_activate_ability_now;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::TargetRef;
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::{PayCostKind, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::{ExileCostSourceZone, Zone};

const SHAMAN: &str = concat!(
    "{T}, Discard a card: Target creature gets +1/+1 until end of turn.\n",
    "{T}, Sacrifice a creature: Target creature gets +2/+2 until end of turn."
);
const NIGHT_SOIL: &str =
    "{1}, Exile two creature cards from a single graveyard: Create a 1/1 green Saproling creature token.";

fn act(runner: &mut GameRunner, action: GameAction, events: &mut Vec<GameEvent>) {
    events.extend(runner.act(action).expect("real action must execute").events);
}

fn resolve(runner: &mut GameRunner, events: &mut Vec<GameEvent>) {
    assert_eq!(
        runner.state().stack.len(),
        1,
        "one paid activation reaches the stack"
    );
    act(runner, GameAction::PassPriority, events);
    act(runner, GameAction::PassPriority, events);
    assert!(
        runner.state().stack.is_empty(),
        "ordinary priority passes resolve the ability"
    );
}

fn assert_refused_without_commit(runner: &mut GameRunner, action: GameAction) {
    let resources = |runner: &GameRunner| {
        runner
            .state()
            .players
            .iter()
            .map(|player| {
                (
                    player.id,
                    player.life,
                    player
                        .mana_pool
                        .mana
                        .iter()
                        .map(|unit| unit.color)
                        .collect::<Vec<_>>(),
                    player.hand.clone(),
                    player.library.clone(),
                    player.graveyard.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let objects = |runner: &GameRunner| {
        runner
            .state()
            .objects
            .iter()
            .map(|(id, object)| {
                (
                    *id,
                    (
                        object.zone,
                        object.tapped,
                        object.power,
                        object.toughness,
                        object.owner,
                        object.controller,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let prompt = |runner: &GameRunner| match &runner.state().waiting_for {
        WaitingFor::PayCost {
            choices,
            count,
            min_count,
            ..
        }
        | WaitingFor::EffectZoneChoice {
            cards: choices,
            count,
            min_count,
            ..
        } => Some((choices.clone(), *count, *min_count)),
        _ => None,
    };
    let before_resources = resources(runner);
    let before_objects = objects(runner);
    let before_prompt = prompt(runner);
    let before_owner = runner.state().waiting_for.acting_player();
    let before_kind = std::mem::discriminant(&runner.state().waiting_for);
    let before_stack = runner
        .state()
        .stack
        .iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    let before_pending = runner
        .state()
        .pending_cast
        .as_ref()
        .map(|pending| pending.object_id);
    assert!(
        runner.act(action).is_err(),
        "illegal payment must be refused"
    );
    assert_eq!(resources(runner), before_resources);
    assert_eq!(objects(runner), before_objects);
    assert_eq!(prompt(runner), before_prompt);
    assert_eq!(runner.state().waiting_for.acting_player(), before_owner);
    assert_eq!(
        std::mem::discriminant(&runner.state().waiting_for),
        before_kind
    );
    assert_eq!(
        runner
            .state()
            .stack
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        before_stack
    );
    assert_eq!(
        runner
            .state()
            .pending_cast
            .as_ref()
            .map(|pending| pending.object_id),
        before_pending
    );
}

fn assert_issued(runner: &GameRunner, action: &GameAction) {
    let contract = AiDecisionContract::issue(runner.state(), P0);
    assert!(contract.contains_action(runner.state(), action));
    assert!(contract.permits(runner.state(), P0, action));
}

fn shaman_fixture() -> (GameRunner, ObjectId, ObjectId, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let source = scenario
        .add_creature_from_oracle(P0, "Blighted Shaman", 1, 1, SHAMAN)
        .id();
    let sacrifice = scenario.add_creature(P0, "Grizzly Bears", 2, 2).id();
    let target = scenario.add_creature(P1, "Hill Giant", 3, 3).id();
    (scenario.build(), source, sacrifice, target)
}

fn announce_shaman(
    runner: &mut GameRunner,
    source: ObjectId,
    target: ObjectId,
    events: &mut Vec<GameEvent>,
) {
    let activation = GameAction::ActivateAbility {
        source_id: source,
        ability_index: 1,
    };
    assert_issued(runner, &activation);
    act(runner, activation, events);
    if matches!(
        runner.state().waiting_for,
        WaitingFor::TargetSelection { .. }
    ) {
        act(
            runner,
            GameAction::ChooseTarget {
                target: Some(TargetRef::Object(target)),
            },
            events,
        );
    }
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::PayCost {
            kind: PayCostKind::Sacrifice,
            count: 1,
            min_count: 1,
            ..
        }
    ));
    assert!(
        !runner.state().objects[&source].tapped,
        "announcement is not payment"
    );
}

#[test]
fn blighted_shaman_can_tap_then_sacrifice_itself_exactly_once() {
    let (mut runner, source, other, target) = shaman_fixture();
    let mut events = Vec::new();
    announce_shaman(&mut runner, source, target, &mut events);
    assert_refused_without_commit(&mut runner, GameAction::SelectCards { cards: vec![] });
    assert_refused_without_commit(
        &mut runner,
        GameAction::SelectCards {
            cards: vec![source, source],
        },
    );
    let payment = GameAction::SelectCards {
        cards: vec![source],
    };
    assert_issued(&runner, &payment);
    act(&mut runner, payment, &mut events);
    assert_eq!(runner.state().objects[&source].zone, Zone::Graveyard);
    assert_eq!(runner.state().objects[&other].zone, Zone::Battlefield);
    assert!(!can_activate_ability_now(runner.state(), P0, source, 1));
    resolve(&mut runner, &mut events);
    assert_eq!(
        (
            runner.state().objects[&target].power,
            runner.state().objects[&target].toughness
        ),
        (Some(5), Some(5))
    );
    let taps: Vec<_> = events
        .iter()
        .enumerate()
        .filter_map(|(i, event)| {
            matches!(event, GameEvent::PermanentTapped { object_id, .. } if *object_id == source)
                .then_some(i)
        })
        .collect();
    let moves: Vec<_> = events.iter().enumerate().filter_map(|(i, event)|
        matches!(event, GameEvent::ZoneChanged { object_id, from: Some(Zone::Battlefield), to: Zone::Graveyard, .. } if *object_id == source).then_some(i)).collect();
    assert_eq!(
        taps.len(),
        1,
        "the tap is paid once and emits its real event"
    );
    assert_eq!(moves.len(), 1, "the sacrifice is committed once");
    assert!(
        taps[0] < moves[0],
        "tap payment precedes the self-sacrifice zone move"
    );
    assert!(runner.state().pending_cast.is_none());
    assert_eq!(
        (
            runner.state().players[0].life,
            runner.state().players[1].life
        ),
        (20, 20)
    );
}

#[test]
fn blighted_shaman_sacrificing_another_creature_retains_the_tap_payment() {
    let (mut runner, source, other, target) = shaman_fixture();
    let mut events = Vec::new();
    announce_shaman(&mut runner, source, target, &mut events);
    let payment = GameAction::SelectCards { cards: vec![other] };
    assert_issued(&runner, &payment);
    act(&mut runner, payment, &mut events);
    assert_eq!(runner.state().objects[&source].zone, Zone::Battlefield);
    assert!(runner.state().objects[&source].tapped);
    assert_eq!(runner.state().objects[&other].zone, Zone::Graveyard);
    resolve(&mut runner, &mut events);
    assert_eq!(runner.state().objects[&target].power, Some(5));
    assert_eq!(events.iter().filter(|event| matches!(event, GameEvent::PermanentTapped { object_id, .. } if *object_id == source)).count(), 1);
    assert!(!can_activate_ability_now(runner.state(), P0, source, 1));
    assert_refused_without_commit(
        &mut runner,
        GameAction::ActivateAbility {
            source_id: source,
            ability_index: 1,
        },
    );
}

#[test]
fn source_tap_cost_refuses_tapped_and_dead_sources_before_any_payment() {
    let (mut runner, source, other, _) = shaman_fixture();
    runner.state_mut().objects.get_mut(&source).unwrap().tapped = true;
    assert!(!can_activate_ability_now(runner.state(), P0, source, 1));
    assert_refused_without_commit(
        &mut runner,
        GameAction::ActivateAbility {
            source_id: source,
            ability_index: 1,
        },
    );
    assert_eq!(runner.state().objects[&other].zone, Zone::Battlefield);

    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let dead = scenario
        .add_creature_to_graveyard(P0, "Blighted Shaman", 1, 1)
        .from_oracle_text(SHAMAN)
        .id();
    scenario.add_creature(P0, "Grizzly Bears", 2, 2);
    let mut runner = scenario.build();
    assert!(!can_activate_ability_now(runner.state(), P0, dead, 1));
    assert_refused_without_commit(
        &mut runner,
        GameAction::ActivateAbility {
            source_id: dead,
            ability_index: 1,
        },
    );
}

fn soil_fixture(
    oracle: &str,
    own_count: usize,
    opposing_count: usize,
) -> (GameRunner, ObjectId, Vec<ObjectId>, Vec<ObjectId>) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(
        P0,
        vec![ManaUnit::new(
            ManaType::Colorless,
            ObjectId(9001),
            false,
            vec![],
        )],
    );
    let source = scenario
        .add_enchantment_from_oracle(P0, "Night Soil", oracle)
        .id();
    let own = (0..own_count)
        .map(|i| {
            scenario
                .add_creature_to_graveyard(P0, &format!("Own Bear {i}"), 2, 2)
                .id()
        })
        .collect();
    let opposing = (0..opposing_count)
        .map(|i| {
            scenario
                .add_creature_to_graveyard(P1, &format!("Opposing Bear {i}"), 2, 2)
                .id()
        })
        .collect();
    scenario.add_land_to_graveyard(P0, "Forest");
    scenario.add_land_to_graveyard(P1, "Forest");
    (scenario.build(), source, own, opposing)
}

fn announce_soil(runner: &mut GameRunner, source: ObjectId, events: &mut Vec<GameEvent>) {
    let activation = GameAction::ActivateAbility {
        source_id: source,
        ability_index: 0,
    };
    assert_issued(runner, &activation);
    act(runner, activation, events);
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::PayCost {
            kind: PayCostKind::ExileFromZone {
                zone: ExileCostSourceZone::Graveyard
            },
            count: 2,
            min_count: 2,
            ..
        }
    ));
}

#[test]
fn night_soil_issues_only_exact_same_graveyard_pairs_and_executes_every_pair() {
    let (mut runner, source, own, opposing) = soil_fixture(NIGHT_SOIL, 3, 2);
    let mut events = Vec::new();
    announce_soil(&mut runner, source, &mut events);
    let contract = AiDecisionContract::issue(runner.state(), P0);
    let pairs: Vec<_> = contract
        .candidates
        .iter()
        .filter_map(|candidate| match &candidate.action {
            GameAction::SelectCards { cards } => Some(cards.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        pairs.len(),
        4,
        "three own pairs and one opponent pair are payable"
    );
    assert!(pairs
        .iter()
        .any(|cards| cards.iter().all(|id| opposing.contains(id))));
    for cards in pairs {
        assert_eq!(cards.len(), 2);
        assert!(
            cards.iter().all(|id| own.contains(id)) || cards.iter().all(|id| opposing.contains(id))
        );
        let mut branch = GameRunner::from_state(runner.state().clone());
        let mut branch_events = events.clone();
        let payment = GameAction::SelectCards {
            cards: cards.clone(),
        };
        assert_issued(&branch, &payment);
        act(&mut branch, payment, &mut branch_events);
        for id in &cards {
            assert_eq!(branch.state().objects[id].zone, Zone::Exile);
        }
        assert_eq!(
            branch.state().players[0].mana_pool.mana.len(),
            0,
            "one generic mana is committed once"
        );
        resolve(&mut branch, &mut branch_events);
        let tokens: Vec<_> = branch
            .state()
            .battlefield
            .iter()
            .filter_map(|id| {
                let object = &branch.state().objects[id];
                (object.is_token && object.controller == P0).then_some(object)
            })
            .collect();
        assert_eq!(tokens.len(), 1, "one resolution creates one Saproling");
        assert_eq!((tokens[0].power, tokens[0].toughness), (Some(1), Some(1)));
        for id in &cards {
            assert_eq!(branch_events.iter().filter(|event| matches!(event,
                GameEvent::ZoneChanged { object_id, from: Some(Zone::Graveyard), to: Zone::Exile, .. } if object_id == id)).count(), 1);
        }
        assert!(branch.state().pending_cast.is_none());
        assert_eq!(
            (
                branch.state().players[0].life,
                branch.state().players[1].life
            ),
            (20, 20)
        );
    }
    for cards in [
        vec![],
        vec![own[0]],
        own.clone(),
        vec![own[0], opposing[0]],
        vec![own[0], own[0]],
    ] {
        let illegal = GameAction::SelectCards { cards };
        assert!(!contract.contains_action(runner.state(), &illegal));
        assert!(!contract.permits(runner.state(), P0, &illegal));
        assert_refused_without_commit(&mut runner, illegal);
    }
}

#[test]
fn night_soil_opponent_only_payment_is_payable_but_split_singletons_are_not() {
    let (mut runner, source, _, opposing) = soil_fixture(NIGHT_SOIL, 0, 2);
    assert!(can_activate_ability_now(runner.state(), P0, source, 0));
    let mut events = Vec::new();
    announce_soil(&mut runner, source, &mut events);
    act(
        &mut runner,
        GameAction::SelectCards { cards: opposing },
        &mut events,
    );
    resolve(&mut runner, &mut events);

    let (mut runner, source, _, _) = soil_fixture(NIGHT_SOIL, 1, 1);
    assert!(!can_activate_ability_now(runner.state(), P0, source, 0));
    assert_refused_without_commit(
        &mut runner,
        GameAction::ActivateAbility {
            source_id: source,
            ability_index: 0,
        },
    );
}

#[test]
fn fixed_your_graveyard_exile_cost_never_spends_opponents_cards() {
    let oracle = "{1}, Exile two creature cards from your graveyard: Create a 1/1 green Saproling creature token.";
    let (mut runner, source, own, opposing) = soil_fixture(oracle, 2, 2);
    let mut events = Vec::new();
    announce_soil(&mut runner, source, &mut events);
    let foreign = GameAction::SelectCards { cards: opposing };
    let contract = AiDecisionContract::issue(runner.state(), P0);
    assert!(!contract.contains_action(runner.state(), &foreign));
    assert_refused_without_commit(&mut runner, foreign);
    act(
        &mut runner,
        GameAction::SelectCards { cards: own },
        &mut events,
    );
    resolve(&mut runner, &mut events);
}

#[test]
fn successive_fixed_exile_costs_keep_their_own_collective_source_rule() {
    let oracle = concat!(
        "{1}, Exile two creature cards from a single graveyard, ",
        "Exile two creature cards from a single graveyard: ",
        "Create a 1/1 green Saproling creature token."
    );
    let (mut runner, source, own, opposing) = soil_fixture(oracle, 2, 2);
    let mut events = Vec::new();
    announce_soil(&mut runner, source, &mut events);
    let first = GameAction::SelectCards { cards: own.clone() };
    assert_issued(&runner, &first);
    act(&mut runner, first, &mut events);
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::PayCost {
            kind: PayCostKind::ExileFromZone { .. },
            count: 2,
            min_count: 2,
            ..
        }
    ));
    assert!(
        runner.state().stack.is_empty(),
        "the second cost is still outstanding"
    );
    assert_refused_without_commit(
        &mut runner,
        GameAction::SelectCards {
            cards: vec![opposing[0]],
        },
    );
    let second = GameAction::SelectCards {
        cards: opposing.clone(),
    };
    assert_issued(&runner, &second);
    act(&mut runner, second, &mut events);
    resolve(&mut runner, &mut events);
    for id in own.iter().chain(&opposing) {
        assert_eq!(runner.state().objects[id].zone, Zone::Exile);
        assert_eq!(events.iter().filter(|event| matches!(event,
            GameEvent::ZoneChanged { object_id, from: Some(Zone::Graveyard), to: Zone::Exile, .. } if object_id == id)).count(), 1);
    }
    assert_eq!(runner.state().players[0].mana_pool.mana.len(), 0);
    assert_eq!(
        runner
            .state()
            .battlefield
            .iter()
            .filter(|id| runner.state().objects[id].is_token)
            .count(),
        1
    );
}

#[test]
fn explicitly_plural_graveyard_cost_allows_a_mixed_owner_payment() {
    let oracle =
        "Exile two creature cards from graveyards: Create a 1/1 green Saproling creature token.";
    let (mut runner, source, own, opposing) = soil_fixture(oracle, 1, 1);
    assert!(can_activate_ability_now(runner.state(), P0, source, 0));
    let mut events = Vec::new();
    announce_soil(&mut runner, source, &mut events);
    let payment = GameAction::SelectCards {
        cards: vec![own[0], opposing[0]],
    };
    assert_issued(&runner, &payment);
    act(&mut runner, payment, &mut events);
    resolve(&mut runner, &mut events);
    assert_eq!(runner.state().objects[&own[0]].zone, Zone::Exile);
    assert_eq!(runner.state().objects[&opposing[0]].zone, Zone::Exile);
    assert_eq!(
        runner.state().players[0].mana_pool.mana.len(),
        1,
        "this cost has no mana leg"
    );
}

#[test]
fn fixed_exile_mana_cost_shares_exact_cardinality_and_single_pile_admission() {
    let oracle = "Exile two creature cards from a single graveyard: Add {G}.";
    let (mut runner, source, own, opposing) = soil_fixture(oracle, 2, 2);
    let mut events = Vec::new();
    act(
        &mut runner,
        GameAction::ActivateAbility {
            source_id: source,
            ability_index: 0,
        },
        &mut events,
    );
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::PayCost {
            kind: PayCostKind::ExileFromManaZone {
                zone: Zone::Graveyard
            },
            count: 2,
            min_count: 2,
            ..
        }
    ));
    let contract = AiDecisionContract::issue(runner.state(), P0);
    for cards in [vec![own[0]], vec![own[0], opposing[0]]] {
        let action = GameAction::SelectCards { cards };
        assert!(!contract.contains_action(runner.state(), &action));
        assert!(!contract.permits(runner.state(), P0, &action));
        assert_refused_without_commit(&mut runner, action);
    }
    let payment = GameAction::SelectCards {
        cards: opposing.clone(),
    };
    assert_issued(&runner, &payment);
    act(&mut runner, payment, &mut events);
    assert!(
        runner.state().stack.is_empty(),
        "mana abilities do not use the stack"
    );
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::Priority { .. }
    ));
    assert_eq!(runner.state().players[0].mana_pool.mana.len(), 2);
    assert_eq!(
        runner.state().players[0]
            .mana_pool
            .mana
            .iter()
            .filter(|unit| unit.color == ManaType::Green)
            .count(),
        1
    );
    for id in opposing {
        assert_eq!(runner.state().objects[&id].zone, Zone::Exile);
        assert_eq!(events.iter().filter(|event| matches!(event,
            GameEvent::ZoneChanged { object_id, from: Some(Zone::Graveyard), to: Zone::Exile, .. } if *object_id == id)).count(), 1);
    }
}

#[test]
fn collective_grouping_does_not_widen_unfiltered_payer_graveyard_scope() {
    use engine::types::ability::{
        AbilityCost, AbilityDefinition, AbilityKind, Effect, ManaContribution, ManaProduction,
        QuantityExpr, TargetFilter,
    };
    use engine::types::mana::{ManaColor, ManaCost};

    for produces_mana in [false, true] {
        for own_count in [0, 2] {
            let mut scenario = GameScenario::new();
            scenario.at_phase(Phase::PreCombatMain);
            scenario.with_mana_pool(
                P0,
                vec![ManaUnit::new(
                    ManaType::Colorless,
                    ObjectId(9001),
                    false,
                    vec![],
                )],
            );
            let effect = if produces_mana {
                Effect::Mana {
                    produced: ManaProduction::Fixed {
                        colors: vec![ManaColor::Green],
                        contribution: ManaContribution::Base,
                    },
                    restrictions: vec![],
                    grants: vec![],
                    expiry: None,
                    target: None,
                }
            } else {
                Effect::GainLife {
                    amount: QuantityExpr::Fixed { value: 2 },
                    player: TargetFilter::Controller,
                }
            };
            let source = scenario
                .add_creature(P0, "Payer-scoped exile source", 0, 0)
                .as_enchantment()
                .with_ability_definition(
                    AbilityDefinition::new(AbilityKind::Activated, effect).cost(
                        AbilityCost::Composite {
                            costs: vec![
                                AbilityCost::Mana {
                                    cost: ManaCost::generic(1),
                                },
                                AbilityCost::Exile {
                                    count: 2,
                                    zone: Some(Zone::Graveyard),
                                    filter: None,
                                    same_zone_owner: true,
                                },
                            ],
                        },
                    ),
                )
                .id();
            let own = (0..own_count)
                .map(|i| {
                    scenario
                        .add_creature_to_graveyard(P0, &format!("Own card {i}"), 2, 2)
                        .id()
                })
                .collect::<Vec<_>>();
            let opposing = (0..2)
                .map(|i| {
                    scenario
                        .add_creature_to_graveyard(P1, &format!("Opposing card {i}"), 2, 2)
                        .id()
                })
                .collect::<Vec<_>>();
            let mut runner = scenario.build();
            let activation = GameAction::ActivateAbility {
                source_id: source,
                ability_index: 0,
            };
            if own_count == 0 {
                assert_refused_without_commit(&mut runner, activation);
                continue;
            }
            let mut events = Vec::new();
            act(&mut runner, activation, &mut events);
            let WaitingFor::PayCost { choices, .. } = &runner.state().waiting_for else {
                panic!("the payer's two cards must pay the cost interactively");
            };
            assert_eq!(
                choices.iter().copied().collect::<BTreeSet<_>>(),
                own.iter().copied().collect::<BTreeSet<_>>()
            );
            assert_refused_without_commit(
                &mut runner,
                GameAction::SelectCards {
                    cards: opposing.clone(),
                },
            );
            act(
                &mut runner,
                GameAction::SelectCards { cards: own.clone() },
                &mut events,
            );
            if produces_mana {
                assert_eq!(runner.state().players[0].mana_pool.total(), 1);
                assert_eq!(
                    runner.state().players[0].mana_pool.mana[0].color,
                    ManaType::Green
                );
                assert!(runner.state().stack.is_empty());
            } else {
                resolve(&mut runner, &mut events);
                assert_eq!(runner.state().players[0].life, 22);
                assert_eq!(runner.state().players[0].mana_pool.total(), 0);
            }
            for id in own {
                assert_eq!(runner.state().objects[&id].zone, Zone::Exile);
            }
            for id in opposing {
                assert_eq!(runner.state().objects[&id].zone, Zone::Graveyard);
            }
        }
    }
}

#[test]
fn resolution_exile_payment_retains_its_collective_rule_across_the_pause() {
    use engine::types::ability::{
        AbilityCost, AbilityDefinition, AbilityKind, Effect, TargetFilter, EXILE_COST_ANY_NUMBER,
    };
    for (count, same_zone_owner) in [(2, false), (2, true), (EXILE_COST_ANY_NUMBER, true)] {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let source = scenario
            .add_creature(P0, "Resolution-time exile cost", 0, 0)
            .as_enchantment()
            .with_ability_definition(AbilityDefinition::new(
                AbilityKind::Activated,
                Effect::PayCost {
                    cost: AbilityCost::Exile {
                        count,
                        zone: Some(Zone::Graveyard),
                        filter: None,
                        same_zone_owner,
                    },
                    scale: None,
                    payer: TargetFilter::Controller,
                },
            ))
            .id();
        let own: Vec<_> = (0..2)
            .map(|i| {
                scenario
                    .add_creature_to_graveyard(P0, &format!("Own card {i}"), 2, 2)
                    .id()
            })
            .collect();
        let opposing: Vec<_> = (0..2)
            .map(|i| {
                scenario
                    .add_creature_to_graveyard(P1, &format!("Opposing card {i}"), 2, 2)
                    .id()
            })
            .collect();
        let mut runner = scenario.build();
        let mut events = Vec::new();
        act(
            &mut runner,
            GameAction::ActivateAbility {
                source_id: source,
                ability_index: 0,
            },
            &mut events,
        );
        act(&mut runner, GameAction::PassPriority, &mut events);
        act(&mut runner, GameAction::PassPriority, &mut events);
        let contract = AiDecisionContract::issue(runner.state(), P0);
        let mut pairs: Vec<_> = contract
            .candidates
            .iter()
            .filter_map(|candidate| match &candidate.action {
                GameAction::SelectCards { cards } => Some(cards.clone()),
                _ => None,
            })
            .collect();
        for cards in [&own, &opposing] {
            assert_issued(
                &runner,
                &GameAction::SelectCards {
                    cards: cards.clone(),
                },
            );
        }
        if count == EXILE_COST_ANY_NUMBER {
            assert_issued(&runner, &GameAction::SelectCards { cards: vec![] });
            if !pairs.iter().any(Vec::is_empty) {
                pairs.push(vec![]);
            }
        }
        let mixed = GameAction::SelectCards {
            cards: vec![own[0], opposing[0]],
        };
        if same_zone_owner {
            assert!(!contract.contains_action(runner.state(), &mixed));
            assert_refused_without_commit(&mut runner, mixed);
        } else {
            assert_issued(&runner, &mixed);
        }
        for pair in pairs {
            let mut branch = GameRunner::from_state(runner.state().clone());
            if same_zone_owner {
                assert!(
                    pair.iter().all(|id| own.contains(id))
                        || pair.iter().all(|id| opposing.contains(id))
                );
            }
            act(
                &mut branch,
                GameAction::SelectCards {
                    cards: pair.clone(),
                },
                &mut Vec::new(),
            );
            for id in own.iter().chain(&opposing) {
                assert_eq!(
                    branch.state().objects[id].zone,
                    if pair.contains(id) {
                        Zone::Exile
                    } else {
                        Zone::Graveyard
                    }
                );
            }
            assert!(branch.state().stack.is_empty());
            assert_eq!(
                (
                    branch.state().players[0].life,
                    branch.state().players[1].life
                ),
                (20, 20)
            );
        }
    }
}
