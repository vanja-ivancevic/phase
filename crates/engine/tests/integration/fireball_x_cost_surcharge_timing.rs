//! Fireball — even division and target-count cost on the {X} casting route.
//!
//! Real Oracle: "This spell costs {1} more to cast for each target beyond the
//! first. / Fireball deals X damage divided evenly, rounded down, among any
//! number of targets."
//!
//! The target-count surcharge is paid when casting; division is determined
//! at resolution over still-legal targets (WotC ruling, 2017-11-17).

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::game::zones;
use engine::types::ability::{CardPlayMode, CastingPermission, ManaSpendPermission, TargetRef};
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::identifiers::{CardId, ObjectId};
use engine::types::mana::{ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::statics::CastFrequency;
use engine::types::zones::{EtbTapState, Zone};
use engine::types::{Duration, PlayerId};

const FIREBALL_ORACLE: &str = "This spell costs {1} more to cast for each target beyond the first.\nFireball deals X damage divided evenly, rounded down, among any number of targets.";

/// {X}{R} — one red plus the variable generic.
fn fireball_cost() -> ManaCost {
    ManaCost::Cost {
        shards: vec![ManaCostShard::X, ManaCostShard::Red],
        generic: 0,
    }
}

fn red_pool(amount: usize) -> Vec<ManaUnit> {
    (0..amount)
        .map(|_| ManaUnit::new(ManaType::Red, ObjectId(0), false, vec![]))
        .collect()
}

fn blue_pool(amount: usize) -> Vec<ManaUnit> {
    (0..amount)
        .map(|_| ManaUnit::new(ManaType::Blue, ObjectId(0), false, vec![]))
        .collect()
}

fn pool_total(runner: &GameRunner, player: PlayerId) -> usize {
    runner
        .state()
        .players
        .iter()
        .find(|p| p.id == player)
        .map(|p| p.mana_pool.total())
        .unwrap_or(0)
}

fn life(runner: &GameRunner, player: PlayerId) -> i32 {
    runner
        .state()
        .players
        .iter()
        .find(|p| p.id == player)
        .map(|p| p.life)
        .expect("player must exist")
}

fn damage(runner: &GameRunner, creature: ObjectId) -> u32 {
    runner.state().objects[&creature].damage_marked
}

/// Three opposing 5/5 creatures and a red mana pool of `pool` mana.
fn fireball_scenario(pool: usize) -> (GameRunner, ObjectId, CardId, Vec<ObjectId>) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);

    let creatures: Vec<ObjectId> = (0..3)
        .map(|i| scenario.add_creature(P1, &format!("Giant {i}"), 5, 5).id())
        .collect();

    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Fireball", false, FIREBALL_ORACLE)
        .with_mana_cost(fireball_cost())
        .id();
    let card_id = CardId(spell.0);

    scenario.with_mana_pool(P0, red_pool(pool));

    (scenario.build(), spell, card_id, creatures)
}

fn cast_fireball(runner: &mut GameRunner, spell: ObjectId, card_id: CardId) {
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("Fireball cast announcement should be accepted");
}

/// CR 601.2b: X is announced before targets.
fn drive_choose_x(runner: &mut GameRunner, x: u32) {
    match runner.state().waiting_for.clone() {
        WaitingFor::ChooseXValue { .. } => {
            runner
                .act(GameAction::ChooseX { value: x })
                .expect("ChooseX should succeed");
        }
        other => panic!("expected ChooseXValue immediately after CastSpell, got {other:?}"),
    }
}

/// CR 601.2c: choose the given targets slot-by-slot (mirrors the production
/// client) without finishing the declaration.
fn choose_targets(runner: &mut GameRunner, targets: &[ObjectId]) {
    for &t in targets {
        assert!(
            matches!(
                runner.state().waiting_for,
                WaitingFor::TargetSelection { .. }
            ),
            "expected TargetSelection while choosing targets, got {:?}",
            runner.state().waiting_for
        );
        runner
            .act(GameAction::ChooseTarget {
                target: Some(TargetRef::Object(t)),
            })
            .expect("ChooseTarget should succeed");
    }
}

/// Finish any remaining optional "any number of targets" slots.
fn finish_targets(runner: &mut GameRunner) {
    if matches!(
        runner.state().waiting_for,
        WaitingFor::TargetSelection { .. }
    ) {
        runner
            .act(GameAction::ChooseTarget { target: None })
            .expect("finishing optional target slots should succeed");
    }
}

/// Cast Fireball for `x` among `targets` and assert the cast reaches the stack
/// without a division prompt (CR 601.2d: an even split is not announced).
fn cast_to_stack(
    runner: &mut GameRunner,
    spell: ObjectId,
    card_id: CardId,
    x: u32,
    targets: &[ObjectId],
) {
    cast_fireball(runner, spell, card_id);
    drive_choose_x(runner, x);
    choose_targets(runner, targets);
    finish_targets(runner);
    assert!(
        !matches!(
            runner.state().waiting_for,
            WaitingFor::DistributeAmong { .. }
        ),
        "an even split must not prompt for a division"
    );
    assert!(
        runner.state().stack.iter().any(|entry| entry.id == spell),
        "Fireball must be on the stack after target declaration and payment, waiting for {:?}",
        runner.state().waiting_for
    );
}

/// CR 601.2f: the final paid cost includes the per-target surcharge. Same X,
/// same starting pool, different target counts ⇒ the 3-target cast pays {1}×2
/// more than the 1-target cast, and each target is dealt its even share.
#[test]
fn fireball_surcharge_scales_with_target_count() {
    const POOL: usize = 20;
    const X: u32 = 3;

    // 1 target: {X}{R} = {3}{R} = 4 mana, no surcharge.
    let (mut runner, spell, card_id, creatures) = fireball_scenario(POOL);
    cast_to_stack(&mut runner, spell, card_id, X, &creatures[..1]);
    let residual_one = pool_total(&runner, P0);
    assert_eq!(residual_one, POOL - 4, "1 target pays {{3}}{{R}} = 4 mana");
    runner.resolve_top();
    assert_eq!(
        damage(&runner, creatures[0]),
        3,
        "a sole target takes all X"
    );

    // 3 targets: {3}{R} + {1}×2 surcharge = 6 mana.
    let (mut runner, spell, card_id, creatures) = fireball_scenario(POOL);
    cast_to_stack(&mut runner, spell, card_id, X, &creatures);
    let residual_three = pool_total(&runner, P0);
    assert_eq!(
        residual_three,
        POOL - 6,
        "3 targets must pay {{3}}{{R}} + {{1}}x2 surcharge = 6 mana (CR 601.2f)"
    );
    assert_eq!(residual_one - residual_three, 2);
    runner.resolve_top();
    for &creature in &creatures {
        assert_eq!(damage(&runner, creature), 1, "3 divided evenly among 3");
    }
}

/// CR 601.2d: "divided evenly, rounded down" — X that does not divide by the
/// target count gives every target the same rounded-down share and the
/// remainder is not dealt to anything.
#[test]
fn fireball_nondivisible_x_deals_equal_shares_and_loses_remainder() {
    const POOL: usize = 20;
    let (mut runner, spell, card_id, creatures) = fireball_scenario(POOL);
    cast_to_stack(&mut runner, spell, card_id, 5, &creatures);
    assert_eq!(pool_total(&runner, P0), POOL - 8, "{{5}}{{R}} + {{1}}x2");

    runner.resolve_top();
    for &creature in &creatures {
        assert_eq!(damage(&runner, creature), 1, "floor(5 / 3) = 1 each");
    }
    assert_eq!(life(&runner, P0), 20, "the remainder is not dealt");
    assert_eq!(life(&runner, P1), 20, "the remainder is not dealt");
}

/// CR 601.2b + CR 601.2d: Fireball cast for X = 0 completes without a division
/// prompt, pays only {R}, and deals no damage.
#[test]
fn fireball_x_zero_completes_without_division() {
    const POOL: usize = 20;
    let (mut runner, spell, card_id, creatures) = fireball_scenario(POOL);
    cast_to_stack(&mut runner, spell, card_id, 0, &[]);
    assert_eq!(pool_total(&runner, P0), POOL - 1, "X = 0 pays only {{R}}");

    runner.resolve_top();
    for &creature in &creatures {
        assert_eq!(damage(&runner, creature), 0);
    }
    assert_eq!(life(&runner, P1), 20);
}

/// CR 601.2c + CR 601.2d: "any number of targets" includes zero. With no
/// targets there is nothing to divide, so the cast completes without a
/// division prompt and the spell deals no damage.
#[test]
fn fireball_with_no_targets_completes_without_division() {
    const POOL: usize = 20;
    let (mut runner, spell, card_id, creatures) = fireball_scenario(POOL);
    cast_to_stack(&mut runner, spell, card_id, 3, &[]);
    assert_eq!(
        pool_total(&runner, P0),
        POOL - 4,
        "no targets beyond the first: {{3}}{{R}}"
    );

    runner.resolve_top();
    for &creature in &creatures {
        assert_eq!(damage(&runner, creature), 0);
    }
    assert_eq!(life(&runner, P0), 20);
    assert_eq!(life(&runner, P1), 20);
}

/// An illegal target is excluded from the resolution-time denominator.
#[test]
fn fireball_divides_over_remaining_legal_targets_at_resolution() {
    let (mut runner, spell, card_id, creatures) = fireball_scenario(20);
    cast_to_stack(&mut runner, spell, card_id, 4, &creatures[..2]);

    zones::move_to_zone(
        runner.state_mut(),
        creatures[1],
        Zone::Graveyard,
        &mut Vec::new(),
    );
    runner.resolve_top();

    assert_eq!(
        damage(&runner, creatures[0]),
        4,
        "the sole remaining legal target receives all 4"
    );
    assert_eq!(damage(&runner, creatures[2]), 0, "never a target");
}

#[test]
fn fireball_can_have_more_targets_than_x_and_deal_zero_to_each() {
    let (mut runner, spell, card_id, creatures) = fireball_scenario(4);
    cast_to_stack(&mut runner, spell, card_id, 1, &creatures);
    assert_eq!(
        pool_total(&runner, P0),
        0,
        "{{1}}{{R}} plus two extra targets"
    );
    runner.resolve_top();
    for creature in creatures {
        assert_eq!(damage(&runner, creature), 0);
    }
    assert_eq!(life(&runner, P1), 20);
}

#[test]
fn fireball_target_offers_exclude_unpayable_surcharge_but_allow_completion() {
    let (mut runner, spell, card_id, creatures) = fireball_scenario(4);
    cast_fireball(&mut runner, spell, card_id);
    drive_choose_x(&mut runner, 3);
    choose_targets(&mut runner, &creatures[..1]);
    let actions = engine::ai_support::legal_actions(runner.state());
    assert!(!actions
        .iter()
        .any(|candidate| matches!(candidate, GameAction::ChooseTarget { target: Some(_) })));
    let finish = actions
        .into_iter()
        .find(|candidate| matches!(candidate, GameAction::ChooseTarget { target: None }))
        .expect("the affordable one-target declaration remains completable");
    runner.act(finish).unwrap();
    assert_eq!(pool_total(&runner, P0), 0);
    runner.resolve_top();
    assert_eq!(damage(&runner, creatures[0]), 3);
    assert_eq!(damage(&runner, creatures[1]), 0);
}

/// CR 601.2f + CR 601.2h: when the surcharge for the declared targets makes the
/// total cost unpayable, the declaration is refused and the cast is left
/// exactly where it was — still declaring targets, nothing paid.
#[test]
fn fireball_unpayable_surcharge_refuses_declaration_cleanly() {
    // Fund EXACTLY the 1-target cost ({3}{R} = 4). Choosing 3 targets raises the
    // real cost to 6, which this pool cannot afford.
    const X: u32 = 3;
    let (mut runner, spell, card_id, creatures) = fireball_scenario(4);
    cast_fireball(&mut runner, spell, card_id);
    drive_choose_x(&mut runner, X);
    choose_targets(&mut runner, &creatures);

    let before = serde_json::to_value(runner.state()).unwrap();
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::TargetSelection { .. }
    ));

    let result = runner.act(GameAction::ChooseTarget { target: None });

    assert!(
        result.is_err(),
        "a 6-mana total cost must be unpayable from a 4-mana pool (CR 601.2h)"
    );
    assert_eq!(serde_json::to_value(runner.state()).unwrap(), before);
    assert_eq!(pool_total(&runner, P0), 4, "nothing may be paid");
}

/// CR 601.2a + CR 609.4b: the exact exile permission elected at announcement
/// survives the ChooseX -> target selection pauses. The first sibling is
/// deliberately ineligible during announcement and made eligible before
/// target declaration completes; losing the elected index would make a
/// first-match lookup select that sibling and drop the elected permission's
/// AnyColor concession, leaving blue mana unable to pay {R}.
#[test]
fn fireball_target_declaration_preserves_elected_exile_permission() {
    const X: u32 = 2;
    let (mut runner, spell, card_id, creatures) = fireball_scenario(3);
    zones::move_to_zone(runner.state_mut(), spell, Zone::Exile, &mut Vec::new());
    runner.state_mut().players[0].mana_pool.mana = blue_pool(3);
    runner
        .state_mut()
        .objects
        .get_mut(&spell)
        .unwrap()
        .casting_permissions = vec![
        CastingPermission::PlayFromExile {
            provenance: engine::types::ability::PlayFromExileProvenance::Impulse,
            duration: Duration::Permanent,
            granted_to: P1,
            mode: CardPlayMode::Play,
            frequency: CastFrequency::Unlimited,
            source_id: None,
            invalidation: None,
            exiled_by_ability_controller: None,
            mana_spend_permission: None,
            card_filter: None,
            single_use_group: None,
            single_use: false,
            cast_cost_modifier: None,
            alt_ability_cost: None,
            land_enter_tapped: EtbTapState::Unspecified,
        },
        CastingPermission::PlayFromExile {
            provenance: engine::types::ability::PlayFromExileProvenance::Impulse,
            duration: Duration::Permanent,
            granted_to: P0,
            mode: CardPlayMode::Play,
            frequency: CastFrequency::Unlimited,
            source_id: None,
            invalidation: None,
            exiled_by_ability_controller: None,
            mana_spend_permission: Some(ManaSpendPermission::AnyColor),
            card_filter: None,
            single_use_group: None,
            single_use: false,
            cast_cost_modifier: None,
            alt_ability_cost: None,
            land_enter_tapped: EtbTapState::Unspecified,
        },
    ];

    cast_fireball(&mut runner, spell, card_id);
    drive_choose_x(&mut runner, X);
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::TargetSelection { .. }
    ));

    let CastingPermission::PlayFromExile { granted_to, .. } = &mut runner
        .state_mut()
        .objects
        .get_mut(&spell)
        .unwrap()
        .casting_permissions[0]
    else {
        panic!("hostile sibling must be a PlayFromExile permission");
    };
    *granted_to = P0;

    choose_targets(&mut runner, &creatures[..1]);
    finish_targets(&mut runner);
    assert!(
        runner.state().stack.iter().any(|entry| entry.id == spell),
        "the elected AnyColor permission must pay {{2}}{{R}} with blue mana, waiting for {:?}",
        runner.state().waiting_for
    );
    assert_eq!(pool_total(&runner, P0), 0, "blue mana must pay {{2}}{{R}}");
}
