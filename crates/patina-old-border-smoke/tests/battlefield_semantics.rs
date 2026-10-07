//! Consumer-visible battlefield semantics from the complete old-border corpus.

use engine::game::casting::can_activate_ability_now;
use engine::game::combat::AttackTarget;
use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::card_type::CoreType;
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const QUIRION_RANGER_ORACLE: &str = concat!(
    "Return a Forest you control to its owner's hand: Untap target creature. ",
    "Activate only once each turn."
);
const ARCADES_SABBOTH_ORACLE: &str = concat!(
    "Flying\n",
    "At the beginning of your upkeep, sacrifice Arcades Sabboth unless you pay {G}{W}{U}.\n",
    "Each untapped creature you control gets +0/+2 as long as it's not attacking.\n",
    "{W}: Arcades Sabboth gets +0/+1 until end of turn."
);
const TREETOP_VILLAGE_ORACLE: &str = concat!(
    "This land enters tapped.\n",
    "{T}: Add {G}.\n",
    "{1}{G}: This land becomes a 3/3 green Ape creature with trample until end of turn. ",
    "It's still a land. (It can deal excess combat damage to the player or planeswalker it's attacking.)"
);
const TIDAL_WARRIOR_ORACLE: &str = "{T}: Target land becomes an Island until end of turn.";

#[test]
fn quirion_ranger_returns_a_tapped_forest_as_an_untapped_hand_card() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let forest = scenario.add_basic_land(P0, ManaColor::Green);
    let ranger = scenario
        .add_creature(P0, "Quirion Ranger", 1, 1)
        .from_oracle_text(QUIRION_RANGER_ORACLE)
        .id();
    let creature = scenario.add_creature(P0, "Returned target", 2, 2).id();
    let mut runner = scenario.build();
    runner.state_mut().objects.get_mut(&forest).unwrap().tapped = true;
    runner
        .state_mut()
        .objects
        .get_mut(&creature)
        .unwrap()
        .tapped = true;

    runner
        .activate(ranger, 0)
        .target_object(creature)
        .pay_with(&[forest])
        .resolve();

    let state = runner.state();
    assert_eq!(state.objects[&forest].zone, Zone::Hand);
    assert!(state.players[0].hand.contains(&forest));
    assert!(
        !state.objects[&forest].tapped,
        "CR 110.5d: a card in hand has no tapped status"
    );
    assert!(
        state.lki_cache[&forest].tapped,
        "the departing permanent's tapped status remains available to LKI consumers"
    );
    assert!(
        !state.objects[&creature].tapped,
        "the paid ability must actually untap its target"
    );
    assert!(
        !can_activate_ability_now(state, P0, ranger, 0),
        "the once-per-turn limit survives payment and resolution"
    );
}

#[test]
fn arcades_sabboth_checks_each_affected_creatures_combat_status() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario
        .add_creature(P0, "Arcades Sabboth", 7, 7)
        .from_oracle_text(ARCADES_SABBOTH_ORACLE);
    let attacker = scenario
        .add_creature(P0, "Vigilant attacker", 2, 2)
        .vigilance()
        .id();
    let nonattacker = scenario.add_creature(P0, "Untapped nonattacker", 2, 2).id();
    let tapped = scenario.add_creature(P0, "Tapped nonattacker", 2, 2).id();
    let opponent = scenario.add_creature(P1, "Opponent nonattacker", 2, 2).id();
    let mut runner = scenario.build();
    runner.state_mut().objects.get_mut(&tapped).unwrap().tapped = true;
    runner.state_mut().layers_dirty.mark_full();
    runner.advance_to_combat();
    runner
        .declare_attackers(&[(attacker, AttackTarget::Player(P1))])
        .expect("the vigilant creature can attack");

    let state = runner.state();
    assert!(
        !state.objects[&attacker].tapped,
        "vigilance keeps the attacking creature untapped"
    );
    assert_eq!(
        state.objects[&attacker].toughness,
        Some(2),
        "the untapped attacker must not receive the nonattacking bonus"
    );
    assert_eq!(
        state.objects[&nonattacker].toughness,
        Some(4),
        "the untapped friendly nonattacker receives +0/+2"
    );
    assert_eq!(
        state.objects[&tapped].toughness,
        Some(2),
        "the tapped friendly nonattacker is outside the affected set"
    );
    assert_eq!(
        state.objects[&opponent].toughness,
        Some(2),
        "the opponent's creature is outside the affected set"
    );
}

#[test]
fn tidal_warrior_keeps_an_animated_treetop_village_a_three_three_creature() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(
        P0,
        vec![
            ManaUnit::new(ManaType::Green, ObjectId(9_902), false, vec![]),
            ManaUnit::new(ManaType::Colorless, ObjectId(9_903), false, vec![]),
        ],
    );
    let land = scenario
        .add_land_from_oracle(P0, "Treetop Village", TREETOP_VILLAGE_ORACLE)
        .id();
    let warrior = scenario
        .add_creature(P0, "Tidal Warrior", 1, 1)
        .from_oracle_text(TIDAL_WARRIOR_ORACLE)
        .id();
    let mut runner = scenario.build();
    runner.activate(land, 1).resolve();
    assert_eq!(runner.state().objects[&land].power, Some(3));
    assert_eq!(runner.state().objects[&land].toughness, Some(3));

    runner.activate(warrior, 0).target_object(land).resolve();

    let animated = &runner.state().objects[&land];
    assert!(animated
        .card_types
        .subtypes
        .iter()
        .any(|subtype| subtype == "Island"));
    assert!(animated.card_types.core_types.contains(&CoreType::Land));
    assert!(animated.card_types.core_types.contains(&CoreType::Creature));
    assert_eq!(
        animated.power,
        Some(3),
        "CR 305.7: setting a land subtype must not end its resolved animation"
    );
    assert_eq!(animated.toughness, Some(3));
}
