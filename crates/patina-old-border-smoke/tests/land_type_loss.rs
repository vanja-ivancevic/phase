//! Parsed-card consumers for land-type/ability loss followed by quoted mana grants.
//! Named-land cases exercise the Alpine Moon static grammar with the existing
//! unrestricted card-name chooser. They do not claim its separate nonbasic-land
//! name restriction is supported.

use engine::game::casting::can_activate_ability_now;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::actions::GameAction;
use engine::types::card_type::CoreType;
use engine::types::game_state::{ManaChoice, ManaChoicePrompt, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaType;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const LITHOFORM_BLIGHT: &str = concat!(
    "Enchant land\n",
    "When Lithoform Blight enters, draw a card.\n",
    "Enchanted land loses all land types and abilities and has \"{T}: Add {C}\" ",
    "and \"{T}, Pay 1 life: Add one mana of any color.\""
);
const GENERIC_AURA: &str = concat!(
    "Enchant land\n",
    "When this enchantment enters, draw a card.\n",
    "Enchanted land loses all abilities and land types, and it gains \"{T}: Add {C}\" ",
    "and \"{T}, Pay 1 life: Add one mana of any color.\""
);
const NAMED_LAND_REWRITER: &str = concat!(
    "As this enchantment enters, choose a card name.\n",
    "Lands your opponents control with the chosen name lose all land types and abilities, ",
    "and they gain \"{T}: Add one mana of any color.\""
);
const GENERIC_MOON: &str = concat!(
    "As this enchantment enters, choose a card name.\n",
    "Lands your opponents control with the chosen name lose all abilities and land types ",
    "and have \"{T}: Add one mana of any color.\""
);
const UTILITY_ORACLE: &str = "{T}: Add {G}.\n{T}: You gain 2 life.";
const COLORS: [ManaType; 5] = [
    ManaType::White,
    ManaType::Blue,
    ManaType::Black,
    ManaType::Red,
    ManaType::Green,
];

// A generic printed utility land avoids relying on a card-name-specific
// implementation and supplies two land subtypes and a nonmana ability to lose.
fn utility_land(
    scenario: &mut GameScenario,
    owner: PlayerId,
    controller: PlayerId,
    name: &str,
) -> ObjectId {
    scenario
        .add_land_from_oracle(owner, name, UTILITY_ORACLE)
        .with_subtypes(vec!["Forest", "Desert"])
        .controlled_by(controller)
        .id()
}

fn assert_land_without_subtypes(runner: &GameRunner, land: ObjectId, ability_count: usize) {
    let object = &runner.state().objects[&land];
    assert_eq!(object.zone, Zone::Battlefield);
    assert!(object.card_types.core_types.contains(&CoreType::Land));
    assert!(object.card_types.subtypes.is_empty(), "{object:#?}");
    assert_eq!(object.abilities.len(), ability_count);
}

fn assert_untouched_land(runner: &GameRunner, land: ObjectId) {
    let object = &runner.state().objects[&land];
    assert_eq!(object.card_types.subtypes, ["Forest", "Desert"]);
    assert_eq!(object.abilities.len(), 2);
    assert!(!object.tapped);
}

fn activate_mana(
    runner: &mut GameRunner,
    controller: PlayerId,
    land: ObjectId,
    ability: usize,
    color: ManaType,
    life_cost: i32,
) {
    assert!(can_activate_ability_now(
        runner.state(),
        controller,
        land,
        ability
    ));
    let life = runner.life(controller);
    assert_eq!(
        runner.state().players[controller.0 as usize]
            .mana_pool
            .total(),
        0
    );
    runner
        .act(GameAction::ActivateAbility {
            source_id: land,
            ability_index: ability,
        })
        .expect("activate the replacement mana ability through the public reducer");
    if color != ManaType::Colorless {
        let WaitingFor::ChooseManaColor { player, choice, .. } = &runner.state().waiting_for else {
            panic!(
                "the replacement must offer a mana color: {:?}",
                runner.state().waiting_for
            );
        };
        assert_eq!(*player, controller);
        let ManaChoicePrompt::SingleColor { options } = choice else {
            panic!("one mana of any color must offer a single-color choice");
        };
        assert_eq!(options.len(), COLORS.len());
        assert!(COLORS.iter().all(|color| options.contains(color)));
        runner
            .act(GameAction::ChooseManaColor {
                choice: ManaChoice::SingleColor(color),
                count: 1,
            })
            .expect("choose the exact color produced by the granted mana ability");
    }
    let pool = &runner.state().players[controller.0 as usize].mana_pool;
    assert_eq!(
        pool.total(),
        1,
        "produce exactly one mana, not the printed utility effect"
    );
    assert_eq!(pool.count_color(color), 1);
    assert_eq!(runner.life(controller), life - life_cost);
    assert!(runner.state().objects[&land].tapped);
    assert!(
        runner.state().stack.is_empty(),
        "mana abilities do not use the stack"
    );
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::Priority { .. }
    ));
    assert!(!can_activate_ability_now(
        runner.state(),
        controller,
        land,
        ability
    ));
}

#[test]
fn lithoform_blight_replaces_land_types_and_printed_utility_with_both_payable_mana_grants() {
    for (name, oracle, ability, color, life_cost) in [
        (
            "Lithoform Blight",
            LITHOFORM_BLIGHT,
            0,
            ManaType::Colorless,
            0,
        ),
        ("Lithoform Blight", LITHOFORM_BLIGHT, 1, ManaType::Black, 1),
        ("Generic Land Rewriter", GENERIC_AURA, 1, ManaType::Blue, 1),
    ] {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        scenario.with_library_top(P0, &["Drawn Card", "Spare Card"]);
        let land = utility_land(&mut scenario, P0, P0, "Enchanted Utility Land");
        let other = utility_land(&mut scenario, P0, P0, "Unenchanted Utility Land");
        let aura = scenario
            .add_spell_to_hand(P0, name, false)
            .as_enchantment()
            .with_subtypes(vec!["Aura"])
            .from_oracle_text_with_keywords(&["Enchant"], oracle)
            .id();
        let mut runner = scenario.build();
        runner.cast(aura).target_object(land).resolve();
        assert_eq!(runner.state().objects[&aura].zone, Zone::Battlefield);
        assert_eq!(
            runner.state().players[0].hand.len(),
            1,
            "resolve the parsed enters trigger"
        );
        assert_land_without_subtypes(&runner, land, 2);
        assert_untouched_land(&runner, other);
        activate_mana(&mut runner, P0, land, ability, color, life_cost);
        assert_untouched_land(&runner, other);

        // The other land's printed utility ability is still executable.
        let life = runner.life(P0);
        runner.activate(other, 1).resolve();
        assert_eq!(runner.life(P0), life + 2);
        assert!(runner.state().objects[&other].tapped);
    }
}

#[test]
fn named_land_loss_replaces_only_opponent_controlled_lands_with_the_chosen_name() {
    for (name, oracle, color) in [
        ("Named Land Rewriter", NAMED_LAND_REWRITER, ManaType::Red),
        ("Generic Named Land Rewriter", GENERIC_MOON, ManaType::Blue),
    ] {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let affected = utility_land(&mut scenario, P0, P1, "Chosen Utility Land");
        let also_affected = utility_land(&mut scenario, P1, P1, "Chosen Utility Land");
        let friendly = utility_land(&mut scenario, P1, P0, "Chosen Utility Land");
        let different_name = utility_land(&mut scenario, P1, P1, "Other Utility Land");
        let moon = scenario
            .add_spell_to_hand(P0, name, false)
            .as_enchantment()
            .from_oracle_text(oracle)
            .id();
        let mut runner = scenario.build();
        runner.state_mut().all_card_names = vec![
            "Chosen Utility Land".to_string(),
            "Other Utility Land".to_string(),
        ]
        .into();
        runner
            .cast(moon)
            .choose_option("Chosen Utility Land")
            .resolve();
        assert_eq!(runner.state().objects[&moon].zone, Zone::Battlefield);
        assert!(runner.state().stack.is_empty());
        assert_land_without_subtypes(&runner, affected, 1);
        assert_land_without_subtypes(&runner, also_affected, 1);
        assert_untouched_land(&runner, friendly);
        assert_untouched_land(&runner, different_name);

        // Same name under the source controller keeps its printed utility.
        let life = runner.life(P0);
        runner.activate(friendly, 1).resolve();
        assert_eq!(runner.life(P0), life + 2);
        runner
            .act(GameAction::PassPriority)
            .expect("give the opponent priority");
        assert_eq!(runner.state().priority_player, P1);
        activate_mana(&mut runner, P1, affected, 0, color, 0);
        assert_land_without_subtypes(&runner, also_affected, 1);
        assert_untouched_land(&runner, different_name);

        // A different opponent-controlled name keeps the printed utility too.
        let life = runner.life(P1);
        runner.activate(different_name, 1).resolve();
        assert_eq!(runner.life(P1), life + 2);
        assert!(runner.state().objects[&different_name].tapped);
    }
}
