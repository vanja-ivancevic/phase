use std::sync::Arc;

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{
    AbilityDefinition, AbilityKind, CoinFlipResult, Effect, QuantityExpr, ReplacementDefinition,
    ReplacementPlayerScope, TargetFilter,
};
use engine::types::actions::GameAction;
use engine::types::counter::CounterType;
use engine::types::events::GameEvent;
use engine::types::game_state::{GameState, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::replacements::ReplacementEvent;
use engine::types::zones::Zone;

const ASSASSIN: &str = "Whenever this creature or another Goblin enters, each player flips a coin. Each player whose coin comes up tails sacrifices a creature of their choice.";
const THUMB: &str = "If you would flip a coin, instead flip two coins and ignore one.";
const CHANCE_ENCOUNTER: &str = "Whenever you win a coin flip, put a luck counter on Chance Encounter.\nAt the beginning of your upkeep, if Chance Encounter has ten or more luck counters on it, you win the game.";
const BOTTLE: &str = "{1}, Sacrifice Bottle of Suleiman: Flip a coin. If you win the flip, create a 5/5 colorless Djinn artifact creature token with flying. If you lose the flip, Bottle of Suleiman deals 5 damage to you.";
const MINOTAUR: &str = "Cumulative upkeep—Flip a coin (At the beginning of your upkeep, put an age counter on this permanent, then sacrifice it unless you pay its upkeep cost for each age counter on it.)\nWhenever you win a coin flip, Karplusan Minotaur deals 1 damage to any target.\nWhenever you lose a coin flip, Karplusan Minotaur deals 1 damage to any target of an opponent's choice.";

fn act(runner: &mut GameRunner, action: GameAction, events: &mut Vec<GameEvent>) {
    events.extend(
        runner
            .act(action)
            .expect("real game action must succeed")
            .events,
    );
}

fn flips(events: &[GameEvent]) -> Vec<(PlayerId, CoinFlipResult)> {
    events
        .iter()
        .filter_map(|event| match event {
            GameEvent::CoinFlipped { player_id, result } => Some((*player_id, *result)),
            _ => None,
        })
        .collect()
}

fn sacrifices(events: &[GameEvent]) -> Vec<(PlayerId, ObjectId)> {
    events
        .iter()
        .filter_map(|event| match event {
            GameEvent::PermanentSacrificed {
                player_id,
                object_id,
            } => Some((*player_id, *object_id)),
            _ => None,
        })
        .collect()
}

fn reseed(runner: &mut GameRunner, seed: u64) {
    let state = runner.state_mut();
    state.rng_seed = seed;
    state.rng_word_pos = 0;
    state.rng = GameState::new_two_player(seed).rng;
}

fn set_replacements(
    runner: &mut GameRunner,
    source: ObjectId,
    definitions: Vec<ReplacementDefinition>,
) {
    let object = runner
        .state_mut()
        .objects
        .get_mut(&source)
        .expect("fixture replacement source");
    object.base_replacement_definitions = Arc::new(definitions);
    object.replacement_definitions = Arc::clone(&object.base_replacement_definitions).into();
    runner.state_mut().layers_dirty.mark_full();
}

struct Round {
    runner: GameRunner,
    assassin: ObjectId,
    goblin: ObjectId,
    spare_goblin: Option<ObjectId>,
    chance: ObjectId,
    coin_observer: Option<ObjectId>,
    creatures: Vec<(PlayerId, ObjectId)>,
}

fn round(seed: u64, caster: PlayerId, thumbs: bool) -> Round {
    let mut scenario = GameScenario::new_n_player(2, seed);
    scenario.at_phase(Phase::PreCombatMain);
    let assassin = scenario
        .add_creature(P0, "Goblin Assassin", 2, 2)
        .with_subtypes(vec!["Goblin", "Assassin"])
        .from_oracle_text(ASSASSIN)
        .id();
    let mut creatures = vec![(P0, assassin)];
    for player in [P0, P1] {
        for _ in 0..3 {
            creatures.push((player, scenario.add_vanilla(player, 2, 2)));
        }
        if thumbs {
            scenario.add_artifact_from_oracle(player, "Krark's Thumb", THUMB);
        }
    }
    let chance = scenario
        .add_enchantment_from_oracle(P0, "Chance Encounter", CHANCE_ENCOUNTER)
        .id();
    let coin_observer = scenario
        .add_creature(P0, "Karplusan Minotaur", 3, 3)
        .with_subtypes(vec!["Minotaur", "Warrior"])
        .from_oracle_text(MINOTAUR)
        .id();
    creatures.push((P0, coin_observer));
    let goblin = scenario
        .add_creature_to_hand(caster, "Raging Goblin", 1, 1)
        .with_subtypes(vec!["Goblin", "Berserker"])
        .from_oracle_text("Haste")
        .id();
    let spare_goblin = scenario
        .add_creature_to_hand(caster, "Raging Goblin", 1, 1)
        .with_subtypes(vec!["Goblin", "Berserker"])
        .from_oracle_text("Haste")
        .id();
    creatures.push((caster, goblin));
    let mut runner = scenario.build();
    runner.state_mut().active_player = caster;
    runner.state_mut().priority_player = caster;
    runner.state_mut().waiting_for = WaitingFor::Priority { player: caster };
    Round {
        runner,
        assassin,
        goblin,
        spare_goblin: Some(spare_goblin),
        chance,
        coin_observer: Some(coin_observer),
        creatures,
    }
}

// Commit a real creature spell, resolve its entry, and stop before the ETB
// trigger resolves. Opponent entries must produce the same Assassin trigger.
fn enter_goblin(round: &mut Round, events: &mut Vec<GameEvent>) {
    round.runner.cast(round.goblin).commit();
    for _ in 0..16 {
        if round.runner.state().objects[&round.goblin].zone == Zone::Battlefield {
            assert!(
                round
                    .runner
                    .state()
                    .stack
                    .iter()
                    .any(|entry| entry.source_id == round.assassin),
                "the printed Goblin ETB trigger must reach the stack"
            );
            return;
        }
        act(&mut round.runner, GameAction::PassPriority, events);
    }
    panic!("Goblin spell failed to enter through GameRunner");
}

fn to_choice(runner: &mut GameRunner, events: &mut Vec<GameEvent>) {
    for _ in 0..32 {
        match runner.state().waiting_for.clone() {
            WaitingFor::Priority { .. } if runner.state().stack.is_empty() => return,
            WaitingFor::Priority { .. } => act(runner, GameAction::PassPriority, events),
            WaitingFor::OrderTriggers { triggers, .. } => act(
                runner,
                GameAction::OrderTriggers {
                    order: (0..triggers.len()).collect(),
                },
                events,
            ),
            _ => return,
        }
    }
    panic!("priority/trigger ordering did not reach a choice or completion");
}

fn choose_sacrifices(round: &mut Round, events: &mut Vec<GameEvent>) -> Vec<PlayerId> {
    let completed_flips = flips(events);
    let mut prompted = Vec::new();
    for _ in 0..16 {
        to_choice(&mut round.runner, events);
        match round.runner.state().waiting_for.clone() {
            WaitingFor::EffectZoneChoice {
                player,
                cards,
                count,
                ..
            } => {
                assert_eq!(count, 1);
                assert_eq!(
                    flips(events),
                    completed_flips,
                    "every flip finishes before any sacrifice choice"
                );
                assert!(
                    completed_flips.contains(&(player, CoinFlipResult::Tails)),
                    "only this player's own tails result may qualify them"
                );
                assert!(
                    cards
                        .iter()
                        .all(|id| round.runner.state().objects[id].controller == player),
                    "a player must choose only from their OWN creatures"
                );
                let chosen = cards
                    .iter()
                    .rev()
                    .copied()
                    .find(|id| Some(*id) != round.coin_observer)
                    .expect(
                        "preserve the live win/lose listener while choosing an eligible creature",
                    );
                // The public reducer must reject a cross-player sacrifice too,
                // not merely omit the other player's permanents from the UI.
                let foreign = round
                    .creatures
                    .iter()
                    .find_map(|(owner, id)| {
                        (*owner != player
                            && round.runner.state().objects[id].zone == Zone::Battlefield)
                            .then_some(*id)
                    })
                    .expect("the other player has a negative-control creature");
                assert!(round
                    .runner
                    .act(GameAction::SelectCards {
                        cards: vec![foreign]
                    })
                    .is_err());
                prompted.push(player);
                act(
                    &mut round.runner,
                    GameAction::SelectCards {
                        cards: vec![chosen],
                    },
                    events,
                );
            }
            WaitingFor::Priority { .. } if round.runner.state().stack.is_empty() => {
                return prompted
            }
            other => panic!("unexpected round choice: {other:?}"),
        }
    }
    panic!("coin round failed to finish its sacrifice choices");
}

fn assert_round(round: &Round, events: &[GameEvent], expected_flippers: &[PlayerId]) {
    let results = flips(events);
    assert_eq!(
        results
            .iter()
            .map(|(player, _)| *player)
            .collect::<Vec<_>>(),
        expected_flippers
    );
    assert!(results
        .iter()
        .all(|(_, result)| matches!(result, CoinFlipResult::Heads | CoinFlipResult::Tails)));
    let tails: Vec<PlayerId> = results
        .iter()
        .filter_map(|(player, result)| (*result == CoinFlipResult::Tails).then_some(*player))
        .collect();
    let sacrificed = sacrifices(events);
    assert_eq!(
        sacrificed
            .iter()
            .map(|(player, _)| *player)
            .collect::<Vec<_>>(),
        tails
    );
    for (player, id) in &sacrificed {
        assert_eq!(round.runner.state().objects[id].owner, *player);
        assert_eq!(round.runner.state().objects[id].zone, Zone::Graveyard);
    }
    for (_, id) in &round.creatures {
        let expected = if sacrificed.iter().any(|(_, chosen)| chosen == id) {
            Zone::Graveyard
        } else {
            Zone::Battlefield
        };
        assert_eq!(round.runner.state().objects[id].zone, expected);
    }
    if let Some(first_sacrifice) = events
        .iter()
        .position(|event| matches!(event, GameEvent::PermanentSacrificed { .. }))
    {
        assert!(events
            .iter()
            .enumerate()
            .filter(|(_, event)| matches!(event, GameEvent::CoinFlipped { .. }))
            .all(|(index, _)| index < first_sacrifice));
    }
    assert_eq!(
        round.runner.state().objects[&round.chance]
            .counters
            .get(&CounterType::Generic("luck".to_string()))
            .copied()
            .unwrap_or(0),
        0,
        "CR 705: heads/tails flips do not trigger Whenever you WIN a coin flip"
    );
    assert!(
        round
            .runner
            .state()
            .players
            .iter()
            .all(|player| player.life == 20),
        "CR 705: neither heads nor tails may fire Minotaur's win/lose damage triggers"
    );
}

#[test]
fn authentic_assassin_opponent_goblin_entry_binds_each_players_face_and_own_choice() {
    let mut saw_controller_only = false;
    let mut saw_opponent_only = false;
    let mut saw_both = false;
    let mut saw_neither = false;
    // Fixed, deterministic seed domain, with reach guards for BOTH mixed-result
    // directions: a last-player/controller-only implementation cannot pass.
    for seed in 0..32 {
        let mut round = round(seed, P1, false);
        let mut events = Vec::new();
        enter_goblin(&mut round, &mut events);
        to_choice(&mut round.runner, &mut events);
        assert_eq!(
            flips(&events).len(),
            2,
            "both instructed players must finish flipping"
        );
        let prompted = choose_sacrifices(&mut round, &mut events);
        saw_controller_only |= prompted == vec![P0];
        saw_opponent_only |= prompted == vec![P1];
        saw_both |= prompted == vec![P1, P0];
        saw_neither |= prompted.is_empty();
        assert_round(&round, &events, &[P1, P0]);
    }
    assert!(
        saw_controller_only && saw_opponent_only && saw_both && saw_neither,
        "the deterministic seed domain must reach both mixed directions and both uniform outcomes"
    );
}

#[test]
fn replacement_keep_choice_qualifies_only_kept_faces_after_the_entire_round() {
    for first_heads in [false, true] {
        let mut round = round(7, P0, true);
        let mut events = Vec::new();
        enter_goblin(&mut round, &mut events);
        reseed(&mut round.runner, u64::from(!first_heads));
        to_choice(&mut round.runner, &mut events);
        match round.runner.state().waiting_for.clone() {
            WaitingFor::CoinFlipKeepChoice {
                player,
                results,
                keep_count,
            } => {
                assert_eq!(player, P0);
                assert_eq!(results.len(), 2);
                assert_eq!(keep_count, 1);
                assert_eq!(results[0], first_heads, "seed 0 heads / seed 1 tails");
            }
            other => panic!("first player must keep a replaced coin: {other:?}"),
        }
        assert!(flips(&events).is_empty());
        // The next player's replacement generates its coins during this action.
        reseed(&mut round.runner, u64::from(first_heads));
        act(
            &mut round.runner,
            GameAction::SelectCoinFlips {
                keep_indices: vec![0],
            },
            &mut events,
        );
        assert_eq!(
            flips(&events),
            vec![(
                P0,
                if first_heads {
                    CoinFlipResult::Heads
                } else {
                    CoinFlipResult::Tails
                }
            )]
        );
        assert!(sacrifices(&events).is_empty());
        match round.runner.state().waiting_for.clone() {
            WaitingFor::CoinFlipKeepChoice {
                player, results, ..
            } => {
                assert_eq!(player, P1);
                assert_eq!(results[0], !first_heads);
            }
            other => panic!("all flips must finish before a sacrifice prompt: {other:?}"),
        }
        act(
            &mut round.runner,
            GameAction::SelectCoinFlips {
                keep_indices: vec![0],
            },
            &mut events,
        );
        let prompted = choose_sacrifices(&mut round, &mut events);
        assert_eq!(prompted, vec![if first_heads { P1 } else { P0 }]);
        assert_round(&round, &events, &[P0, P1]);
    }
}

#[test]
fn competing_replacements_resume_before_coin_keep_and_then_qualified_sacrifice() {
    let mut round = round(7, P0, true);
    let thumb = round
        .runner
        .state()
        .battlefield
        .iter()
        .copied()
        .find(|id| {
            let object = &round.runner.state().objects[id];
            object.name == "Krark's Thumb" && object.controller == P0
        })
        .expect("P0 Thumb");
    // Copy the authentic replacement definition, not its card name. The generic
    // mechanism supports two applicable doubling effects and CR 616 ordering.
    let second = round.runner.state().objects[&thumb].replacement_definitions[0].clone();
    set_replacements(&mut round.runner, round.chance, vec![second]);
    let mut events = Vec::new();
    enter_goblin(&mut round, &mut events);
    reseed(&mut round.runner, 1);
    to_choice(&mut round.runner, &mut events);
    assert!(matches!(
        round.runner.state().waiting_for,
        WaitingFor::ReplacementChoice { player: P0, .. }
    ));
    assert!(flips(&events).is_empty());
    act(
        &mut round.runner,
        GameAction::ChooseReplacement { index: 0 },
        &mut events,
    );
    match round.runner.state().waiting_for.clone() {
        WaitingFor::CoinFlipKeepChoice {
            player, results, ..
        } => {
            assert_eq!(player, P0);
            assert_eq!(
                results.len(),
                4,
                "both replacement doublers apply exactly once"
            );
            assert!(!results[0]);
        }
        other => panic!("replacement ordering must resume into coin keep: {other:?}"),
    }
    assert!(flips(&events).is_empty());
    reseed(&mut round.runner, 0);
    act(
        &mut round.runner,
        GameAction::SelectCoinFlips {
            keep_indices: vec![0],
        },
        &mut events,
    );
    assert!(matches!(
        round.runner.state().waiting_for,
        WaitingFor::CoinFlipKeepChoice { player: P1, .. }
    ));
    assert!(sacrifices(&events).is_empty());
    act(
        &mut round.runner,
        GameAction::SelectCoinFlips {
            keep_indices: vec![0],
        },
        &mut events,
    );
    assert_eq!(choose_sacrifices(&mut round, &mut events), vec![P0]);
    assert_round(&round, &events, &[P0, P1]);
}

#[test]
fn prevented_next_resolution_cannot_reuse_previous_players_tails() {
    let mut round = round(7, P0, true);
    let mut first = Vec::new();
    enter_goblin(&mut round, &mut first);
    reseed(&mut round.runner, 1);
    to_choice(&mut round.runner, &mut first);
    reseed(&mut round.runner, 1);
    act(
        &mut round.runner,
        GameAction::SelectCoinFlips {
            keep_indices: vec![0],
        },
        &mut first,
    );
    act(
        &mut round.runner,
        GameAction::SelectCoinFlips {
            keep_indices: vec![0],
        },
        &mut first,
    );
    assert_eq!(choose_sacrifices(&mut round, &mut first), vec![P0, P1]);
    assert_round(&round, &first, &[P0, P1]);

    // The generic count-zero replacement prevents each logical flip. Remove
    // only the fixture Thumb replacements to avoid an unrelated ordering choice.
    let thumbs: Vec<ObjectId> = round
        .runner
        .state()
        .battlefield
        .iter()
        .copied()
        .filter(|id| round.runner.state().objects[id].name == "Krark's Thumb")
        .collect();
    for id in thumbs {
        set_replacements(&mut round.runner, id, vec![]);
    }
    let mut prevent =
        ReplacementDefinition::new(ReplacementEvent::CoinFlip).execute(AbilityDefinition::new(
            AbilityKind::Spell,
            Effect::FlipCoins {
                count: QuantityExpr::Fixed { value: 0 },
                win_effect: None,
                lose_effect: None,
                flipper: TargetFilter::Controller,
            },
        ));
    prevent.valid_player = Some(ReplacementPlayerScope::AnyPlayer);
    set_replacements(&mut round.runner, round.chance, vec![prevent]);
    // A fresh genuine ETB resolution, not a condition-only injected ability.
    round.goblin = round
        .spare_goblin
        .take()
        .expect("second staged Goblin spell");
    round
        .creatures
        .retain(|(_, id)| round.runner.state().objects[id].zone == Zone::Battlefield);
    round.creatures.push((P0, round.goblin));
    let mut second = Vec::new();
    // The final sacrifice chooser may still hold priority. Use the public
    // pass action to return it to the staged Goblin's owner before casting.
    if round.runner.state().priority_player != P0 {
        act(&mut round.runner, GameAction::PassPriority, &mut second);
    }
    enter_goblin(&mut round, &mut second);
    to_choice(&mut round.runner, &mut second);
    assert!(
        flips(&second).is_empty(),
        "prevented flips emit no heads/tails outcome"
    );
    assert!(choose_sacrifices(&mut round, &mut second).is_empty());
    assert!(sacrifices(&second).is_empty());
    assert_round(&round, &second, &[]);
}

#[test]
fn tails_player_without_creatures_flips_but_cannot_choose_another_players_creature() {
    let mut scenario = GameScenario::new_n_player(2, 1);
    scenario.at_phase(Phase::PreCombatMain);
    let assassin = scenario
        .add_creature_to_hand(P0, "Goblin Assassin", 2, 2)
        .with_subtypes(vec!["Goblin", "Assassin"])
        .from_oracle_text(ASSASSIN)
        .id();
    let survivor = scenario.add_vanilla(P0, 2, 2);
    let chance = scenario
        .add_enchantment_from_oracle(P0, "Chance Encounter", CHANCE_ENCOUNTER)
        .id();
    scenario.add_artifact_from_oracle(P0, "Krark's Thumb", THUMB);
    scenario.add_artifact_from_oracle(P1, "Krark's Thumb", THUMB);
    let mut round = Round {
        runner: scenario.build(),
        assassin,
        goblin: assassin,
        spare_goblin: None,
        chance,
        coin_observer: None,
        creatures: vec![(P0, assassin), (P0, survivor)],
    };
    let mut events = Vec::new();
    enter_goblin(&mut round, &mut events);
    reseed(&mut round.runner, 0);
    to_choice(&mut round.runner, &mut events);
    reseed(&mut round.runner, 1);
    act(
        &mut round.runner,
        GameAction::SelectCoinFlips {
            keep_indices: vec![0],
        },
        &mut events,
    );
    act(
        &mut round.runner,
        GameAction::SelectCoinFlips {
            keep_indices: vec![0],
        },
        &mut events,
    );
    to_choice(&mut round.runner, &mut events);
    assert_eq!(
        flips(&events),
        vec![(P0, CoinFlipResult::Heads), (P1, CoinFlipResult::Tails)]
    );
    assert!(sacrifices(&events).is_empty());
    assert!(matches!(
        round.runner.state().waiting_for,
        WaitingFor::Priority { .. }
    ));
    assert!(round.runner.state().stack.is_empty());
    assert_eq!(
        round.runner.state().objects[&assassin].zone,
        Zone::Battlefield
    );
    assert_eq!(
        round.runner.state().objects[&survivor].zone,
        Zone::Battlefield
    );
}

#[test]
fn authentic_win_lose_flip_still_fires_both_result_triggers_and_its_printed_branch() {
    for (seed, result) in [(0, CoinFlipResult::Won), (1, CoinFlipResult::Lost)] {
        let mut scenario = GameScenario::new_n_player(2, seed);
        scenario.at_phase(Phase::PreCombatMain);
        scenario.with_mana_pool(
            P0,
            vec![ManaUnit::new(
                ManaType::Colorless,
                ObjectId(9001),
                false,
                vec![],
            )],
        );
        let chance = scenario
            .add_enchantment_from_oracle(P0, "Chance Encounter", CHANCE_ENCOUNTER)
            .id();
        let bottle = scenario
            .add_artifact_from_oracle(P0, "Bottle of Suleiman", BOTTLE)
            .id();
        scenario
            .add_creature(P0, "Karplusan Minotaur", 3, 3)
            .with_subtypes(vec!["Minotaur", "Warrior"])
            .from_oracle_text(MINOTAUR);
        let mut runner = scenario.build();
        // The Minotaur's losing trigger is chosen by the opponent; the real
        // trigger target prompt still permits that player to choose themself.
        let outcome = runner.activate(bottle, 0).target_player(P1).resolve();
        assert_eq!(flips(outcome.events()), vec![(P0, result)]);
        assert_eq!(runner.state().objects[&bottle].zone, Zone::Graveyard);
        assert_eq!(
            runner.state().objects[&chance]
                .counters
                .get(&CounterType::Generic("luck".to_string()))
                .copied()
                .unwrap_or(0),
            u32::from(result == CoinFlipResult::Won)
        );
        assert_eq!(
            runner.state().players[0].life,
            if result == CoinFlipResult::Won {
                20
            } else {
                15
            }
        );
        assert_eq!(runner.state().players[1].life, 19,
            "positive control: the authentic Minotaur's matching Won OR Lost trigger really deals damage");
        let djinn = runner
            .state()
            .battlefield
            .iter()
            .filter(|id| {
                runner.state().objects[id]
                    .card_types
                    .subtypes
                    .contains(&"Djinn".to_string())
            })
            .count();
        assert_eq!(djinn, usize::from(result == CoinFlipResult::Won));
    }
}
