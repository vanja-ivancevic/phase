//! Reducer-backed announcement contracts: payment branches and sparse X values.
//!
//! These are consumer regressions, not historical replay evidence. The original
//! C0 failures were Thunderclap's unpayable printed-cost decline (game 21,
//! prefix 70) and Spell Blast's targetless X=1 (game 9, prefix 295).

use engine::ai_support::{
    apply_ai_action_proposal, candidate_actions, AiDecisionContract, AiProposalApplication,
};
use engine::game::engine::apply_interaction;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{
    AbilityCost, CastTimingPermission, Comparator, ContinuousModification, ControllerRef, Duration,
    FilterProp, QuantityExpr, StaticDefinition, TargetFilter, TargetRef, TypedFilter,
};
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::{
    ActionResult, CastPaymentMode, GameState, PayCostKind, WaitingFor,
};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::statics::{CastFrequency, CostModifyMode, CostReductionReach, StaticMode};
use engine::types::zones::Zone;

fn mana(kind: ManaType) -> ManaUnit {
    ManaUnit::new(kind, ObjectId(90_001), false, vec![])
}

/// Compare the whole enumerated announcement domain against the real public
/// reducer, including cancellation. A preview-only disagreement is a failure,
/// not permission to accept an illegal action or silently drop a legal one.
fn truthful_contract(state: &GameState) -> AiDecisionContract {
    let owner = state.waiting_for.acting_player().expect("one announcer");
    let contract = AiDecisionContract::issue(state, owner);
    for candidate in candidate_actions(state) {
        let mut live = state.clone();
        let executable = apply_interaction(
            &mut live,
            contract.authorized_actor,
            owner,
            candidate.action.clone(),
        )
        .is_ok();
        assert_eq!(
            contract.contains_action(state, &candidate.action),
            executable,
            "issued/public-reducer disagreement for {:?}",
            candidate.action
        );
    }
    for candidate in &contract.candidates {
        let mut live = state.clone();
        assert!(
            matches!(
                apply_ai_action_proposal(
                    &mut live,
                    &contract,
                    contract.authorized_actor,
                    candidate.action.clone(),
                ),
                AiProposalApplication::AppliedAction { .. }
            ),
            "issued announcement must submit successfully: {:?}",
            candidate.action
        );
    }
    contract
}

fn submit(runner: &mut GameRunner, wanted: GameAction) -> ActionResult {
    let owner = runner
        .state()
        .waiting_for
        .acting_player()
        .expect("one owner");
    let contract = AiDecisionContract::issue(runner.state(), owner);
    let issued = contract
        .candidates
        .iter()
        .find(|candidate| candidate.action == wanted)
        .unwrap_or_else(|| panic!("missing executable candidate {wanted:?}"))
        .action
        .clone();
    match apply_ai_action_proposal(
        runner.state_mut(),
        &contract,
        contract.authorized_actor,
        issued,
    ) {
        AiProposalApplication::AppliedAction { result } => result,
        other => panic!("issued announcement did not apply: {other:?}"),
    }
}

fn cast(runner: &mut GameRunner, card: ObjectId, targets: Vec<ObjectId>) {
    let card_id = runner.state().objects[&card].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: card,
            card_id,
            targets,
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("real cast announcement");
}

fn resolve_top(runner: &mut GameRunner) {
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::Priority { .. }
    ));
    runner.act(GameAction::PassPriority).expect("first pass");
    runner
        .act(GameAction::PassPriority)
        .expect("second pass resolves top");
}

fn thunderclap(payable_base: bool) -> (GameRunner, ObjectId, ObjectId, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let mountain = scenario.add_basic_land(P0, ManaColor::Red);
    if payable_base {
        scenario.with_mana_pool(
            P0,
            vec![
                mana(ManaType::Colorless),
                mana(ManaType::Colorless),
                mana(ManaType::Red),
            ],
        );
    }
    let target = scenario
        .add_creature(P1, "Three-toughness target", 2, 3)
        .id();
    let spell = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Thunderclap",
            true,
            "You may sacrifice a Mountain rather than pay this spell's mana cost.\nThunderclap deals 3 damage to target creature.",
        )
        .with_mana_cost(ManaCost::Cost { shards: vec![ManaCostShard::Red], generic: 2 })
        .id();
    let mut runner = scenario.build();
    // Sacrifice remains payable even when the Mountain cannot supply mana.
    runner
        .state_mut()
        .objects
        .get_mut(&mountain)
        .unwrap()
        .tapped = true;
    cast(&mut runner, spell, vec![target]);
    assert!(
        matches!(
            &runner.state().waiting_for,
            WaitingFor::OptionalCostChoice { pending_cast, .. }
                if !pending_cast.deferred_target_selection
                    && pending_cast.ability.targets == vec![TargetRef::Object(target)]
        ),
        "the selected target precedes this alternative-cost decision"
    );
    (runner, spell, mountain, target)
}
fn reject_thunderclap_decline(
    runner: &mut GameRunner,
    spell: ObjectId,
    mountain: ObjectId,
    target: ObjectId,
) {
    let spell_zone = runner.state().objects[&spell].zone;
    let stack: Vec<_> = runner.state().stack.iter().map(|entry| entry.id).collect();
    assert!(apply_interaction(
        runner.state_mut(),
        P0,
        P0,
        GameAction::DecideOptionalCost { pay: false },
    )
    .is_err());
    assert_eq!(runner.state().objects[&spell].zone, spell_zone);
    assert_eq!(runner.state().objects[&mountain].zone, Zone::Battlefield);
    assert!(runner.state().objects[&mountain].tapped);
    assert_eq!(runner.state().objects[&target].zone, Zone::Battlefield);
    assert_eq!(runner.state().players[0].mana_pool.total(), 0);
    assert_eq!(runner.state().players[0].life, 20);
    assert_eq!(runner.state().players[1].life, 20);
    assert_eq!(
        runner
            .state()
            .stack
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        stack
    );
    assert!(
        matches!(
            &runner.state().waiting_for,
            WaitingFor::OptionalCostChoice { player, pending_cast, .. }
                if *player == P0 && pending_cast.object_id == spell
                    && pending_cast.ability.targets == vec![TargetRef::Object(target)]
        ),
        "the rejected decline leaves the original decision and target available"
    );
}

#[test]
fn unpayable_printed_cost_decline_is_not_issued_but_sacrifice_executes() {
    let (mut runner, spell, mountain, target) = thunderclap(false);
    let contract = truthful_contract(runner.state());
    assert!(!contract.contains_action(
        runner.state(),
        &GameAction::DecideOptionalCost { pay: false }
    ));
    assert!(contract.contains_action(
        runner.state(),
        &GameAction::DecideOptionalCost { pay: true }
    ));
    assert!(contract.contains_action(runner.state(), &GameAction::CancelCast));
    reject_thunderclap_decline(&mut runner, spell, mountain, target);
    submit(&mut runner, GameAction::DecideOptionalCost { pay: true });
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::PayCost {
            kind: PayCostKind::Sacrifice,
            ..
        }
    ));
    submit(
        &mut runner,
        GameAction::SelectCards {
            cards: vec![mountain],
        },
    );
    assert_eq!(runner.state().objects[&mountain].zone, Zone::Graveyard);
    assert_eq!(runner.state().objects[&spell].zone, Zone::Stack);
    assert_eq!(runner.state().players[0].mana_pool.total(), 0);
    assert!(runner.state().pending_cast.is_none());
    assert_eq!(runner.state().stack.len(), 1);
    resolve_top(&mut runner);
    assert_eq!(runner.state().objects[&target].zone, Zone::Graveyard);
    assert_eq!(runner.state().objects[&spell].zone, Zone::Graveyard);
    assert!(runner.state().stack.is_empty());
    assert_eq!(runner.state().players[0].life, 20);
    assert_eq!(runner.state().players[1].life, 20);
}

#[test]
fn both_payable_thunderclap_branches_remain_executable() {
    for pay in [false, true] {
        let (mut runner, spell, mountain, target) = thunderclap(true);
        let contract = truthful_contract(runner.state());
        assert!(contract.contains_action(
            runner.state(),
            &GameAction::DecideOptionalCost { pay: false }
        ));
        assert!(contract.contains_action(
            runner.state(),
            &GameAction::DecideOptionalCost { pay: true }
        ));
        submit(&mut runner, GameAction::DecideOptionalCost { pay });
        if pay {
            submit(
                &mut runner,
                GameAction::SelectCards {
                    cards: vec![mountain],
                },
            );
            assert_eq!(runner.state().objects[&mountain].zone, Zone::Graveyard);
            assert_eq!(runner.state().players[0].mana_pool.total(), 3);
        } else {
            assert_eq!(runner.state().objects[&mountain].zone, Zone::Battlefield);
            assert!(runner.state().objects[&mountain].tapped);
            assert_eq!(runner.state().players[0].mana_pool.total(), 0);
        }
        assert!(runner.state().pending_cast.is_none());
        assert_eq!(runner.state().objects[&spell].zone, Zone::Stack);
        resolve_top(&mut runner);
        assert_eq!(runner.state().objects[&target].zone, Zone::Graveyard);
        assert_eq!(runner.state().objects[&spell].zone, Zone::Graveyard);
        assert!(runner.state().stack.is_empty());
    }
}

#[test]
fn cancellation_preserves_alternative_cost_resources_and_returns_the_spell() {
    for payable in [false, true] {
        let (mut runner, spell, mountain, target) = thunderclap(payable);
        truthful_contract(runner.state());
        if !payable {
            reject_thunderclap_decline(&mut runner, spell, mountain, target);
        }
        submit(&mut runner, GameAction::CancelCast);
        assert_eq!(runner.state().objects[&spell].zone, Zone::Hand);
        assert!(runner.state().players[0].hand.contains(&spell));
        assert_eq!(runner.state().objects[&mountain].zone, Zone::Battlefield);
        assert!(runner.state().objects[&mountain].tapped);
        assert_eq!(runner.state().objects[&target].zone, Zone::Battlefield);
        assert_eq!(
            runner.state().players[0].mana_pool.total(),
            if payable { 3 } else { 0 }
        );
        assert!(runner.state().stack.is_empty());
        assert!(runner.state().pending_cast.is_none());
        assert!(
            matches!(runner.state().waiting_for, WaitingFor::Priority { player } if player == P0)
        );
    }
}

fn spell_blast() -> (GameRunner, ObjectId, ObjectId, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let mut pool = vec![mana(ManaType::Colorless); 11];
    pool.push(mana(ManaType::Blue));
    scenario.with_mana_pool(P0, pool);
    let two = scenario
        .add_spell_to_hand_from_oracle(P0, "Mana value two spell", true, "You gain 1 life.")
        .with_mana_cost(ManaCost::generic(2))
        .id();
    let four = scenario
        .add_spell_to_hand_from_oracle(P0, "Mana value four spell", true, "You gain 1 life.")
        .with_mana_cost(ManaCost::generic(4))
        .id();
    let counter = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Spell Blast",
            true,
            "Counter target spell with mana value X.",
        )
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::X, ManaCostShard::Blue],
            generic: 0,
        })
        .id();
    let mut runner = scenario.build();
    cast(&mut runner, two, vec![]);
    cast(&mut runner, four, vec![]);
    assert_eq!(runner.state().stack.len(), 2);
    assert_eq!(runner.state().players[0].mana_pool.total(), 6);
    cast(&mut runner, counter, vec![]);
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::ChooseXValue { min: 0, max: 5, .. }
    ));
    (runner, counter, two, four)
}
fn reject_targetless_spell_blast_x(
    runner: &mut GameRunner,
    counter: ObjectId,
    two: ObjectId,
    four: ObjectId,
) {
    let counter_zone = runner.state().objects[&counter].zone;
    let stack: Vec<_> = runner.state().stack.iter().map(|entry| entry.id).collect();
    for value in [0, 1, 3, 5] {
        assert!(
            apply_interaction(runner.state_mut(), P0, P0, GameAction::ChooseX { value },).is_err(),
            "X={value} has no matching spell to target"
        );
        assert_eq!(runner.state().objects[&counter].zone, counter_zone);
        assert_eq!(runner.state().objects[&two].zone, Zone::Stack);
        assert_eq!(runner.state().objects[&four].zone, Zone::Stack);
        assert_eq!(runner.state().players[0].mana_pool.total(), 6);
        assert_eq!(runner.state().players[0].life, 20);
        assert_eq!(runner.state().players[1].life, 20);
        assert_eq!(
            runner
                .state()
                .stack
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            stack
        );
        assert!(
            matches!(
                &runner.state().waiting_for,
                WaitingFor::ChooseXValue { player, min: 0, max: 5, pending_cast, .. }
                    if *player == P0 && pending_cast.object_id == counter
                        && pending_cast.ability.chosen_x.is_none()
            ),
            "rejection leaves X unannounced and the original decision available"
        );
    }
}

#[test]
fn target_coupled_x_contract_is_sparse_and_each_legal_value_counters_its_target() {
    for (value, counter_two) in [(2, true), (4, false)] {
        let (mut runner, counter, two, four) = spell_blast();
        let contract = truthful_contract(runner.state());
        let values: Vec<_> = contract
            .candidates
            .iter()
            .filter_map(|candidate| match candidate.action {
                GameAction::ChooseX { value } => Some(value),
                _ => None,
            })
            .collect();
        assert_eq!(values, vec![2, 4]);
        assert!(contract.contains_action(runner.state(), &GameAction::CancelCast));
        reject_targetless_spell_blast_x(&mut runner, counter, two, four);
        let result = submit(&mut runner, GameAction::ChooseX { value });
        assert!(result.events.iter().any(|event| matches!(event,
            GameEvent::XValueChosen { object_id, value: chosen, .. }
                if *object_id == counter && *chosen == value)));
        let target = if counter_two { two } else { four };
        let survivor = if counter_two { four } else { two };
        // The established target authority auto-declares the unique legal
        // assignment; X does not need a redundant target-choice round trip.
        let ability = runner.state().stack.back().unwrap().ability().unwrap();
        assert_eq!(ability.targets, vec![TargetRef::Object(target)]);
        assert_eq!(ability.chosen_x, Some(value));
        assert!(runner.state().pending_cast.is_none());
        assert_eq!(runner.state().stack.len(), 3);
        assert_eq!(
            runner.state().players[0].mana_pool.total(),
            5 - value as usize
        );
        assert_eq!(runner.state().objects[&counter].zone, Zone::Stack);
        resolve_top(&mut runner);
        assert_eq!(runner.state().objects[&target].zone, Zone::Graveyard);
        assert_eq!(runner.state().objects[&counter].zone, Zone::Graveyard);
        assert_eq!(runner.state().objects[&survivor].zone, Zone::Stack);
        assert_eq!(runner.state().stack.len(), 1);
        assert_eq!(
            runner.state().players[0].life,
            20,
            "countered spell never resolves"
        );
        resolve_top(&mut runner);
        assert!(runner.state().stack.is_empty());
        assert_eq!(
            runner.state().players[0].life,
            21,
            "the other spell still resolves"
        );
        assert_eq!(runner.state().players[1].life, 20);
    }
}

#[test]
fn cancelling_sparse_x_preserves_the_original_stack_and_all_unspent_mana() {
    let (mut runner, counter, two, four) = spell_blast();
    truthful_contract(runner.state());
    reject_targetless_spell_blast_x(&mut runner, counter, two, four);
    submit(&mut runner, GameAction::CancelCast);
    assert_eq!(runner.state().objects[&counter].zone, Zone::Hand);
    assert!(runner.state().players[0].hand.contains(&counter));
    assert_eq!(runner.state().objects[&two].zone, Zone::Stack);
    assert_eq!(runner.state().objects[&four].zone, Zone::Stack);
    assert_eq!(runner.state().stack.len(), 2);
    assert_eq!(runner.state().players[0].mana_pool.total(), 6);
    assert!(runner.state().pending_cast.is_none());
    resolve_top(&mut runner);
    resolve_top(&mut runner);
    assert!(runner.state().stack.is_empty());
    assert_eq!(runner.state().players[0].life, 22);
}

#[test]
fn targetless_x_keeps_every_affordable_value_and_consumes_the_chosen_payment() {
    for value in 0..=4 {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let mut pool = vec![mana(ManaType::Colorless); 4];
        pool.push(mana(ManaType::White));
        scenario.with_mana_pool(P0, pool);
        let spell = scenario
            .add_spell_to_hand_from_oracle(P0, "Targetless X life spell", true, "You gain X life.")
            .with_mana_cost(ManaCost::Cost {
                shards: vec![ManaCostShard::X, ManaCostShard::White],
                generic: 0,
            })
            .id();
        let mut runner = scenario.build();
        cast(&mut runner, spell, vec![]);
        let contract = truthful_contract(runner.state());
        for affordable in 0..=4 {
            assert!(contract
                .contains_action(runner.state(), &GameAction::ChooseX { value: affordable }));
        }
        assert!(contract.contains_action(runner.state(), &GameAction::CancelCast));
        submit(&mut runner, GameAction::ChooseX { value });
        assert!(runner.state().pending_cast.is_none());
        assert_eq!(runner.state().objects[&spell].zone, Zone::Stack);
        assert_eq!(
            runner.state().players[0].mana_pool.total(),
            4 - value as usize
        );
        resolve_top(&mut runner);
        assert_eq!(runner.state().players[0].life, 20 + value as i32);
        assert_eq!(runner.state().objects[&spell].zone, Zone::Graveyard);
        assert!(runner.state().stack.is_empty());
    }
}

#[test]
fn nullmage_requires_two_opponent_cards_before_issuing_and_resolves_all_targets() {
    const ORACLE: &str = "{T}: Return two target cards from an opponent's graveyard to their hand. Destroy target artifact or enchantment.";
    for hero in [P0, P1] {
        for count in 0..=2 {
            let opponent = if hero == P0 { P1 } else { P0 };
            let mut scenario = GameScenario::new();
            scenario.at_phase(Phase::PreCombatMain);
            scenario.with_graveyard(hero, &["Own Card A", "Own Card B", "Own Card C"]);
            for name in ["Returned Card A", "Returned Card B"]
                .into_iter()
                .take(count)
            {
                scenario.with_graveyard(opponent, &[name]);
            }
            let source = scenario
                .add_creature_from_oracle(hero, "Nullmage Advocate", 2, 3, ORACLE)
                .id();
            let relic = scenario
                .add_artifact_from_oracle(opponent, "Sol Ring", "{T}: Add {C}{C}.")
                .id();
            let mut runner = scenario.build();
            if hero == P1 {
                submit(&mut runner, GameAction::PassPriority);
            }
            let activation = GameAction::ActivateAbility {
                source_id: source,
                ability_index: 0,
            };
            let contract = AiDecisionContract::issue(runner.state(), hero);
            assert_eq!(
                contract.contains_action(runner.state(), &activation),
                count == 2,
                "the controller's three graveyard cards cannot satisfy an opponent's minimum"
            );
            if count < 2 {
                let before = serde_json::to_value(runner.state()).unwrap();
                assert!(runner.act(activation).is_err());
                assert_eq!(serde_json::to_value(runner.state()).unwrap(), before);
                continue;
            }
            let returned = runner.state().players[opponent.0 as usize]
                .graveyard
                .clone();
            submit(&mut runner, activation);
            for target in returned.iter().copied().chain(std::iter::once(relic)) {
                submit(
                    &mut runner,
                    GameAction::ChooseTarget {
                        target: Some(TargetRef::Object(target)),
                    },
                );
            }
            assert!(runner.state().objects[&source].tapped);
            resolve_top(&mut runner);
            let opponent_state = &runner.state().players[opponent.0 as usize];
            for name in ["Returned Card A", "Returned Card B"] {
                assert!(opponent_state
                    .hand
                    .iter()
                    .any(|id| runner.state().objects[id].name == name));
            }
            assert_eq!(runner.state().players[hero.0 as usize].graveyard.len(), 3);
            assert!(opponent_state
                .graveyard
                .iter()
                .any(|id| runner.state().objects[id].name == "Sol Ring"));
            assert!(!runner.state().battlefield.contains(&relic));
        }
    }
}

#[test]
fn fireball_unpayable_extra_target_does_not_enumerate_target_permutations() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(
        P0,
        vec![
            mana(ManaType::Red),
            mana(ManaType::Colorless),
            mana(ManaType::Colorless),
            mana(ManaType::Colorless),
        ],
    );
    let targets: Vec<_> = (0..12)
        .map(|index| {
            scenario
                .add_creature(P1, &format!("Fireball target {index}"), 2, 10)
                .id()
        })
        .collect();
    let spell = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Fireball",
            false,
            "This spell costs {1} more to cast for each target beyond the first.\nFireball deals X damage divided evenly, rounded down, among any number of targets.",
        )
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::X, ManaCostShard::Red],
            generic: 0,
        })
        .id();
    let mut runner = scenario.build();
    cast(&mut runner, spell, vec![]);
    submit(&mut runner, GameAction::ChooseX { value: 3 });
    submit(
        &mut runner,
        GameAction::ChooseTarget {
            target: Some(TargetRef::Object(targets[0])),
        },
    );

    // Every extra target is already over the mana available. Enumerating all
    // permutations of the twelve bodies cannot make that prefix payable.
    let contract = AiDecisionContract::issue(runner.state(), P0);
    assert!(contract.contains_action(runner.state(), &GameAction::ChooseTarget { target: None }));
    for target in targets.iter().skip(1) {
        assert!(!contract.contains_action(
            runner.state(),
            &GameAction::ChooseTarget {
                target: Some(TargetRef::Object(*target)),
            }
        ));
    }
    assert!(!contract.contains_action(
        runner.state(),
        &GameAction::ChooseTarget {
            target: Some(TargetRef::Player(P1)),
        }
    ));
    submit(&mut runner, GameAction::ChooseTarget { target: None });
    assert_eq!(runner.state().players[0].mana_pool.total(), 0);
    assert_eq!(runner.state().objects[&spell].zone, Zone::Stack);
    resolve_top(&mut runner);
    assert_eq!(runner.state().objects[&targets[0]].damage_marked, 3);
    for target in targets.iter().skip(1) {
        assert_eq!(runner.state().objects[target].damage_marked, 0);
    }
}

#[test]
fn target_dependent_rebate_keeps_an_initially_unpayable_prefix_completable() {
    target_dependent_rebate_keeps_a_payable_completion(false);
}

#[test]
fn transient_target_dependent_rebate_keeps_an_initially_unpayable_prefix_completable() {
    target_dependent_rebate_keeps_a_payable_completion(true);
}

fn target_dependent_rebate_keeps_a_payable_completion(player_wide: bool) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(P0, vec![mana(ManaType::Red), mana(ManaType::Colorless)]);
    let first = scenario.add_creature(P1, "First hostile target", 2, 8).id();
    let second = scenario
        .add_creature(P1, "Second hostile target", 2, 8)
        .id();
    let friendly = scenario.add_creature(P0, "Rebate target", 2, 8).id();
    let mut definition = Some(
        StaticDefinition::new(StaticMode::ModifyCost {
            mode: CostModifyMode::Reduce,
            amount: ManaCost::generic(2),
            spell_filter: Some(TargetFilter::Typed(TypedFilter::card().properties(vec![
                FilterProp::Targets {
                    filter: Box::new(TargetFilter::Typed(
                        TypedFilter::creature().controller(ControllerRef::You),
                    )),
                },
            ]))),
            dynamic_count: None,
            reach: CostReductionReach::SpillsToGeneric,
        })
        .affected(TargetFilter::Typed(
            TypedFilter::card().controller(ControllerRef::You),
        )),
    );
    let source = {
        let mut source = scenario.add_creature(P0, "Target-sensitive rebate source", 2, 8);
        if !player_wide {
            source.with_static_definition(definition.take().unwrap());
        }
        source.id()
    };
    let spell = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Target-count surcharge mechanism",
            false,
            "This spell costs {1} more to cast for each target beyond the first.\nTarget-count surcharge mechanism deals 3 damage divided evenly, rounded down, among any number of targets.",
        )
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Red],
            generic: 1,
        })
        .id();
    let mut runner = scenario.build();
    if player_wide {
        runner.state_mut().add_transient_continuous_effect(
            source,
            P0,
            Duration::UntilEndOfTurn,
            TargetFilter::SpecificPlayer { id: P0 },
            vec![ContinuousModification::GrantStaticAbility {
                definition: Box::new(definition.unwrap()),
            }],
            None,
        );
    }
    cast(&mut runner, spell, vec![]);
    for target in [first, second, friendly] {
        // The two-hostile-target prefix costs {2}{R}, more than the pool.
        // Its friendly third target activates the {2} rebate, bringing the
        // final obligation back to {1}{R}. A cost-only cutoff must not drop it.
        submit(
            &mut runner,
            GameAction::ChooseTarget {
                target: Some(TargetRef::Object(target)),
            },
        );
    }
    submit(&mut runner, GameAction::ChooseTarget { target: None });
    assert_eq!(runner.state().players[0].mana_pool.total(), 0);
    assert_eq!(runner.state().objects[&spell].zone, Zone::Stack);
    resolve_top(&mut runner);
    for target in [first, second, friendly] {
        assert_eq!(runner.state().objects[&target].damage_marked, 1);
    }
}

const SPINNING_DARKNESS: &str = "You may exile the top three black cards of your graveyard rather than pay this spell's mana cost.\nSpinning Darkness deals 3 damage to target nonblack creature. You gain 3 life.";

fn spinning_darkness(
    hero: PlayerId,
    black_count: usize,
    tax_payment: Option<usize>,
) -> (GameRunner, ObjectId, ObjectId, Vec<ObjectId>) {
    let opponent = if hero == P0 { P1 } else { P0 };
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    if let Some(units) = tax_payment {
        scenario.add_artifact_from_oracle(
            opponent,
            "Sphere of Resistance",
            "Spells cost {1} more to cast.",
        );
        scenario.with_mana_pool(hero, vec![mana(ManaType::Colorless); units]);
    }
    let mut black = Vec::new();
    for index in 0..black_count {
        black.push(
            scenario
                .add_creature_to_graveyard(hero, &format!("Black {index}"), 1, 1)
                .with_color(vec![ManaColor::Black])
                .id(),
        );
        scenario
            .add_creature_to_graveyard(hero, &format!("Interleaved white {index}"), 1, 1)
            .with_color(vec![ManaColor::White]);
    }
    for index in 0..3 {
        scenario
            .add_creature_to_graveyard(opponent, &format!("Opponent black {index}"), 1, 1)
            .with_color(vec![ManaColor::Black]);
    }
    let target = scenario
        .add_creature(opponent, "Nonblack target", 2, 4)
        .with_color(vec![ManaColor::Blue])
        .id();
    let spell = scenario
        .add_spell_to_hand_from_oracle(hero, "Spinning Darkness", true, SPINNING_DARKNESS)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Black, ManaCostShard::Black],
            generic: 4,
        })
        .with_color(vec![ManaColor::Black])
        .id();
    let mut runner = scenario.build();
    if hero == P1 {
        submit(&mut runner, GameAction::PassPriority);
    }
    (runner, spell, target, black)
}

fn submit_card_set(runner: &mut GameRunner, expected: &[ObjectId]) {
    let owner = runner.state().waiting_for.acting_player().unwrap();
    let contract = AiDecisionContract::issue(runner.state(), owner);
    let selected = contract
        .candidates
        .iter()
        .find(|candidate| {
            matches!(&candidate.action, GameAction::SelectCards { cards }
            if cards.len() == expected.len() && cards.iter().all(|id| expected.contains(id)))
        })
        .expect("the exact cost set must be issued")
        .action
        .clone();
    submit(runner, selected);
}

#[test]
fn spinning_darkness_exiles_only_the_top_three_matching_owned_cards_and_resolves() {
    for hero in [P0, P1] {
        let opponent = if hero == P0 { P1 } else { P0 };
        let (mut runner, spell, target, black) = spinning_darkness(hero, 4, None);
        let original = runner.state().players[hero.0 as usize].graveyard.clone();
        let opposing = runner.state().players[opponent.0 as usize]
            .graveyard
            .clone();
        let top = [black[3], black[2], black[1]];
        cast(&mut runner, spell, vec![target]);
        submit(&mut runner, GameAction::DecideOptionalCost { pay: true });
        match &runner.state().waiting_for {
            WaitingFor::PayCost {
                choices,
                count,
                min_count,
                ..
            } => {
                assert_eq!(choices, &top);
                assert_eq!((*count, *min_count), (3, 3));
            }
            other => panic!("expected a fixed ordered exile cost, got {other:?}"),
        }
        for wrong in [vec![black[3], black[2]], vec![black[3], black[2], black[0]]] {
            let before = serde_json::to_value(runner.state()).unwrap();
            assert!(runner
                .act(GameAction::SelectCards { cards: wrong })
                .is_err());
            assert_eq!(serde_json::to_value(runner.state()).unwrap(), before);
        }
        submit_card_set(&mut runner, &top);
        assert_eq!(runner.state().objects[&spell].zone, Zone::Stack);
        assert_eq!(
            runner.state().players[hero.0 as usize].graveyard,
            original
                .iter()
                .copied()
                .filter(|id| !top.contains(id))
                .collect::<engine::im::Vector<_>>(),
        );
        assert_eq!(
            runner.state().players[opponent.0 as usize].graveyard,
            opposing
        );
        for id in top {
            assert_eq!(runner.state().objects[&id].zone, Zone::Exile);
        }
        resolve_top(&mut runner);
        assert_eq!(runner.state().objects[&target].damage_marked, 3);
        assert_eq!(runner.state().players[hero.0 as usize].life, 23);
        assert_eq!(runner.state().objects[&spell].zone, Zone::Graveyard);
    }
}

#[test]
fn spinning_darkness_rejects_insufficient_quality_and_retains_a_generic_tax() {
    for (black_count, tax_payment) in [(2, None), (3, Some(0))] {
        let (mut runner, spell, target, _) = spinning_darkness(P0, black_count, tax_payment);
        let before = serde_json::to_value(runner.state()).unwrap();
        let card_id = runner.state().objects[&spell].card_id;
        assert!(
            runner
                .act(GameAction::CastSpell {
                    object_id: spell,
                    card_id,
                    targets: vec![target],
                    payment_mode: CastPaymentMode::Auto,
                })
                .is_err(),
            "unpayable control: black_count={black_count}, tax_payment={tax_payment:?}"
        );
        assert_eq!(serde_json::to_value(runner.state()).unwrap(), before);
    }
    let (mut runner, spell, target, black) = spinning_darkness(P0, 3, Some(1));
    cast(&mut runner, spell, vec![target]);
    submit(&mut runner, GameAction::DecideOptionalCost { pay: true });
    submit_card_set(&mut runner, &black);
    assert_eq!(runner.state().players[0].mana_pool.total(), 0);
    assert_eq!(runner.state().objects[&spell].zone, Zone::Stack);
    resolve_top(&mut runner);
    assert_eq!(runner.state().objects[&target].damage_marked, 3);
    assert_eq!(runner.state().players[0].life, 23);
}

#[test]
fn nonmana_alternative_base_survives_tracked_exile_cost_recomputation() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.add_artifact_from_oracle(P1, "Sphere of Resistance", "Spells cost {1} more to cast.");
    scenario.with_mana_pool(P0, vec![mana(ManaType::Colorless)]);
    scenario.with_graveyard(P0, &["Chosen cost card"]);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Any-exile alternative", true, "You gain 3 life.")
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Black, ManaCostShard::Black],
            generic: 4,
        })
        .id();
    let mut runner = scenario.build();
    // Consume the legacy exported effect-cost representation. Its tracked-set
    // payment recomputes mana; that must use the elected zero base plus the tax.
    runner
        .state_mut()
        .objects
        .get_mut(&spell)
        .unwrap()
        .casting_options = vec![serde_json::from_value(serde_json::json!({
        "kind": "AlternativeCost",
        "cost": {
            "type": "EffectCost",
            "effect": {
                "type": "ChangeZone",
                "origin": "Graveyard",
                "destination": "Exile",
                "target": { "type": "Any" },
                "owner_library": false,
                "enter_transformed": false,
                "enter_tapped": false,
                "enters_attacking": false
            }
        }
    }))
    .unwrap()];
    let cost_card = runner.state().players[0].graveyard[0];
    cast(&mut runner, spell, vec![]);
    submit(&mut runner, GameAction::DecideOptionalCost { pay: true });
    submit_card_set(&mut runner, std::slice::from_ref(&cost_card));
    assert_eq!(runner.state().objects[&cost_card].zone, Zone::Exile);
    assert_eq!(runner.state().players[0].mana_pool.total(), 0);
    assert_eq!(runner.state().objects[&spell].zone, Zone::Stack);
    resolve_top(&mut runner);
    assert_eq!(runner.state().players[0].life, 23);
}

#[test]
fn timing_required_nonmana_alternative_replaces_printed_base_but_retains_the_tax() {
    for (mana_units, expected_accept) in [(0, false), (1, true)] {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::End);
        scenario
            .add_creature(P0, "Primal Prayers grant carrier", 1, 1)
            .with_static_definition(
                StaticDefinition::new(StaticMode::CastWithAlternativeCost {
                    cost: AbilityCost::PayEnergy {
                        amount: QuantityExpr::Fixed { value: 1 },
                    },
                    timing_permission: Some(CastTimingPermission::AsThoughHadFlash),
                    frequency: CastFrequency::Unlimited,
                })
                .affected(TargetFilter::Typed(
                    TypedFilter::creature()
                        .controller(ControllerRef::You)
                        .properties(vec![FilterProp::Cmc {
                            comparator: Comparator::LE,
                            value: QuantityExpr::Fixed { value: 3 },
                        }]),
                ))
                .active_zones(vec![Zone::Battlefield]),
            );
        scenario.add_artifact_from_oracle(
            P1,
            "Sphere of Resistance",
            "Spells cost {1} more to cast.",
        );
        scenario.with_mana_pool(P0, vec![mana(ManaType::Colorless); mana_units]);
        let spell = scenario
            .add_creature_to_hand(P0, "Creature consuming granted energy cost", 2, 2)
            .with_mana_cost(ManaCost::generic(2))
            .id();
        let mut runner = scenario.build();
        let state = runner.state_mut();
        state.active_player = P1;
        state.priority_player = P0;
        state.waiting_for = WaitingFor::Priority { player: P0 };
        state.players[0].energy = 1;
        state.objects.get_mut(&spell).unwrap().casting_options.push(
            engine::types::ability::SpellCastingOption::alternative_cost(AbilityCost::Mana {
                cost: ManaCost::generic(1),
            }),
        );
        let before = serde_json::to_value(runner.state()).unwrap();
        let action = GameAction::CastSpell {
            object_id: spell,
            card_id: runner.state().objects[&spell].card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        };
        let contract = AiDecisionContract::issue(runner.state(), P0);
        assert_eq!(
            contract.contains_action(runner.state(), &action),
            expected_accept
        );
        if !expected_accept {
            assert!(runner.act(action).is_err());
            assert_eq!(serde_json::to_value(runner.state()).unwrap(), before);
            continue;
        }
        submit(&mut runner, action);
        assert_eq!(runner.state().players[0].energy, 0);
        assert_eq!(runner.state().players[0].mana_pool.total(), 0);
        assert_eq!(runner.state().objects[&spell].zone, Zone::Stack);
        assert_eq!(
            runner.state().objects[&spell].cast_timing_permission,
            Some((
                CastTimingPermission::AsThoughHadFlash,
                runner.state().turn_number
            ))
        );
        resolve_top(&mut runner);
        assert_eq!(runner.state().objects[&spell].zone, Zone::Battlefield);
    }
}
