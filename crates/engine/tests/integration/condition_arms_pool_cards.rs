//! Three old-border pool cards whose intervening-`if` clause used to be dropped.
//!
//! Every body here exercises the parser's PUBLIC surface (`parse_oracle_text`)
//! and asserts the typed condition the parse now carries. Before these arms the
//! three lines lowered with `condition: None` — a silent drop for Zealots
//! en-Dal, a `Condition_If`/`Duration_ThisTurn` swallow pair for Krovikan
//! Vampire, and a whole-line `DynamicQty` swallow for Chaos Lord.
//!
//! * **Chaos Lord** — "if the number of permanents is even": parity. The AST
//!   carries no modulo operator, and adding one to `QuantityExpr` would oblige
//!   every quantity walker in the crate to classify it, so the condition is
//!   encoded with the existing half-rounding pair:
//!   `n is even ⟺ ⌊n/2⌋ == ⌈n/2⌉` (and `!=` for odd — see
//!   `parser/oracle_nom/condition.rs::parity_comparison`).
//! * **Zealots en-Dal** — "if all nonland permanents you control are white":
//!   count of the population vs count of the population restricted to the
//!   colour. The coloured set is a subset by construction, so equality holds
//!   exactly when every member is white — and an empty board gives `0 == 0`,
//!   which is the vacuously-true printed reading.
//! * **Krovikan Vampire** — "if a creature dealt damage by this creature this
//!   turn died": the phase-trigger reading of the damage-history death
//!   predicate. Its trigger event is the end step, not a death, so the
//!   condition reads the turn ledgers; the body's "that card" referent is bound
//!   to the same predicate instead of being left as an unbound `ParentTarget`
//!   that no target slot could ever resolve.
//!
//! **Stack size.** `parse_oracle_text` overflows the default 8 MB test stack
//! and prints a convincing PARTIAL negative on the way down. Every body that
//! calls it therefore runs on a 256 MB thread via `on_big_stack`.

use engine::parser::oracle::ParsedAbilities;
use engine::parser::oracle_ir::diagnostic::OracleDiagnostic;
use engine::parser::parse_oracle_text;
use engine::types::ability::{
    AbilityCondition, AbilityDefinition, Comparator, ControllerRef, Effect, FilterProp,
    QuantityExpr, QuantityRef, RoundingMode, TargetFilter, TriggerCondition, TriggerDefinition,
    TypeFilter, TypedFilter,
};
use engine::types::mana::ManaColor;
use engine::types::triggers::TriggerMode;
use engine::types::zones::Zone;
use engine::types::Phase;

/// `parse_oracle_text` recurses deeply enough to blow the default 8 MB test
/// stack. A blown stack does NOT look like a failure — it looks like a
/// plausible partial parse. Run every such body on 256 MB.
fn on_big_stack<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn 256MB parser thread")
        .join()
        .expect("parser thread must not panic")
}

fn parse_creature(text: &'static str, name: &'static str) -> ParsedAbilities {
    on_big_stack(move || parse_oracle_text(text, name, &[], &["Creature".to_string()], &[]))
}

/// Every `SwallowedClause` detector that fired for this parse.
fn swallowed_detectors(parsed: &ParsedAbilities) -> Vec<String> {
    parsed
        .parse_warnings
        .iter()
        .filter_map(|warning| match warning {
            OracleDiagnostic::SwallowedClause { detector, .. } => Some(detector.clone()),
            _ => None,
        })
        .collect()
}

/// Assert that no swallow warning named `detector` for this parse, printing the
/// full warning list when one did.
fn assert_no_swallow(parsed: &ParsedAbilities, detector: &str, context: &str) {
    let detectors = swallowed_detectors(parsed);
    assert!(
        !detectors.iter().any(|d| d == detector),
        "{context}: the parser must represent this clause, not swallow it — \
         expected no `{detector}` swallow warning, got {detectors:?}"
    );
}

/// The single phase trigger this card's text produces. Selecting by mode (with
/// a cardinality check) rather than by index keeps the assertion honest if the
/// parser ever emits an extra definition for the same line.
fn sole_phase_trigger<'a>(parsed: &'a ParsedAbilities, card: &str) -> &'a TriggerDefinition {
    let phase_triggers: Vec<&TriggerDefinition> = parsed
        .triggers
        .iter()
        .filter(|trigger| trigger.mode == TriggerMode::Phase)
        .collect();
    assert_eq!(
        phase_triggers.len(),
        1,
        "{card}: expected exactly one phase trigger, got {:?}",
        parsed
            .triggers
            .iter()
            .map(|t| (&t.mode, &t.condition))
            .collect::<Vec<_>>()
    );
    phase_triggers[0]
}

/// The trigger's execute-body root, with its presence pinned: a card whose
/// clause left the body unparsed must fail here rather than pass a condition
/// assertion over an empty body.
fn execute_body<'a>(trigger: &'a TriggerDefinition, card: &str) -> &'a AbilityDefinition {
    trigger
        .execute
        .as_deref()
        .unwrap_or_else(|| panic!("{card}: the trigger must carry an execute body"))
}

/// CR 107.1: `ObjectCount` over a permanent type phrase, counted on the
/// battlefield (the zone the bare noun leaves implicit).
fn battlefield_permanent_count(expr: &QuantityExpr) -> Option<&TargetFilter> {
    let QuantityExpr::Ref {
        qty: QuantityRef::ObjectCount { filter },
    } = expr
    else {
        return None;
    };
    Some(filter)
}

fn is_battlefield_zone(filter: &TargetFilter) -> bool {
    matches!(
        filter,
        TargetFilter::Typed(TypedFilter { properties, .. })
            if properties.contains(&FilterProp::InZone { zone: Zone::Battlefield })
    )
}

// ── Chaos Lord ────────────────────────────────────────────────────────────
// DISCRIMINATING. Pre-change no condition was represented at all: the whole
// line was swallowed with a `DynamicQty` report, and the quantity had no typed
// home. Revert the parity arm and both the `QuantityCheck` destructure and the
// `DynamicQty` assertion fire.
//
// **Where the condition lands, and why.** The printed "if" follows the effect
// instruction ("target opponent gains control of this creature if …"), so it is
// NOT a CR 603.4 intervening-if — that rule covers an `if` immediately after
// the trigger condition. It is a resolution-time gate, and it is lowered onto
// the instruction it gates (`AbilityCondition::QuantityCheck`), leaving the
// trigger's own `condition` empty. That split is what this test pins: hoisting
// it to the trigger would change WHEN the game asks the question.

const CHAOS_LORD: &str = "At the beginning of your upkeep, target opponent gains control of this creature if the number of permanents is even.";

#[test]
fn chaos_lord_parity_condition_is_typed_and_the_body_survives() {
    let parsed = parse_creature(CHAOS_LORD, "Chaos Lord");
    let trigger = sole_phase_trigger(&parsed, "Chaos Lord");
    assert_eq!(
        trigger.phase,
        Some(Phase::Upkeep),
        "the printed 'At the beginning of your upkeep' is the trigger event"
    );
    assert_eq!(
        trigger.condition, None,
        "a post-instruction 'if' is resolution-checked, not a CR 603.4 \
         intervening-if"
    );

    let body = execute_body(trigger, "Chaos Lord");
    let Some(AbilityCondition::QuantityCheck {
        lhs,
        comparator,
        rhs,
    }) = body.condition.as_ref()
    else {
        panic!(
            "expected the parity gate on the instruction it gates, got {:?}",
            body.condition
        );
    };
    // Even ⟺ the two half-roundings agree; odd is the NE sibling.
    assert_eq!(
        *comparator,
        Comparator::EQ,
        "'is even' is the equal-half-roundings leg"
    );

    // Both operands count the SAME population — all permanents (CR 110.1), which
    // the bare noun leaves on the battlefield. The count is the INNER expression
    // of the half-rounding leg, not the operand itself.
    let counts = |expr: &QuantityExpr, rounding: RoundingMode| -> bool {
        let QuantityExpr::DivideRounded {
            inner,
            divisor,
            rounding: actual,
        } = expr
        else {
            return false;
        };
        if *divisor != 2 || *actual != rounding {
            return false;
        }
        let Some(filter) = battlefield_permanent_count(inner.as_ref()) else {
            return false;
        };
        is_battlefield_zone(filter)
            && matches!(
                filter,
                TargetFilter::Typed(TypedFilter { type_filters, .. })
                    if type_filters.contains(&TypeFilter::Permanent)
            )
    };
    assert!(
        counts(lhs, RoundingMode::Down),
        "lhs must be ⌊count/2⌋ over all permanents, got {lhs:?}"
    );
    assert!(
        counts(rhs, RoundingMode::Up),
        "rhs must be ⌈count/2⌉ over all permanents, got {rhs:?}"
    );

    // Reach-guard: the clause was STRIPPED from the body, so the printed effect
    // parsed rather than being absorbed into an unimplemented remnant.
    assert!(
        matches!(body.effect.as_ref(), Effect::GiveControl { .. }),
        "the control-change effect must survive the hoisted condition, got {:?}",
        body.effect
    );

    assert_no_swallow(&parsed, "DynamicQty", "Chaos Lord");
    assert_no_swallow(&parsed, "Condition_If", "Chaos Lord");
}

// ── Zealots en-Dal ────────────────────────────────────────────────────────
// DISCRIMINATING. Pre-change `condition` is `None` with NO warning at all — the
// silent drop class. Revert the colour arm and the `condition` destructure
// fires.

const ZEALOTS_EN_DAL: &str = "At the beginning of your upkeep, if all nonland permanents you control are white, you gain 1 life.";

/// The `nonland permanents you control` population, with its three axes pinned
/// (permanent + nonland type filters, `you` controller) and the battlefield
/// zone the count reads.
fn is_your_nonland_permanents(filter: &TargetFilter) -> bool {
    let TargetFilter::Typed(TypedFilter {
        type_filters,
        controller,
        properties,
    }) = filter
    else {
        return false;
    };
    type_filters.contains(&TypeFilter::Permanent)
        && type_filters.contains(&TypeFilter::Non(Box::new(TypeFilter::Land)))
        && controller == &Some(ControllerRef::You)
        && properties.contains(&FilterProp::InZone {
            zone: Zone::Battlefield,
        })
}

#[test]
fn zealots_en_dal_colour_uniformity_is_a_two_quantity_comparison() {
    let parsed = parse_creature(ZEALOTS_EN_DAL, "Zealots en-Dal");
    let trigger = sole_phase_trigger(&parsed, "Zealots en-Dal");
    assert_eq!(trigger.phase, Some(Phase::Upkeep));

    let Some(TriggerCondition::QuantityComparison {
        lhs,
        comparator,
        rhs,
    }) = trigger.condition.as_ref()
    else {
        panic!(
            "expected the colour-uniformity QuantityComparison, got {:?}",
            trigger.condition
        );
    };
    assert_eq!(
        *comparator,
        Comparator::EQ,
        "∀x ∈ S. white(x) ⟺ |S| == |{{x ∈ S : white(x)}}|"
    );

    let Some(base) = battlefield_permanent_count(lhs) else {
        panic!("lhs must count the subject population, got {lhs:?}");
    };
    let Some(restricted) = battlefield_permanent_count(rhs) else {
        panic!("rhs must count the colour-restricted population, got {rhs:?}");
    };
    assert!(
        is_your_nonland_permanents(base),
        "lhs must be the printed 'nonland permanents you control', got {base:?}"
    );
    assert!(
        is_your_nonland_permanents(restricted),
        "rhs must be the same population, got {restricted:?}"
    );
    let TargetFilter::Typed(TypedFilter { properties, .. }) = restricted else {
        unreachable!("checked above");
    };
    assert!(
        properties.contains(&FilterProp::HasColor {
            color: ManaColor::White
        }),
        "rhs must be the white members of that population, got {restricted:?}"
    );
    // The restriction must be a NARROWING: the two operands differ only by the
    // colour property, so equality cannot be satisfied by miscounting either.
    assert_ne!(
        lhs, rhs,
        "the operands must not collapse to the same expression — that would make \
         the condition vacuously true"
    );

    // Reach-guard: the body is the printed life gain.
    assert!(
        matches!(
            execute_body(trigger, "Zealots en-Dal").effect.as_ref(),
            Effect::GainLife { .. }
        ),
        "the life gain must survive the hoisted condition, got {:?}",
        execute_body(trigger, "Zealots en-Dal").effect
    );

    assert_no_swallow(&parsed, "Condition_If", "Zealots en-Dal");
}

// ── Krovikan Vampire ──────────────────────────────────────────────────────
// DISCRIMINATING. Pre-change: `condition` is `None`, the line reports both a
// `Condition_If` and a `Duration_ThisTurn` swallow, and the reanimation's
// target is an unbound `ParentTarget`. Revert the phase-form arm (or the
// referent binding) and the corresponding assertion below fires.

const KROVIKAN_VAMPIRE: &str = "At the beginning of each end step, if a creature dealt damage by this creature this turn died, put that card onto the battlefield under your control. Sacrifice it when you lose control of this creature.";

#[test]
fn krovikan_vampire_damage_death_gate_binds_its_reanimation_referent() {
    let parsed = parse_creature(KROVIKAN_VAMPIRE, "Krovikan Vampire");
    let trigger = sole_phase_trigger(&parsed, "Krovikan Vampire");
    assert_eq!(
        trigger.phase,
        Some(Phase::End),
        "the printed 'At the beginning of each end step' is the trigger event"
    );

    // CR 603.4 + CR 700.4: the damage-history death predicate. This is the
    // phase-trigger reading — `game/triggers.rs` answers it from the turn's
    // damage + zone-change ledgers, because the trigger's own event (the end
    // step) carries no dying creature.
    assert_eq!(
        trigger.condition,
        Some(TriggerCondition::DealtDamageBySourceThisTurn),
        "the printed condition must be hoisted as the source-scoped damage-death gate"
    );

    // CR 400.7 + CR 608.2i: the body's "that card" is the creature this source
    // damaged this turn that died — a graveyard creature card carrying the
    // source-qualified damage history, NOT an unbound `ParentTarget`.
    let body = execute_body(trigger, "Krovikan Vampire");
    let Effect::ChangeZone {
        origin,
        destination,
        target,
        enters_under,
        ..
    } = body.effect.as_ref()
    else {
        panic!(
            "the reanimation must lower to ChangeZone, got {:?}",
            body.effect
        );
    };
    assert_eq!(
        *destination,
        Zone::Battlefield,
        "the reanimation's destination is the battlefield"
    );
    assert_eq!(
        *origin,
        Some(Zone::Graveyard),
        "the referent population is the graveyard, so the origin is pinned"
    );
    assert_ne!(
        target,
        &TargetFilter::ParentTarget,
        "an unbound ParentTarget can never resolve for a phase trigger; the \
         referent must name the died creature"
    );
    let TargetFilter::Typed(TypedFilter {
        type_filters,
        properties,
        ..
    }) = target
    else {
        panic!("the referent must be a typed graveyard-card filter, got {target:?}");
    };
    assert!(
        type_filters.contains(&TypeFilter::Creature),
        "the referent is a creature card, got {target:?}"
    );
    assert!(
        properties.contains(&FilterProp::InZone {
            zone: Zone::Graveyard
        }),
        "the referent is in a graveyard (it died), got {target:?}"
    );
    assert!(
        properties.contains(&FilterProp::WasDealtDamageBySourceThisTurn),
        "the referent must carry the SAME source-qualified damage history the \
         condition answers, got {target:?}"
    );
    assert_eq!(
        enters_under,
        &Some(ControllerRef::You),
        "the printed 'under your control' must survive, got {enters_under:?}"
    );

    assert_no_swallow(&parsed, "Condition_If", "Krovikan Vampire");
    assert_no_swallow(&parsed, "Duration_ThisTurn", "Krovikan Vampire");
}
