//! Vohar drains only when its loot ability discards an instant or sorcery.

use engine::game::scenario::{GameScenario, P0, P1};

const VOHAR: &str = "{T}: Draw a card, then discard a card. If you discarded an instant or \
sorcery card this way, each opponent loses 1 life and you gain 1 life.\n\
{2}, Sacrifice Vohar: You may cast target instant or sorcery card from your graveyard this \
turn. If that spell would be put into your graveyard, exile it instead. Activate only as a \
sorcery.";

fn activate_vohar(discard_instant: bool) -> (i32, i32) {
    let mut scenario = GameScenario::new();
    if discard_instant {
        scenario.add_spell_to_library_top(P0, "Drawn Instant", true);
    } else {
        scenario.add_card_to_library_top(P0, "Drawn Land");
    }
    let vohar = scenario
        .add_creature_from_oracle(P0, "Vohar, Vodalian Desecrator", 1, 2, VOHAR)
        .id();
    let mut runner = scenario.build();
    runner.activate(vohar, 0).resolve();
    (
        runner.state().players[P0.0 as usize].life,
        runner.state().players[P1.0 as usize].life,
    )
}

#[test]
fn vohar_drains_after_discarding_an_instant() {
    assert_eq!(
        activate_vohar(true),
        (21, 19),
        "discarding an instant must make the opponent lose 1 life and Vohar's controller gain 1"
    );
}

#[test]
fn vohar_does_not_drain_after_discarding_a_land() {
    assert_eq!(
        activate_vohar(false),
        (20, 20),
        "discarding a land must not trigger Vohar's drain rider"
    );
}

#[test]
fn vohar_gains_once_after_draining_multiple_opponents() {
    let mut scenario = GameScenario::new_n_player(4, 42);
    scenario.add_spell_to_library_top(P0, "Drawn Instant", true);
    let vohar = scenario
        .add_creature_from_oracle(P0, "Vohar, Vodalian Desecrator", 1, 2, VOHAR)
        .id();
    let mut runner = scenario.build();
    runner.activate(vohar, 0).resolve();
    let life: Vec<_> = runner
        .state()
        .players
        .iter()
        .map(|player| player.life)
        .collect();
    assert_eq!(life, [21, 19, 19, 19]);
}

#[test]
fn a_false_scoped_guard_does_not_skip_the_next_independent_instruction() {
    let mut scenario = GameScenario::new();
    scenario.add_card_to_library_top(P0, "Drawn Land");
    let oracle = format!("{} You gain 2 life.", VOHAR.split_once('\n').unwrap().0);
    let source = scenario
        .add_creature_from_oracle(P0, "Conditional Loot", 1, 2, &oracle)
        .id();
    let mut runner = scenario.build();
    runner.activate(source, 0).resolve();
    assert_eq!(runner.state().players[0].life, 22);
    assert_eq!(runner.state().players[1].life, 20);
}
