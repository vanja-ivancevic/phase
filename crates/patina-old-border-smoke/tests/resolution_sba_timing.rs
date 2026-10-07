//! CR 704.3-704.4: player-loss SBAs wait for a resolving spell's choices.

use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::WaitingFor;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

#[test]
fn lethal_chain_lightning_finishes_optional_payment_before_player_loss() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain).with_life(P1, 1);
    let caster_mountain = scenario.add_basic_land(P0, ManaColor::Red);
    scenario.add_basic_land(P1, ManaColor::Red);
    scenario.add_basic_land(P1, ManaColor::Red);
    let spell = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Chain Lightning",
            false,
            "Chain Lightning deals 3 damage to any target. Then that player or that permanent's controller may pay {R}{R}. If the player does, they may copy this spell and may choose a new target for that copy.",
        )
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Red],
            generic: 0,
        })
        .id();
    let mut runner = scenario.build();
    runner.cast(spell).target_player(P1).commit();
    assert!(runner.state().objects[&caster_mountain].tapped);
    assert_eq!(runner.state().objects[&spell].zone, Zone::Stack);
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::Priority { player: P0 }
    ));
    let first = runner
        .act(GameAction::PassPriority)
        .expect("first priority pass");
    assert!(matches!(
        first.waiting_for,
        WaitingFor::Priority { player: P1 }
    ));
    let damage = runner
        .act(GameAction::PassPriority)
        .expect("resolve damage to the optional payment");
    assert_eq!(runner.life(P1), -2);
    assert!(!runner.state().players[1].is_eliminated);
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::OptionalEffectChoice { player: P1, source_id, .. } if source_id == spell
    ));
    assert!(runner.state().game_end.is_none());
    assert!(runner.state().active_optional_effect_frame().is_some());
    assert_eq!(
        runner
            .state()
            .resolving_stack_entry
            .as_ref()
            .expect("the spell retains its live resolving carrier")
            .source_id,
        spell
    );
    assert_eq!(
        first
            .events
            .iter()
            .chain(&damage.events)
            .filter(|event| {
                matches!(
                    event,
                    GameEvent::PlayerLost { .. } | GameEvent::GameOver { .. }
                )
            })
            .count(),
        0
    );

    let finished = runner
        .act(GameAction::DecideOptionalEffect { accept: false })
        .expect("the damaged player can decline before the spell finishes");
    assert!(matches!(
        finished.waiting_for,
        WaitingFor::GameOver { winner: Some(P0) }
    ));
    assert!(runner.state().players[1].is_eliminated);
    assert!(runner.state().resolution_stack.is_empty());
    assert!(runner.state().resolving_stack_entry.is_none());
    assert!(runner.state().active_spell_resolution().is_none());
    assert!(runner.state().pending_resolution_completion.is_none());
    assert!(runner.state().stack.is_empty());
    assert_eq!(runner.state().objects[&spell].zone, Zone::Graveyard);
    assert_eq!(
        finished
            .events
            .iter()
            .filter(|event| matches!(event, GameEvent::PlayerLost { player_id: P1 }))
            .count(),
        1
    );
    assert_eq!(
        finished
            .events
            .iter()
            .filter(|event| matches!(event, GameEvent::GameOver { winner: Some(P0) }))
            .count(),
        1
    );
}
