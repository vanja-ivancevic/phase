//! Issue #5145: Violent Eruption deals 4 damage divided as you choose among
//! any number of targets. The production client commits targets slot-by-slot via
//! `GameAction::ChooseTarget`; a nonempty target set must surface
//! `WaitingFor::DistributeAmong` before payment, same as bulk `SelectTargets`.

use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::ability::TargetRef;
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::mana::{ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;

const VIOLENT_ERUPTION_ORACLE: &str =
    "Violent Eruption deals 4 damage divided as you choose among any number of targets.\nMadness {1}{R}{R} (If you discard this card, discard it into exile. When you do, cast it for its madness cost or put it into your graveyard.)";

fn add_red_mana(
    runner: &mut engine::game::scenario::GameRunner,
    player: engine::types::PlayerId,
    count: usize,
) {
    let dummy = engine::types::identifiers::ObjectId(0);
    let pool = &mut runner
        .state_mut()
        .players
        .iter_mut()
        .find(|p| p.id == player)
        .unwrap()
        .mana_pool;
    for _ in 0..count {
        pool.add(ManaUnit::new(ManaType::Red, dummy, false, vec![]));
    }
}

fn p1_life(runner: &engine::game::scenario::GameRunner) -> i32 {
    runner
        .state()
        .players
        .iter()
        .find(|p| p.id == P1)
        .map(|p| p.life)
        .expect("P1 must exist")
}

/// Cast Violent Eruption, announce `[Player(P1), Object(bear)]` slot-by-slot and
/// return the runner parked at the division prompt.
fn violent_eruption_at_division() -> (
    engine::game::scenario::GameRunner,
    engine::types::identifiers::ObjectId,
) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);

    let bear = scenario.add_creature(P1, "Bear", 2, 2).id();

    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Violent Eruption", true, VIOLENT_ERUPTION_ORACLE)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Red, ManaCostShard::Red, ManaCostShard::Red],
            generic: 1,
        })
        .id();

    let mut runner = scenario.build();
    add_red_mana(&mut runner, P0, 6);

    let card_id = runner.state().objects[&spell].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("cast Violent Eruption should be accepted");
    for target in [TargetRef::Player(P1), TargetRef::Object(bear)] {
        runner
            .act(GameAction::ChooseTarget {
                target: Some(target),
            })
            .expect("ChooseTarget should succeed");
    }
    runner
        .act(GameAction::ChooseTarget { target: None })
        .expect("skipping optional tail should complete target selection");
    assert!(
        matches!(
            runner.state().waiting_for,
            WaitingFor::DistributeAmong { total: 4, .. }
        ),
        "expected the 4-damage division prompt, got {:?}",
        runner.state().waiting_for
    );
    (runner, bear)
}

/// CR 601.2d: a controller-announced division must cover every chosen target
/// exactly once — omitting a target or naming one twice is not a division
/// among the chosen targets even when the amounts sum to the pool. A rejected
/// announcement leaves the prompt open for a legal one.
#[test]
fn violent_eruption_rejects_partial_or_repeated_division() {
    let (mut runner, bear) = violent_eruption_at_division();
    let prompt = runner.state().waiting_for.clone();

    for distribution in [
        vec![(TargetRef::Player(P1), 4)],
        vec![(TargetRef::Object(bear), 2), (TargetRef::Object(bear), 2)],
        vec![(TargetRef::Player(P1), 4), (TargetRef::Object(bear), 0)],
    ] {
        assert!(
            runner
                .act(GameAction::DistributeAmong { distribution })
                .is_err(),
            "an incomplete, repeated or zero-share division must be rejected"
        );
        assert_eq!(runner.state().waiting_for, prompt);
    }

    runner
        .act(GameAction::DistributeAmong {
            distribution: vec![(TargetRef::Player(P1), 2), (TargetRef::Object(bear), 2)],
        })
        .expect("a complete 2/2 division should be accepted");
    runner.advance_until_stack_empty();
    assert_eq!(p1_life(&runner), 18, "P1 takes exactly its 2");
}

/// CR 608.2b + CR 601.2d: when one target becomes illegal before resolution the
/// surviving target receives only the share announced for it — the division is
/// not redone.
#[test]
fn violent_eruption_illegal_target_keeps_survivor_share() {
    let (mut runner, bear) = violent_eruption_at_division();
    runner
        .act(GameAction::DistributeAmong {
            distribution: vec![(TargetRef::Player(P1), 1), (TargetRef::Object(bear), 3)],
        })
        .expect("1/3 distribution should be accepted");

    engine::game::zones::move_to_zone(
        runner.state_mut(),
        bear,
        engine::types::zones::Zone::Graveyard,
        &mut Vec::new(),
    );
    runner.advance_until_stack_empty();

    assert_eq!(
        p1_life(&runner),
        19,
        "the surviving player target takes only its announced 1"
    );
}

#[test]
fn violent_eruption_choose_target_path_divides_damage_among_two_targets() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);

    let bear = scenario.add_creature(P1, "Bear", 2, 2).id();

    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Violent Eruption", true, VIOLENT_ERUPTION_ORACLE)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Red, ManaCostShard::Red, ManaCostShard::Red],
            generic: 1,
        })
        .id();

    let mut runner = scenario.build();
    add_red_mana(&mut runner, P0, 6);

    let card_id = runner.state().objects[&spell].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("cast Violent Eruption should be accepted");

    assert!(
        matches!(
            runner.state().waiting_for,
            WaitingFor::TargetSelection { .. }
        ),
        "expected target selection after cast announcement"
    );

    // Mirror the client: one ChooseTarget per slot (opponent player, then creature).
    runner
        .act(GameAction::ChooseTarget {
            target: Some(TargetRef::Player(P1)),
        })
        .expect("first ChooseTarget (player) should succeed");
    runner
        .act(GameAction::ChooseTarget {
            target: Some(TargetRef::Object(bear)),
        })
        .expect("second ChooseTarget (creature) should succeed");
    // Skip remaining optional slots — mirrors the client "done" action.
    runner
        .act(GameAction::ChooseTarget { target: None })
        .expect("skipping optional tail should complete target selection");

    let WaitingFor::DistributeAmong { total, targets, .. } = runner.state().waiting_for.clone()
    else {
        panic!(
            "expected DistributeAmong after slot-by-slot target selection, got {:?}",
            runner.state().waiting_for,
        );
    };
    assert_eq!(total, 4, "damage pool to divide must be 4");
    assert_eq!(
        targets.len(),
        2,
        "both chosen targets must participate in the distribution",
    );

    runner
        .act(GameAction::DistributeAmong {
            distribution: vec![(TargetRef::Player(P1), 1), (TargetRef::Object(bear), 3)],
        })
        .expect("1/3 distribution should be accepted");

    runner.advance_until_stack_empty();

    assert_eq!(
        runner.state().objects[&bear].damage_marked,
        3,
        "creature must take only its assigned share",
    );
    let p1_life = runner
        .state()
        .players
        .iter()
        .find(|p| p.id == P1)
        .map(|p| p.life)
        .expect("P1 must exist");
    assert_eq!(
        p1_life, 19,
        "opponent must lose only their assigned share (20 - 1)",
    );
}

#[test]
fn violent_eruption_zero_targets_pays_and_resolves_without_a_division_prompt() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let bear = scenario.add_creature(P1, "Bear", 2, 2).id();
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Violent Eruption", true, VIOLENT_ERUPTION_ORACLE)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Red; 3],
            generic: 1,
        })
        .id();
    let mut runner = scenario.build();
    add_red_mana(&mut runner, P0, 4);
    let cast = GameAction::CastSpell {
        object_id: spell,
        card_id: runner.state().objects[&spell].card_id,
        targets: vec![],
        payment_mode: CastPaymentMode::Auto,
    };
    assert!(engine::ai_support::legal_actions(runner.state()).contains(&cast));
    runner.act(cast).unwrap();
    let finish = GameAction::ChooseTarget { target: None };
    assert!(engine::ai_support::legal_actions(runner.state()).contains(&finish));
    runner.act(finish).unwrap();
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::Priority { .. }
    ));
    assert_eq!(runner.state().players[P0.0 as usize].mana_pool.total(), 0);
    runner.advance_until_stack_empty();
    assert_eq!(runner.state().players[P0.0 as usize].life, 20);
    assert_eq!(runner.state().players[P1.0 as usize].life, 20);
    assert_eq!(runner.state().objects[&bear].damage_marked, 0);
    assert_eq!(
        runner.state().objects[&spell].zone,
        engine::types::zones::Zone::Graveyard
    );
}
