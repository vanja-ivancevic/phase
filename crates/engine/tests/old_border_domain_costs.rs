//! Old-border Domain cost regressions through the public parser API.
//!
//! Kept as a standalone integration target so card-gap work can exercise the
//! parser without linking Phase's complete unit-test harness.

use engine::game::ability_utils::{begin_target_selection_for_ability, build_target_slots};
use engine::parser::parse_oracle_text;
use engine::types::ability::{
    AbilityCondition, AbilityCost, AbilityDefinition, Comparator, ControllerRef, Effect,
    QuantityExpr, QuantityRef, ResolvedAbility, TargetFilter, TargetRef, TypedFilter,
};
use engine::types::game_state::GameState;
use engine::types::identifiers::ObjectId;
use engine::types::player::PlayerId;

#[test]
fn draco_upkeep_payment_uses_domain_reduction() {
    let parsed = parse_oracle_text(
        "Domain — This spell costs {2} less to cast for each basic land type among lands you control.\nFlying\nDomain — At the beginning of your upkeep, sacrifice this creature unless you pay {10}. This cost is reduced by {2} for each basic land type among lands you control.",
        "Draco",
        &[],
        &["Artifact".to_string(), "Creature".to_string()],
        &[],
    );

    let unless = parsed
        .triggers
        .first()
        .and_then(|trigger| trigger.unless_pay.as_ref())
        .expect("Draco upkeep must retain its unless-payment");
    assert!(matches!(
        &unless.cost,
        AbilityCost::ManaDynamic {
            quantity: QuantityExpr::Sum { exprs },
        } if matches!(
            exprs.as_slice(),
            [
                QuantityExpr::Fixed { value: 10 },
                QuantityExpr::Multiply { factor: -2, inner },
            ] if matches!(
                inner.as_ref(),
                QuantityExpr::Ref {
                    qty: QuantityRef::BasicLandTypeCount {
                        controller: ControllerRef::You,
                    },
                }
            )
        )
    ));
}

#[test]
fn tithe_retains_its_targeted_optional_second_search() {
    fn tree_has_conditioned_optional(definition: &AbilityDefinition) -> bool {
        (definition.optional && definition.condition.is_some())
            || definition
                .sub_ability
                .as_deref()
                .is_some_and(tree_has_conditioned_optional)
            || definition
                .else_ability
                .as_deref()
                .is_some_and(tree_has_conditioned_optional)
            || definition
                .mode_abilities
                .iter()
                .any(tree_has_conditioned_optional)
    }

    let parsed = parse_oracle_text(
        "Search your library for a Plains card. If target opponent controls more lands than you, you may search your library for an additional Plains card. Reveal those cards, put them into your hand, then shuffle.",
        "Tithe",
        &[],
        &["Instant".to_string()],
        &[],
    );

    assert!(
        parsed.abilities.iter().any(tree_has_conditioned_optional),
        "Tithe must retain its target-dependent optional search: {:?}",
        parsed.abilities
    );
}

#[test]
fn target_opponent_condition_offers_only_an_opponent_slot() {
    let state = GameState::new_two_player(7);
    let mut ability = ResolvedAbility::new(
        Effect::Draw {
            count: QuantityExpr::Fixed { value: 1 },
            target: TargetFilter::Controller,
        },
        vec![],
        ObjectId(1),
        PlayerId(0),
    );
    ability.condition = Some(AbilityCondition::QuantityCheck {
        lhs: QuantityExpr::Ref {
            qty: QuantityRef::ObjectCount {
                filter: TargetFilter::Typed(
                    TypedFilter::land().controller(ControllerRef::TargetOpponent),
                ),
            },
        },
        comparator: Comparator::GT,
        rhs: QuantityExpr::Ref {
            qty: QuantityRef::ObjectCount {
                filter: TargetFilter::Typed(TypedFilter::land().controller(ControllerRef::You)),
            },
        },
    });

    let slots = build_target_slots(&state, &ability).expect("conditional target slot builds");
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].legal_targets, vec![TargetRef::Player(PlayerId(1))]);
    let progress =
        begin_target_selection_for_ability(&state, &ability, &slots, &ability.target_constraints)
            .expect("conditional target slot begins selection");
    assert_eq!(
        progress.current_legal_targets,
        vec![TargetRef::Player(PlayerId(1))]
    );
}
