//! Darigaaz's paid combat trigger counts cards of the color actually chosen.

use engine::game::combat::AttackTarget;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{ChoiceType, TargetRef};
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const P2: PlayerId = PlayerId(2);

// Verbatim current Oracle: https://api.scryfall.com/cards/named?exact=Darigaaz%2C%20the%20Igniter
const DARIGAAZ_ORACLE: &str = concat!(
    "Flying\n",
    "Whenever Darigaaz deals combat damage to a player, you may pay {2}{R}. ",
    "If you do, choose a color, then that player reveals their hand and Darigaaz ",
    "deals damage to the player equal to the number of cards of that color revealed this way."
);

fn hand_card(
    scenario: &mut GameScenario,
    player: PlayerId,
    name: &str,
    shards: Vec<ManaCostShard>,
) -> ObjectId {
    scenario
        .add_creature_to_hand(player, name, 1, 1)
        .with_mana_cost(ManaCost::Cost { shards, generic: 1 })
        .id()
}

fn board(mixed_hand: bool) -> (GameScenario, ObjectId, Vec<ObjectId>) {
    let mut scenario = GameScenario::new_n_player(3, 42);
    scenario.at_phase(Phase::PreCombatMain);
    let source = scenario
        .add_creature_from_oracle(P0, "Darigaaz, the Igniter", 6, 6, DARIGAAZ_ORACLE)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![
                ManaCostShard::Black,
                ManaCostShard::Red,
                ManaCostShard::Green,
            ],
            generic: 3,
        })
        .with_subtypes(vec!["Dragon"])
        .id();

    // Neither the controller's hand nor the first opponent's hand is the
    // revealed population. P2 is the player Darigaaz actually damages.
    hand_card(
        &mut scenario,
        P0,
        "Controller Red Card",
        vec![ManaCostShard::Red],
    );
    hand_card(
        &mut scenario,
        P0,
        "Another Controller Red Card",
        vec![ManaCostShard::Red],
    );
    hand_card(
        &mut scenario,
        P1,
        "Other Opponent Green Card",
        vec![ManaCostShard::Green],
    );

    let mut hand = Vec::new();
    if mixed_hand {
        hand.push(hand_card(
            &mut scenario,
            P2,
            "Red Card",
            vec![ManaCostShard::Red],
        ));
        hand.push(hand_card(
            &mut scenario,
            P2,
            "Another Red Card",
            vec![ManaCostShard::Red],
        ));
        hand.push(hand_card(
            &mut scenario,
            P2,
            "Red Blue Card",
            vec![ManaCostShard::Red, ManaCostShard::Blue],
        ));
        hand.push(hand_card(
            &mut scenario,
            P2,
            "White Card",
            vec![ManaCostShard::White],
        ));
        hand.push(scenario.add_land_to_hand(P2, "Colorless Land").id());
    }
    (scenario, source, hand)
}

fn resolve_combat_trigger(
    runner: &mut GameRunner,
    source: ObjectId,
    color: &str,
    accept: bool,
) -> Vec<GameEvent> {
    let p0_life = runner.life(P0);
    let p1_life = runner.life(P1);
    let source_choices = runner.state().objects[&source].chosen_attributes.clone();
    runner.advance_to_combat();
    let mut events = runner
        .declare_attackers(&[(source, AttackTarget::Player(P2))])
        .expect("Darigaaz attacks the second opponent")
        .events;
    let mut offers = 0;
    let mut colors = 0;
    let mut settled = false;
    for _ in 0..96 {
        let action = match runner.state().waiting_for.clone() {
            WaitingFor::DeclareBlockers { .. } => GameAction::DeclareBlockers {
                assignments: vec![],
            },
            WaitingFor::OptionalEffectChoice {
                player, source_id, ..
            } => {
                assert_eq!(player, P0, "Darigaaz's controller decides whether to pay");
                assert_eq!(source_id, source);
                offers += 1;
                assert_eq!(offers, 1, "one payment offer per combat-damage trigger");
                assert_eq!(
                    runner.life(P2),
                    14,
                    "the trigger follows real six-point combat damage"
                );
                for _ in 0..3 {
                    runner
                        .state_mut()
                        .add_mana_to_pool(
                            P0,
                            ManaUnit::new(ManaType::Red, ObjectId(0), false, vec![]),
                        )
                        .expect("provide the mana while deciding the optional payment");
                }
                GameAction::DecideOptionalEffect { accept }
            }
            WaitingFor::NamedChoice {
                player,
                choice_type,
                options,
                ..
            } => {
                assert!(accept, "declining payment must not ask for a color");
                assert_eq!(
                    player, P0,
                    "the source controller chooses, not the damaged player"
                );
                assert!(matches!(choice_type, ChoiceType::Color { .. }));
                assert!(options.iter().any(|option| option == color));
                colors += 1;
                assert_eq!(colors, 1, "the printed chain chooses exactly one color");
                assert_eq!(
                    runner.state().players[0].mana_pool.total(),
                    0,
                    "pay exactly {{2}}{{R}} before choosing"
                );
                assert_eq!(
                    runner.life(P2),
                    14,
                    "noncombat damage must wait for the color choice"
                );
                GameAction::ChooseOption {
                    choice: color.to_string(),
                }
            }
            WaitingFor::Priority { .. } if offers == 1 && runner.state().stack.is_empty() => {
                settled = true;
                break;
            }
            WaitingFor::Priority { .. } => GameAction::PassPriority,
            other => panic!("unexpected prompt in Darigaaz's printed chain: {other:?}"),
        };
        events.extend(
            runner
                .act(action)
                .expect("the combat/choice action is legal")
                .events,
        );
    }
    assert!(settled, "Darigaaz's actual trigger must finish resolving");
    assert_eq!(offers, 1);
    assert_eq!(colors, usize::from(accept));
    assert_eq!(
        runner.state().players[0].mana_pool.total(),
        if accept { 0 } else { 3 }
    );
    assert_eq!(runner.life(P0), p0_life, "do not damage the controller");
    assert_eq!(
        runner.life(P1),
        p1_life,
        "do not substitute the first opponent"
    );
    assert_eq!(
        runner.state().objects[&source].chosen_attributes,
        source_choices,
        "preserve the source's existing persistent choices"
    );
    events
}

fn assert_reveal(events: &[GameEvent], hand: &[ObjectId]) {
    let revealed: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            GameEvent::CardsRevealed {
                player, card_ids, ..
            } => Some((*player, card_ids.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        revealed,
        vec![(P2, hand.to_vec())],
        "reveal exactly the damaged player's whole hand"
    );
}

fn noncombat_damage(events: &[GameEvent], source: ObjectId) -> u32 {
    events
        .iter()
        .filter_map(|event| match event {
            GameEvent::DamageDealt {
                source_id,
                target,
                amount,
                is_combat: false,
                ..
            } if *source_id == source => {
                assert_eq!(*target, TargetRef::Player(P2));
                Some(*amount)
            }
            _ => None,
        })
        .sum()
}

#[test]
fn darigaaz_counts_only_matching_revealed_cards_and_multicolor_once() {
    for (color, damage) in [("Red", 3), ("Blue", 1)] {
        let (scenario, source, hand) = board(true);
        let mut runner = scenario.build();
        let events = resolve_combat_trigger(&mut runner, source, color, true);
        assert_reveal(&events, &hand);
        assert_eq!(
            runner.life(P2),
            14 - damage,
            "count objects, not all revealed cards or their number of colors"
        );
        assert_eq!(noncombat_damage(&events, source), damage as u32);
        assert!(runner.state().players[2].hand.iter().eq(hand.iter()));
        for id in &hand {
            assert_eq!(
                runner.state().objects[id].zone,
                Zone::Hand,
                "revealing does not move the card"
            );
        }
    }
}

#[test]
fn darigaaz_nonempty_hand_with_no_chosen_color_matches_deals_no_extra_damage() {
    let (scenario, source, hand) = board(true);
    let mut runner = scenario.build();
    let events = resolve_combat_trigger(&mut runner, source, "Green", true);
    assert_reveal(&events, &hand);
    assert_eq!(runner.life(P2), 14);
    assert_eq!(noncombat_damage(&events, source), 0);
    assert!(runner.state().players[2].hand.iter().eq(hand.iter()));
}

#[test]
fn darigaaz_empty_latest_reveal_does_not_reuse_an_older_matching_set() {
    let (mut scenario, source, hand) = board(false);
    // This generic producer fixture establishes an older nonempty set through
    // public cast/reveal/quantity resolution, never by forging tracked state.
    let earlier_reveal = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Earlier Hand Reveal",
            false,
            "Target player reveals their hand. You gain 1 life for each card revealed this way.",
        )
        .id();
    let mut runner = scenario.build();
    runner.cast(earlier_reveal).target_player(P0).resolve();
    assert_eq!(
        runner.life(P0),
        22,
        "the earlier reveal counts both controller cards"
    );

    let events = resolve_combat_trigger(&mut runner, source, "Red", true);
    assert_eq!(
        runner.life(P2),
        14,
        "the latest empty reveal counts zero, not the older red cards"
    );
    assert_eq!(noncombat_damage(&events, source), 0);
    assert!(runner.state().players[2].hand.iter().eq(hand.iter()));
    assert!(!events.iter().any(|event| matches!(
        event,
        GameEvent::CardsRevealed {
            player: P0 | P1,
            ..
        }
    )));
}

#[test]
fn darigaaz_declined_payment_neither_chooses_nor_reveals_nor_deals_extra_damage() {
    let (scenario, source, hand) = board(true);
    let mut runner = scenario.build();
    let events = resolve_combat_trigger(&mut runner, source, "Red", false);
    assert_eq!(runner.life(P2), 14);
    assert_eq!(noncombat_damage(&events, source), 0);
    assert!(!events
        .iter()
        .any(|event| matches!(event, GameEvent::CardsRevealed { .. })));
    assert!(runner.state().players[2].hand.iter().eq(hand.iter()));
}
