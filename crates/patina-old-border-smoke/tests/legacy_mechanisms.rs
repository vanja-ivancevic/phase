//! Smoke coverage for mechanics reachable from Patina's old-border pool.
//!
//! The suite must remain compact and database-free: it is the fast, repeatable
//! boundary for mechanism batches. Card-data regeneration, full Phase
//! integration, and Forge differential runs remain separate milestone gates.

use engine::ai_support::legal_actions;
use engine::game::casting::can_activate_ability_now;
use engine::game::scenario::{GameScenario, P0};
use engine::parser::oracle::parse_oracle_text;
use engine::types::ability::AbilityTag;
use engine::types::actions::GameAction;
use engine::types::identifiers::ObjectId;
use engine::types::keywords::Keyword;
use engine::types::mana::{ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const LEGACY_CYCLING_ORACLE: &str = "Cycling {2}";

fn cycling_index(state: &engine::types::game_state::GameState, card: ObjectId) -> usize {
    state.objects[&card]
        .abilities
        .iter()
        .position(|ability| ability.ability_tag == Some(AbilityTag::Cycling))
        .expect("Cycling must synthesize an activated ability")
}

/// CR 702.29a: the old-border Cycling keyword must parse and function only
/// while the source card is in its owner's hand.
#[test]
fn cycling_parses_and_is_legal_only_from_hand() {
    let parsed = parse_oracle_text(
        LEGACY_CYCLING_ORACLE,
        "Legacy Cycling Card",
        &[],
        &["Artifact".to_string()],
        &[],
    );
    assert!(
        parsed
            .extracted_keywords
            .iter()
            .any(|keyword| matches!(keyword, Keyword::Cycling(_))),
        "Cycling {{2}} must produce a typed Cycling keyword: {:?}",
        parsed.extracted_keywords
    );

    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_library_top(P0, &["Drawn Card"]);
    scenario.with_mana_pool(
        P0,
        vec![ManaUnit::new(ManaType::Colorless, ObjectId(9_901), false, vec![]); 2],
    );
    let card = scenario
        .add_land_to_hand(P0, "Legacy Cycling Card")
        .from_oracle_text(LEGACY_CYCLING_ORACLE)
        .id();

    let mut runner = scenario.build();
    let ability_index = cycling_index(runner.state(), card);
    assert!(
        can_activate_ability_now(runner.state(), P0, card, ability_index),
        "Cycling must be legal from hand"
    );

    runner
        .act(GameAction::ActivateAbility {
            source_id: card,
            ability_index,
        })
        .expect("activate Cycling from hand");
    runner.advance_until_stack_empty();
    assert_eq!(runner.state().objects[&card].zone, Zone::Graveyard);
    assert_eq!(
        runner.state().players[0].hand.len(),
        1,
        "Cycling must draw one card"
    );

    // The card has left hand, so its hand-zone Cycling ability cannot be
    // advertised or accepted from its new graveyard incarnation.
    assert!(
        !legal_actions(runner.state()).iter().any(|action| matches!(
            action,
            GameAction::ActivateAbility { source_id, ability_index: index }
                if *source_id == card && *index == ability_index
        )),
        "Cycling must not remain an offered action outside hand"
    );
}
