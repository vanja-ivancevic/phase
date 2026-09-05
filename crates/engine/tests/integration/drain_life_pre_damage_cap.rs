//! Drain Life's bounded, resolution-local life gain.
//!
//! The important distinction is between the amount of damage the preceding
//! instruction actually dealt (after prevention/replacement) and the maximum
//! life gain based on the target's value *before* that damage.  A live lookup
//! after damage would read a player's reduced life total or a planeswalker's
//! reduced loyalty, and would therefore be wrong by exactly the damage dealt.

use engine::game::effects::resolve_ability_chain;
use engine::game::scenario::{GameScenario, P0, P1};
use engine::parser::parse_oracle_text;
use engine::types::ability::{
    Effect, QuantityExpr, QuantityRef, ResolvedAbility, TargetFilter, TargetRef,
};
use engine::types::events::GameEvent;
use engine::types::identifiers::ObjectId;

const DRAIN_LIFE_ORACLE: &str = "Spend only black mana on X.\n\
Drain Life deals X damage to any target. You gain life equal to the damage dealt, but not more life than the player's life total before the damage was dealt, the planeswalker's loyalty before the damage was dealt, or the creature's toughness.";

fn drain_life_chain(target: TargetRef, amount: i32) -> ResolvedAbility {
    let gain = ResolvedAbility::new(
        Effect::GainLife {
            amount: QuantityExpr::Ref {
                qty: QuantityRef::PreviousDamageAmountCappedByTargetPreDamageValue,
            },
            player: TargetFilter::Controller,
        },
        vec![],
        ObjectId(9000),
        P0,
    );
    let mut damage = ResolvedAbility::new(
        Effect::DealDamage {
            amount: QuantityExpr::Fixed { value: amount },
            target: TargetFilter::Any,
            damage_source: None,
            excess: None,
        },
        vec![target],
        ObjectId(9000),
        P0,
    );
    damage.sub_ability = Some(Box::new(gain));
    damage
}

/// The parser must preserve the full cap rather than accepting only the first
/// sentence and silently treating Drain Life as an ordinary damage-plus-gain.
#[test]
fn drain_life_oracle_lowers_to_the_bounded_previous_damage_quantity() {
    let parsed = parse_oracle_text(
        DRAIN_LIFE_ORACLE,
        "Drain Life",
        &[],
        &["Sorcery".to_string()],
        &[],
    );
    let gain = parsed
        .abilities
        .first()
        .and_then(|ability| ability.sub_ability.as_ref())
        .unwrap_or_else(|| panic!("Drain Life must retain its gain-life clause: {parsed:?}"));

    assert!(matches!(
        &*gain.effect,
        Effect::GainLife {
            amount: QuantityExpr::Ref {
                qty: QuantityRef::PreviousDamageAmountCappedByTargetPreDamageValue,
            },
            player: TargetFilter::Controller,
        }
    ));
}

/// CR 608.2c + CR 120.3: five damage to a player at two life causes exactly
/// two life to be gained. The chain drives the production damage resolver, so
/// it proves both the pre-damage capture and the subsequent actual-damage
/// look-back; replacing the cap with a post-damage live lookup would return 0.
#[test]
fn player_target_cap_is_captured_before_damage_and_limits_gain() {
    let mut state = engine::types::game_state::GameState::new_two_player(42);
    state.players[1].life = 2;
    let mut events = Vec::new();

    resolve_ability_chain(
        &mut state,
        &drain_life_chain(TargetRef::Player(P1), 5),
        &mut events,
        0,
    )
    .expect("Drain Life chain resolves");

    assert_eq!(
        state.players[0].life, 22,
        "gain is capped at target's pre-hit life"
    );
    assert_eq!(
        state.players[1].life, -3,
        "the damage itself still deals five"
    );
    assert_eq!(
        state.last_damage_target_pre_damage_life_gain_cap, None,
        "after the following gain instruction consumes the cap, it must not leak into a later chain step"
    );
    assert!(events.iter().any(|event| matches!(
        event,
        GameEvent::DamageDealt {
            target: TargetRef::Player(P1),
            amount: 5,
            ..
        }
    )));
}

/// Creature targets use toughness, not marked damage or a post-SBA value. A
/// five-damage hit on a 1/1 still gains only one life even though the damage is
/// actually dealt in full before state-based actions are checked.
#[test]
fn creature_target_cap_uses_pre_damage_toughness() {
    let mut scenario = GameScenario::new();
    let target = scenario.add_creature(P1, "Target", 1, 1).id();
    let mut runner = scenario.build();
    let mut events = Vec::new();

    resolve_ability_chain(
        runner.state_mut(),
        &drain_life_chain(TargetRef::Object(target), 5),
        &mut events,
        0,
    )
    .expect("Drain Life chain resolves");

    assert_eq!(
        runner.life(P0),
        21,
        "gain is capped at the target's toughness"
    );
    assert_eq!(runner.state().objects[&target].damage_marked, 5);
}
