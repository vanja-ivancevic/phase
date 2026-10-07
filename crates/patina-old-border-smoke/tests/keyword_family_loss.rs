//! Public-engine regressions for quoted keyword-family removal.

use engine::ai_support::legal_actions;
use engine::game::casting::can_activate_ability_now;
use engine::game::scenario::{GameScenario, P0, P1};
use engine::parser::oracle::parse_oracle_text;
use engine::types::ability::Effect;
use engine::types::actions::GameAction;
use engine::types::identifiers::ObjectId;
use engine::types::keywords::Keyword;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;

const TOLARIA_ORACLE: &str = concat!(
    "{T}: Add {U}.\n",
    "{T}: Target creature loses banding and all \"bands with other\" abilities until end of turn. ",
    "Activate only during any upkeep step."
);
const LOSS_ABILITY: usize = 1;

fn printed_keywords() -> Vec<Keyword> {
    vec![
        Keyword::Banding,
        Keyword::BandsWithOther("Legend".to_string()),
        Keyword::BandsWithOther("Wolf".to_string()),
        Keyword::Flying,
        Keyword::Vigilance,
    ]
}

fn add_keyword_creature(scenario: &mut GameScenario, player: PlayerId, name: &str) -> ObjectId {
    let mut creature = scenario.add_creature(player, name, 2, 2);
    for keyword in printed_keywords() {
        creature.with_keyword(keyword);
    }
    creature.id()
}

/// CR 702.22 + CR 611.2a: the quoted name denotes an entire keyword family,
/// not a granted ability or one particular bands-with-other quality.
#[test]
fn tolaria_removes_banding_and_every_bands_with_other_quality_only_until_cleanup() {
    // Keeping P0 active while varying Tolaria's controller proves "any upkeep"
    // includes both the controller's and an opponent's upkeep.
    for controller in [P0, P1] {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::Upkeep);
        scenario.with_library_top(P0, &["First Draw", "Later Draw"]);
        scenario.with_library_top(P1, &["Opponent Draw"]);
        let tolaria = scenario
            .add_land_from_oracle(controller, "Tolaria", TOLARIA_ORACLE)
            .id();
        let target = add_keyword_creature(&mut scenario, P1, "Target Creature");
        let untouched = add_keyword_creature(&mut scenario, P0, "Untargeted Creature");
        let mut runner = scenario.build();
        if controller == P1 {
            runner
                .act(GameAction::PassPriority)
                .expect("give P1 priority in P0's upkeep");
        }
        assert_eq!(runner.state().phase, Phase::Upkeep);
        assert!(can_activate_ability_now(
            runner.state(),
            controller,
            tolaria,
            LOSS_ABILITY
        ));
        assert!(legal_actions(runner.state()).iter().any(|action| matches!(
            action,
            GameAction::ActivateAbility { source_id, ability_index }
                if *source_id == tolaria && *ability_index == LOSS_ABILITY
        )));
        assert_eq!(runner.state().objects[&target].keywords, printed_keywords());

        runner
            .activate(tolaria, LOSS_ABILITY)
            .target_object(target)
            .resolve();
        assert!(
            runner.state().stack.is_empty(),
            "the targeted activation must finish resolving"
        );
        assert!(
            runner.state().objects[&tolaria].tapped,
            "the real tap cost must be paid"
        );
        assert_eq!(
            runner.state().objects[&target].keywords,
            vec![Keyword::Flying, Keyword::Vigilance],
            "remove banding and both distinct family instances, but retain unrelated keywords"
        );
        assert_eq!(
            runner.state().objects[&target].base_keywords,
            printed_keywords()
        );
        assert_eq!(
            runner.state().objects[&untouched].keywords,
            printed_keywords()
        );

        runner.advance_to_combat();
        runner.declare_attackers(&[]).expect("declare no attackers");
        runner.advance_to_end_step();
        assert_eq!(runner.state().phase, Phase::End);
        assert_eq!(
            runner.state().objects[&target].keywords,
            vec![Keyword::Flying, Keyword::Vigilance],
            "until-end-of-turn removal remains active during the end step"
        );
        runner.advance_to_upkeep();
        assert_eq!(runner.state().phase, Phase::Upkeep);
        assert_eq!(
            runner.state().active_player,
            P1,
            "the engine must have crossed cleanup"
        );
        assert_eq!(
            runner.state().objects[&target].keywords,
            printed_keywords(),
            "cleanup restores both printed bands-with-other qualities and banding"
        );
    }
}

/// CR 602.5 + CR 503: parsing the loss must not discard its upkeep-only gate.
#[test]
fn tolaria_keyword_loss_activation_remains_illegal_in_main_phase() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let tolaria = scenario
        .add_land_from_oracle(P0, "Tolaria", TOLARIA_ORACLE)
        .id();
    let target = add_keyword_creature(&mut scenario, P1, "Target Creature");
    let mut runner = scenario.build();
    assert!(!can_activate_ability_now(
        runner.state(),
        P0,
        tolaria,
        LOSS_ABILITY
    ));
    assert!(!legal_actions(runner.state()).iter().any(|action| matches!(
        action,
        GameAction::ActivateAbility { source_id, ability_index }
            if *source_id == tolaria && *ability_index == LOSS_ABILITY
    )));
    assert!(runner
        .act(GameAction::ActivateAbility {
            source_id: tolaria,
            ability_index: LOSS_ABILITY,
        })
        .is_err());
    assert!(runner.state().stack.is_empty());
    assert!(!runner.state().objects[&tolaria].tapped);
    assert_eq!(runner.state().objects[&target].keywords, printed_keywords());
}

/// CR 113.3b: a quoted activated ability remains a grant, even when its own
/// effect removes a keyword. The inner loss must not execute with the grant.
#[test]
fn quoted_keyword_loss_inside_a_granted_activation_is_not_an_outer_loss() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let grantor = scenario
        .add_land_from_oracle(
            P0,
            "Ability Grantor",
            "{T}: Target creature gains \"{T}: Target creature loses banding until end of turn.\" until end of turn.",
        )
        .id();
    let recipient = add_keyword_creature(&mut scenario, P0, "Ability Recipient");
    let target = add_keyword_creature(&mut scenario, P1, "Inner Loss Target");
    let mut runner = scenario.build();

    runner
        .activate(grantor, 0)
        .target_object(recipient)
        .resolve();
    assert!(runner.state().stack.is_empty());
    assert_eq!(
        runner.state().objects[&recipient].keywords,
        printed_keywords()
    );
    assert_eq!(runner.state().objects[&target].keywords, printed_keywords());
    assert_eq!(runner.state().objects[&recipient].abilities.len(), 1);
    assert!(can_activate_ability_now(runner.state(), P0, recipient, 0));

    runner
        .activate(recipient, 0)
        .target_object(target)
        .resolve();
    assert!(runner.state().stack.is_empty());
    assert!(runner.state().objects[&recipient].tapped);
    assert_eq!(
        runner.state().objects[&target].keywords,
        vec![
            Keyword::BandsWithOther("Legend".to_string()),
            Keyword::BandsWithOther("Wolf".to_string()),
            Keyword::Flying,
            Keyword::Vigilance,
        ],
        "only the inner activation removes banding; its family conjunct was not invented"
    );
    assert_eq!(
        runner.state().objects[&recipient].keywords,
        printed_keywords()
    );
}

/// A known family name does not license swallowing an unknown conjunct, a
/// qualified bare keyword, or trailing prose outside the quoted reference.
#[test]
fn quoted_family_loss_requires_complete_keyword_conjuncts() {
    for oracle in [
        "{T}: Target creature loses banding from artifact creatures and all \"bands with other\" abilities until end of turn.",
        "{T}: Target creature loses banding and all \"bands with other\" abilities except flying until end of turn.",
        "{T}: Target creature loses banding and all \"unrecognized keyword family\" abilities until end of turn.",
    ] {
        let parsed = parse_oracle_text(
            oracle,
            "Unsupported Loss",
            &[],
            &["Land".to_string()],
            &[],
        );
        assert_eq!(parsed.abilities.len(), 1, "{parsed:#?}");
        assert!(
            matches!(parsed.abilities[0].effect.as_ref(), Effect::Unimplemented { .. }),
            "an unconsumed loss must not become a partially successful continuous effect: {parsed:#?}"
        );
    }
}

/// The first textual loss verb can belong to the inner granted activation;
/// the outer loss still owns its quoted family reference and recipient.
#[test]
fn quoted_grant_and_outer_family_loss_keep_distinct_recipients() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let grantor = scenario
        .add_land_from_oracle(
            P0,
            "Grant and Loss Source",
            "{T}: Target creature gains \"{T}: Target creature gains flying and loses banding until end of turn.\" and loses all \"bands with other\" abilities until end of turn.",
        )
        .id();
    let recipient = add_keyword_creature(&mut scenario, P0, "Ability Recipient");
    let target = add_keyword_creature(&mut scenario, P1, "Inner Loss Target");
    let mut runner = scenario.build();

    runner
        .activate(grantor, 0)
        .target_object(recipient)
        .resolve();
    assert!(runner.state().stack.is_empty());
    assert_eq!(
        runner.state().objects[&recipient].keywords,
        vec![Keyword::Banding, Keyword::Flying, Keyword::Vigilance],
        "the outer loss removes only the recipient's keyword family"
    );
    assert_eq!(runner.state().objects[&recipient].abilities.len(), 1);
    assert_eq!(runner.state().objects[&target].keywords, printed_keywords());

    runner
        .activate(recipient, 0)
        .target_object(target)
        .resolve();
    assert!(runner.state().stack.is_empty());
    assert_eq!(
        runner.state().objects[&target].keywords,
        vec![
            Keyword::BandsWithOther("Legend".to_string()),
            Keyword::BandsWithOther("Wolf".to_string()),
            Keyword::Flying,
            Keyword::Vigilance,
        ],
        "the inner activation retains the target's unrelated family instances"
    );
    assert_eq!(
        runner.state().objects[&recipient].keywords,
        vec![Keyword::Banding, Keyword::Flying, Keyword::Vigilance]
    );
}
