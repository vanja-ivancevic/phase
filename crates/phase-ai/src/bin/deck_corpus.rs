//! Faithful, bounded native name-deck corpus execution, not a format-policy audit.
//!
//! Every game is a historical Freeform mechanism replay. Input order and physical
//! copies survive unchanged; policy eligibility and supplied provenance remain
//! external evidence. Reports are JSONL, and only one action-cap-bounded ReplayLog
//! is retained at a time. No GameState is cloned for reporting.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use engine::ai_support::{apply_ai_action_proposal, AiDecisionContract, AiProposalApplication};
use engine::database::CardDatabase;
use engine::game::deck_loading::{
    load_and_hydrate_decks, resolve_deck_list, resolve_player_deck_list, DeckEntry, DeckList,
    PlayerDeckList, PlayerDeckPayload,
};
use engine::game::deck_validation::DeckCompatibilityRequest;
use engine::game::engine::{
    resolve_all_ready_access, resolve_all_ready_prefix, start_game, ResolveAllReadyAccess,
};
use engine::types::actions::GameAction;
use engine::types::format::{FormatConfig, GameFormat};
use engine::types::game_state::{GameState, StackEntry, StackEntryKind, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::match_config::MatchConfig;
use engine::types::player::{Player, PlayerId};
use engine::types::replay::{ReplayHeader, ReplayLog};
use engine::types::zones::Zone;
use phase_ai::config::{
    create_config_for_players, AiDifficulty, Platform, ACCEPTED_DIFFICULTY_LABELS,
};
use phase_ai::{choose_action_with_session, session::AiSession};
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{json, Value};

const HELP: &str = "deck-corpus <cards-directory> --requests FILE --output NEW_DIRECTORY
    --seed U64 [--seed U64 ...] --policy phase-ai|uniform-issued-candidate
    --schedule mirror|pair --action-cap U32 --turn-cap U32 --metadata FILE
    [--difficulty VeryEasy|Easy|Medium|Hard|VeryHard|CEDH]
    [--start-game U64] [--end-game U64]

The cards directory must contain card-data.json. Requests are the existing
DeckCompatibilityRequest[] JSON in manifest order (name arrays repeat copies).
Each input is hero once per schedule/seed, with two seat legs. Mirror uses the
same deck in both seats; pair uses the next input, wrapping at the end. Both legs
use the explicit seed and the engine's real starting-player contest. Repeated
seeds are rejected. All games use FormatConfig::freeform(), preserving historical
35/40-card decks; this is NOT Standard/Premodern/old-border policy admission.
The turn cap permits turns 1..=N and stops on entering turn N+1. The action cap
bounds submitted actions plus atomic Resolve All boundaries, not internal reducer
work. Uniform sampling is over finite, stably ordered engine-issued candidates,
excluding voluntary Concede; it is NOT uniform over all combinatorial actions.
PhaseAI uses its native measurement configuration (default difficulty Medium).

Game ranges use original global schedule indices, start inclusive and end exclusive.
The default is the entire schedule. All requests remain in their original order;
skipped legs are not replayed. A range does not certify the unexecuted legs.

--metadata supplies a JSON object containing the caller's source/card/deck SHA
bindings and other provenance. They are stored verbatim, not independently
verified here. Policy eligibility must be supplied/audited separately.

Output: run.json, inputs.jsonl, games.jsonl, checkpoints.jsonl, counts.jsonl,
summary.json and replays/game-N.json (engine ReplayLog v3). Exit 0 means every
selected leg completed; a partial range cannot mark the full schedule completed.
Input loss, stopped games, rejection and invariant failure exit 1; CLI/I/O errors
exit 2. Checkpoints are trusted, unredacted diagnostic projections, not full states
and not a cross-engine correctness oracle.";

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Policy {
    PhaseAi,
    UniformIssuedCandidate,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum Schedule {
    Mirror,
    Pair,
}

#[derive(Debug, Serialize)]
struct Options {
    cards: PathBuf,
    requests: PathBuf,
    output: PathBuf,
    seeds: Vec<u64>,
    policy: Policy,
    schedule: Schedule,
    action_cap: u32,
    turn_cap: u32,
    metadata: PathBuf,
    difficulty: AiDifficulty,
    start_game: u64,
    end_game: Option<u64>,
}

fn parse_options(mut args: impl Iterator<Item = OsString>) -> Result<Option<Options>, String> {
    let Some(first) = args.next() else {
        return Err(HELP.to_owned());
    };
    if first == "--help" || first == "-h" {
        return Ok(None);
    }
    let cards = PathBuf::from(first);
    let mut fields = BTreeMap::new();
    let mut seeds = Vec::new();
    for flag in std::iter::from_fn(|| args.next())
        .collect::<Vec<_>>()
        .chunks(2)
    {
        let key = flag[0].to_str().ok_or("non-UTF-8 option name")?;
        if key == "--help" || key == "-h" {
            return Ok(None);
        }
        if !matches!(
            key,
            "--requests"
                | "--output"
                | "--seed"
                | "--policy"
                | "--schedule"
                | "--action-cap"
                | "--turn-cap"
                | "--metadata"
                | "--difficulty"
                | "--start-game"
                | "--end-game"
        ) {
            return Err(format!("unknown option {key}"));
        }
        let value = flag
            .get(1)
            .ok_or_else(|| format!("missing value for {key}"))?;
        if key == "--seed" {
            let seed = value
                .to_str()
                .ok_or("non-UTF-8 seed")?
                .parse::<u64>()
                .map_err(|error| format!("invalid seed: {error}"))?;
            if seeds.contains(&seed) {
                return Err(format!("duplicate seed {seed}"));
            }
            seeds.push(seed);
        } else if fields.insert(key.to_owned(), value.clone()).is_some() {
            return Err(format!("duplicate option {key}"));
        }
    }
    let required = |key: &str| {
        fields
            .get(key)
            .ok_or_else(|| format!("required option {key}"))
    };
    let text = |key: &str| {
        required(key)?
            .to_str()
            .ok_or_else(|| format!("non-UTF-8 {key}"))
    };
    let positive = |key: &str| -> Result<u32, String> {
        let value = text(key)?
            .parse::<u32>()
            .map_err(|error| format!("invalid {key}: {error}"))?;
        if value == 0 {
            Err(format!("{key} must be positive"))
        } else {
            Ok(value)
        }
    };
    let index = |key: &str| -> Result<Option<u64>, String> {
        fields
            .get(key)
            .map(|value| {
                value
                    .to_str()
                    .ok_or_else(|| format!("non-UTF-8 {key}"))?
                    .parse::<u64>()
                    .map_err(|error| format!("invalid {key}: {error}"))
            })
            .transpose()
    };
    if seeds.is_empty() {
        return Err("at least one explicit --seed is required".to_owned());
    }
    let difficulty = match fields.get("--difficulty") {
        Some(value) => {
            let label = value.to_str().ok_or("non-UTF-8 difficulty")?;
            if !ACCEPTED_DIFFICULTY_LABELS
                .iter()
                .any(|accepted| accepted.eq_ignore_ascii_case(label.trim()))
            {
                return Err(format!("unknown difficulty {label}"));
            }
            AiDifficulty::from_label(label)
        }
        None => AiDifficulty::Medium,
    };
    Ok(Some(Options {
        cards,
        requests: PathBuf::from(required("--requests")?),
        output: PathBuf::from(required("--output")?),
        seeds,
        policy: match text("--policy")? {
            "phase-ai" => Policy::PhaseAi,
            "uniform-issued-candidate" => Policy::UniformIssuedCandidate,
            value => return Err(format!("unknown policy {value}")),
        },
        schedule: match text("--schedule")? {
            "mirror" => Schedule::Mirror,
            "pair" => Schedule::Pair,
            value => return Err(format!("unknown schedule {value}")),
        },
        action_cap: positive("--action-cap")?,
        turn_cap: positive("--turn-cap")?,
        metadata: PathBuf::from(required("--metadata")?),
        start_game: index("--start-game")?.unwrap_or(0),
        end_game: index("--end-game")?,
        difficulty,
    }))
}

fn selected_game_range(
    start: u64,
    end: Option<u64>,
    full_expected: u64,
) -> Result<std::ops::Range<u64>, String> {
    let end = end.unwrap_or(full_expected);
    if start >= end || end > full_expected {
        return Err(format!(
            "game range {start}..{end} must be nonempty and inside 0..{full_expected}"
        ));
    }
    Ok(start..end)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    serde_json::from_reader(BufReader::new(file))
        .map_err(|error| format!("read {}: {error}", path.display()))
}

fn new_writer(path: &Path) -> Result<BufWriter<File>, String> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create {}: {error}", path.display()))?;
    Ok(BufWriter::new(file))
}

fn json_line(writer: &mut impl Write, value: &impl Serialize) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, value)
        .map_err(|error| format!("serialize output: {error}"))?;
    writer
        .write_all(b"\n")
        .map_err(|error| format!("write output: {error}"))
}

fn flush(writer: &mut impl Write) -> Result<(), String> {
    writer
        .flush()
        .map_err(|error| format!("flush output: {error}"))
}

fn json_file(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut writer = new_writer(path)?;
    json_line(&mut writer, value)?;
    flush(&mut writer)
}

struct Input {
    deck: Option<PlayerDeckList>,
    errors: Vec<String>,
    main_copies: usize,
    sideboard_copies: usize,
}

// Inspect the raw DTO before serde can discard an unsupported/unknown zone.
// This remains a DeckCompatibilityRequest consumer, not another deck parser.
fn prepare_input(db: &CardDatabase, raw: &Value) -> Input {
    let mut errors = Vec::new();
    if let Some(object) = raw.as_object() {
        for key in object.keys() {
            if !matches!(
                key.as_str(),
                "main_deck"
                    | "sideboard"
                    | "commander"
                    | "companion"
                    | "planar_deck"
                    | "scheme_deck"
                    | "signature_spell"
                    | "selected_format"
                    | "selected_match_type"
                    | "player_count"
                    | "summary_only"
                    | "draft_set_codes"
                    | "draft_set_code"
            ) {
                errors.push(format!(
                    "unsupported request field {key}; refusing to discard it"
                ));
            }
        }
    }
    let request: DeckCompatibilityRequest = match DeckCompatibilityRequest::deserialize(raw) {
        Ok(request) => request,
        Err(error) => {
            return Input {
                deck: None,
                errors: vec![format!("invalid request: {error}")],
                main_copies: 0,
                sideboard_copies: 0,
            }
        }
    };
    let main_copies = request.main_deck.len();
    let sideboard_copies = request.sideboard.len();
    for (zone, names) in [
        ("commander", &request.commander),
        ("companion", &request.companion),
        ("planar_deck", &request.planar_deck),
        ("scheme_deck", &request.scheme_deck),
        ("signature_spell", &request.signature_spell),
    ] {
        if !names.is_empty() {
            errors.push(format!(
                "nonordinary zone {zone} has {} copies; requires separate format",
                names.len()
            ));
        }
    }
    if request.player_count != 2 {
        errors.push(format!("nonordinary player_count {}", request.player_count));
    }
    if let Some(selected) = &request.selected_format {
        if !matches!(
            selected.tag(),
            GameFormat::Standard
                | GameFormat::Limited
                | GameFormat::Pioneer
                | GameFormat::Modern
                | GameFormat::Premodern
                | GameFormat::Legacy
                | GameFormat::Vintage
                | GameFormat::Historic
                | GameFormat::Timeless
                | GameFormat::Pauper
                | GameFormat::Freeform
        ) {
            errors.push(format!(
                "nonordinary selected_format {:?}; requires separate format",
                selected.tag()
            ));
        }
    }
    if request
        .selected_match_type
        .is_some_and(|kind| kind != MatchConfig::default().match_type)
    {
        errors.push("nonordinary match type; this runner executes single duel games".to_owned());
    }
    let deck = PlayerDeckList {
        main_deck: request.main_deck,
        sideboard: request.sideboard,
        ..PlayerDeckList::default()
    };
    for (zone, names) in [
        ("main_deck", &deck.main_deck),
        ("sideboard", &deck.sideboard),
    ] {
        if names.len() > u32::MAX as usize {
            errors.push(format!("{zone} copy count exceeds resolver's u32 boundary"));
        }
        for (copy, name) in names.iter().enumerate() {
            // canonical_name rejects bogus/reversed composite suffixes which the
            // intentionally lossy get_face_by_name front-half lookup can accept.
            if db.canonical_name(name).is_none() || db.get_face_by_name(name).is_none() {
                errors.push(format!(
                    "{zone}[{copy}] unresolved or uncertain card name {name:?}"
                ));
            }
        }
    }
    if errors.is_empty() {
        let payload = resolve_player_deck_list(db, &deck);
        if let Err(error) = verify_resolved(db, &deck, &payload) {
            errors.push(error);
        }
    }
    Input {
        deck: errors.is_empty().then_some(deck),
        errors,
        main_copies,
        sideboard_copies,
    }
}

fn verify_zone(
    db: &CardDatabase,
    names: &[String],
    entries: &[DeckEntry],
    zone: &str,
) -> Result<(), String> {
    let mut expected = BTreeMap::<&str, u64>::new();
    for name in names {
        let face = db
            .get_face_by_name(name)
            .ok_or_else(|| format!("{zone}: unresolved {name:?}"))?;
        *expected.entry(&face.name).or_default() += 1;
    }
    let mut actual = BTreeMap::<&str, u64>::new();
    for entry in entries {
        *actual.entry(&entry.card.name).or_default() += u64::from(entry.count);
    }
    if expected != actual || actual.values().sum::<u64>() != names.len() as u64 {
        return Err(format!("{zone}: resolved physical-copy identities/counts differ: expected {expected:?}, actual {actual:?}"));
    }
    Ok(())
}

fn verify_resolved(
    db: &CardDatabase,
    names: &PlayerDeckList,
    payload: &PlayerDeckPayload,
) -> Result<(), String> {
    verify_zone(db, &names.main_deck, &payload.main_deck, "main_deck")?;
    verify_zone(db, &names.sideboard, &payload.sideboard, "sideboard")
}

fn verify_loaded(
    db: &CardDatabase,
    state: &GameState,
    player: PlayerId,
    names: &PlayerDeckList,
) -> Result<(), String> {
    let pool = state
        .deck_pools
        .iter()
        .find(|pool| pool.player == player)
        .ok_or_else(|| format!("missing loaded deck pool for {player:?}"))?;
    verify_zone(
        db,
        &names.main_deck,
        &pool.registered_main,
        "loaded registered main",
    )?;
    verify_zone(
        db,
        &names.main_deck,
        &pool.current_main,
        "loaded current main",
    )?;
    verify_zone(
        db,
        &names.sideboard,
        &pool.registered_sideboard,
        "loaded registered sideboard",
    )?;
    verify_zone(
        db,
        &names.sideboard,
        &pool.current_sideboard,
        "loaded current sideboard",
    )?;
    let loaded = state
        .players
        .iter()
        .find(|seat| seat.id == player)
        .ok_or_else(|| format!("missing loaded seat {player:?}"))?;
    if loaded.library.len() != names.main_deck.len() {
        return Err(format!(
            "{player:?}: initial physical library has {} of {} copies",
            loaded.library.len(),
            names.main_deck.len()
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    CompletedWin,
    CompletedDraw,
    TurnCap,
    ActionCap,
    NoEligibleActor,
    EmptyDomain,
    ConcedeOnly,
    PolicyNoAction,
    PolicyConcede,
    RejectedAction,
    StaleContract,
    InvariantFailure,
    InputRejected,
}

impl Outcome {
    fn completed(self) -> bool {
        matches!(self, Self::CompletedWin | Self::CompletedDraw)
    }

    fn label(self) -> &'static str {
        match self {
            Self::CompletedWin => "completed_win",
            Self::CompletedDraw => "completed_draw",
            Self::TurnCap => "turn_cap",
            Self::ActionCap => "action_cap",
            Self::NoEligibleActor => "no_eligible_actor",
            Self::EmptyDomain => "empty_domain",
            Self::ConcedeOnly => "concede_only",
            Self::PolicyNoAction => "policy_no_action",
            Self::PolicyConcede => "policy_concede",
            Self::RejectedAction => "rejected_action",
            Self::StaleContract => "stale_contract",
            Self::InvariantFailure => "invariant_failure",
            Self::InputRejected => "input_rejected",
        }
    }
}

fn terminal_or_cap(
    state: &GameState,
    steps: u32,
    action_cap: u32,
    turn_cap: u32,
) -> Option<Outcome> {
    match state.waiting_for {
        WaitingFor::GameOver { winner: Some(_) } => return Some(Outcome::CompletedWin),
        WaitingFor::GameOver { winner: None } => return Some(Outcome::CompletedDraw),
        _ => {}
    }
    if state.turn_number > turn_cap {
        Some(Outcome::TurnCap)
    } else if steps >= action_cap {
        Some(Outcome::ActionCap)
    } else {
        None
    }
}

fn candidate_domain(contract: &AiDecisionContract) -> Result<usize, Outcome> {
    if contract.candidates.is_empty() {
        return Err(Outcome::EmptyDomain);
    }
    let count = contract
        .candidates
        .iter()
        .filter(|candidate| !matches!(candidate.action, GameAction::Concede { .. }))
        .count();
    if count == 0 {
        Err(Outcome::ConcedeOnly)
    } else {
        Ok(count)
    }
}

#[derive(Default)]
struct Invariants {
    zone_members: HashSet<ObjectId>,
    stack_entries: HashSet<ObjectId>,
    attackers: HashSet<ObjectId>,
}

impl Invariants {
    fn member(
        &mut self,
        state: &GameState,
        id: ObjectId,
        zone: Zone,
        owner: Option<PlayerId>,
    ) -> Result<(), String> {
        let object = state
            .objects
            .get(&id)
            .ok_or_else(|| format!("{zone:?} references missing object {id:?}"))?;
        if object.zone != zone || owner.is_some_and(|owner| object.owner != owner) {
            return Err(format!(
                "{id:?} zone/owner disagrees with {zone:?}/{owner:?}"
            ));
        }
        if !self.zone_members.insert(id) {
            return Err(format!("duplicate physical zone membership {id:?}"));
        }
        Ok(())
    }

    fn check(&mut self, state: &GameState) -> Result<(), String> {
        state
            .resolution_stack
            .validate(&state.waiting_for)
            .map_err(|error| format!("resolution stack: {error}"))?;
        self.zone_members.clear();
        self.stack_entries.clear();
        self.attackers.clear();
        let valid_player = |id: PlayerId| state.players.iter().any(|player| player.id == id);
        if !valid_player(state.active_player) || !valid_player(state.priority_player) {
            return Err("active/priority player is absent".to_owned());
        }
        for player in &state.players {
            for (zone, ids) in [
                (Zone::Library, &player.library),
                (Zone::Hand, &player.hand),
                (Zone::Graveyard, &player.graveyard),
            ] {
                for &id in ids {
                    self.member(state, id, zone, Some(player.id))?;
                }
            }
        }
        for (zone, ids) in [
            (Zone::Battlefield, &state.battlefield),
            (Zone::Exile, &state.exile),
            (Zone::Command, &state.command_zone),
        ] {
            for &id in ids {
                self.member(state, id, zone, None)?;
            }
        }
        for (id, object) in &state.objects {
            if *id != object.id || !valid_player(object.owner) || !valid_player(object.controller) {
                return Err(format!("object key/owner/controller inconsistency {id:?}"));
            }
        }
        for entry in &state.stack {
            if !self.stack_entries.insert(entry.id) || !valid_player(entry.controller) {
                return Err(format!(
                    "stack entry identity/controller inconsistency {:?}",
                    entry.id
                ));
            }
            // Ability sources can leave/cease: CR 113.7a allows LKI. Only spells
            // have physical stack-zone membership. Announced pending spells
            // may retain their origin-zone field until finalization (CR 601.2a).
            if matches!(entry.kind, StackEntryKind::Spell { .. }) {
                let object = state
                    .objects
                    .get(&entry.source_id)
                    .ok_or_else(|| format!("spell source absent {:?}", entry.source_id))?;
                let pending = state
                    .waiting_for
                    .pending_cast_ref()
                    .or(state.pending_cast.as_deref());
                let announced = pending.is_some_and(|pending| {
                    pending.object_id == entry.source_id
                        && pending.activation_ability_index.is_none()
                        && object.zone == pending.origin_zone
                });
                if object.zone == Zone::Stack {
                    self.member(state, entry.source_id, Zone::Stack, None)?;
                } else if !announced {
                    return Err(format!(
                        "spell {:?} is neither stack-resident nor its pending origin",
                        entry.id
                    ));
                }
            }
        }
        if let Some(combat) = &state.combat {
            for attacker in &combat.attackers {
                if !self.attackers.insert(attacker.object_id)
                    || !valid_player(attacker.defending_player)
                    || !state
                        .objects
                        .get(&attacker.object_id)
                        .is_some_and(|object| object.zone == Zone::Battlefield)
                {
                    return Err(format!(
                        "invalid live combat attacker {:?}",
                        attacker.object_id
                    ));
                }
            }
            for (attacker, blockers) in &combat.blocker_assignments {
                if !self.attackers.contains(attacker) {
                    return Err(format!(
                        "block assignment names absent attacker {attacker:?}"
                    ));
                }
                for (index, blocker) in blockers.iter().enumerate() {
                    if blockers[..index].contains(blocker)
                        || !state
                            .objects
                            .get(blocker)
                            .is_some_and(|object| object.zone == Zone::Battlefield)
                        || !combat
                            .blocker_to_attacker
                            .get(blocker)
                            .is_some_and(|attackers| attackers.contains(attacker))
                    {
                        return Err(format!(
                            "invalid or nonreciprocal block {blocker:?}/{attacker:?}"
                        ));
                    }
                }
            }
            for (blocker, attackers) in &combat.blocker_to_attacker {
                for (index, attacker) in attackers.iter().enumerate() {
                    if attackers[..index].contains(attacker)
                        || !combat
                            .blocker_assignments
                            .get(attacker)
                            .is_some_and(|blockers| blockers.contains(blocker))
                    {
                        return Err(format!("invalid reverse block {blocker:?}/{attacker:?}"));
                    }
                }
            }
        }
        Ok(())
    }
}

// Read-only projections, not fake traces or state snapshots. Zone order and
// object IDs disambiguate repeated physical cards; battlefield values are the
// engine's current layer-derived consumer values.
struct CounterValues<'a, K>(&'a HashMap<K, u32>);

impl<K: Ord + Serialize> Serialize for CounterValues<'_, K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // The engine's deterministic map serializer is crate-private. Sort
        // borrowed keys here; never copy names or counter payloads.
        let mut entries: Vec<_> = self.0.iter().collect();
        entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
        let mut map = serializer.serialize_map(Some(entries.len()))?;
        for (key, count) in entries {
            map.serialize_entry(key, count)?;
        }
        map.end()
    }
}

struct ObjectCheckpoint<'a> {
    state: &'a GameState,
    id: ObjectId,
    permanent: bool,
}

impl Serialize for ObjectCheckpoint<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("id", &self.id)?;
        if let Some(object) = self.state.objects.get(&self.id) {
            map.serialize_entry("card_id", &object.card_id)?;
            map.serialize_entry("name", &object.name)?;
            map.serialize_entry("owner", &object.owner)?;
            if self.permanent {
                map.serialize_entry("controller", &object.controller)?;
                map.serialize_entry("tapped", &object.tapped)?;
                map.serialize_entry("damage", &object.damage_marked)?;
                map.serialize_entry("power", &object.power)?;
                map.serialize_entry("toughness", &object.toughness)?;
                map.serialize_entry("counters", &CounterValues(&object.counters))?;
                map.serialize_entry("types", &object.card_types)?;
                map.serialize_entry("phase_status", &object.phase_status)?;
                map.serialize_entry("summoning_sick", &object.summoning_sick)?;
            }
        } else {
            map.serialize_entry("missing", &true)?;
        }
        map.end()
    }
}

struct ZoneCheckpoints<'a, T: ?Sized> {
    state: &'a GameState,
    ids: &'a T,
    permanent: bool,
}

impl<'a, T: ?Sized> Serialize for ZoneCheckpoints<'a, T>
where
    &'a T: IntoIterator<Item = &'a ObjectId>,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(None)?;
        for &id in self.ids {
            sequence.serialize_element(&ObjectCheckpoint {
                state: self.state,
                id,
                permanent: self.permanent,
            })?;
        }
        sequence.end()
    }
}

struct PlayerCheckpoint<'a> {
    state: &'a GameState,
    player: &'a Player,
}

impl Serialize for PlayerCheckpoint<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let player = self.player;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("id", &player.id)?;
        map.serialize_entry("life", &player.life)?;
        map.serialize_entry("eliminated", &player.is_eliminated)?;
        map.serialize_entry("mana", &player.mana_pool)?;
        map.serialize_entry("poison", &player.poison_counters)?;
        map.serialize_entry("energy", &player.energy)?;
        map.serialize_entry("counters", &CounterValues(&player.player_counters))?;
        map.serialize_entry(
            "library",
            &ZoneCheckpoints {
                state: self.state,
                ids: &player.library,
                permanent: false,
            },
        )?;
        map.serialize_entry(
            "hand",
            &ZoneCheckpoints {
                state: self.state,
                ids: &player.hand,
                permanent: false,
            },
        )?;
        map.serialize_entry(
            "graveyard",
            &ZoneCheckpoints {
                state: self.state,
                ids: &player.graveyard,
                permanent: false,
            },
        )?;
        map.end()
    }
}

struct PlayerCheckpoints<'a>(&'a GameState);

impl Serialize for PlayerCheckpoints<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.0.players.len()))?;
        for player in &self.0.players {
            sequence.serialize_element(&PlayerCheckpoint {
                state: self.0,
                player,
            })?;
        }
        sequence.end()
    }
}

struct StackCheckpoint<'a>(&'a StackEntry);

impl Serialize for StackCheckpoint<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let entry = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("id", &entry.id)?;
        map.serialize_entry("source", &entry.source_id)?;
        map.serialize_entry("controller", &entry.controller)?;
        map.serialize_entry(
            "kind",
            match entry.kind {
                StackEntryKind::Spell { .. } => "spell",
                StackEntryKind::ActivatedAbility { .. } => "activated",
                StackEntryKind::TriggeredAbility { .. } => "triggered",
                StackEntryKind::KeywordAction { .. } => "keyword",
                StackEntryKind::CombatDamage { .. } => "combat_damage",
            },
        )?;
        map.serialize_entry("targets", &entry.ability().map(|ability| &ability.targets))?;
        map.end()
    }
}

struct StackCheckpoints<'a>(&'a GameState);

impl Serialize for StackCheckpoints<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.0.stack.len()))?;
        for entry in &self.0.stack {
            sequence.serialize_element(&StackCheckpoint(entry))?;
        }
        sequence.end()
    }
}

struct Checkpoint<'a> {
    game: u64,
    steps: u32,
    state: &'a GameState,
    reason: &'a str,
}

impl Serialize for Checkpoint<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let state = self.state;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("game", &self.game)?;
        map.serialize_entry("steps", &self.steps)?;
        map.serialize_entry("reason", self.reason)?;
        map.serialize_entry("turn", &state.turn_number)?;
        map.serialize_entry("active_player", &state.active_player)?;
        map.serialize_entry("priority_player", &state.priority_player)?;
        map.serialize_entry("phase", &state.phase)?;
        map.serialize_entry("prompt", state.waiting_for.variant_name())?;
        map.serialize_entry("semantic_owners", &state.waiting_for.acting_players())?;
        map.serialize_entry("state_revision", &state.state_revision)?;
        map.serialize_entry("players", &PlayerCheckpoints(state))?;
        map.serialize_entry(
            "battlefield",
            &ZoneCheckpoints {
                state,
                ids: &state.battlefield,
                permanent: true,
            },
        )?;
        map.serialize_entry("stack", &StackCheckpoints(state))?;
        map.serialize_entry(
            "exile",
            &ZoneCheckpoints {
                state,
                ids: &state.exile,
                permanent: false,
            },
        )?;
        map.serialize_entry(
            "command",
            &ZoneCheckpoints {
                state,
                ids: &state.command_zone,
                permanent: false,
            },
        )?;
        map.serialize_entry("combat", &state.combat)?;
        map.serialize_entry("resolution_depth", &state.resolution_stack.len())?;
        map.serialize_entry("rng_word_pos", &state.rng.get_word_pos())?;
        map.end()
    }
}

fn checkpoint(
    writer: &mut impl Write,
    game: u64,
    steps: u32,
    state: &GameState,
    reason: &str,
) -> Result<(), String> {
    json_line(
        writer,
        &Checkpoint {
            game,
            steps,
            state,
            reason,
        },
    )
}

#[derive(Serialize)]
struct GameReport {
    game: u64,
    hero_index: usize,
    opponent_index: usize,
    hero_seat: u8,
    seed: u64,
    outcome: Outcome,
    winner: Option<PlayerId>,
    turn: u32,
    steps: u32,
    accepted_actions: u32,
    resolve_all_boundaries: u32,
    replay: Option<String>,
    failure: Option<Value>,
}

fn play_game(
    db: &Arc<CardDatabase>,
    inputs: &[Input],
    options: &Options,
    report: &mut GameReport,
    checkpoints: &mut impl Write,
) -> Result<(), String> {
    let hero = &inputs[report.hero_index];
    let opponent = &inputs[report.opponent_index];
    let (Some(hero_deck), Some(opponent_deck)) = (&hero.deck, &opponent.deck) else {
        report.outcome = Outcome::InputRejected;
        report.failure =
            Some(json!({"hero_errors": hero.errors, "opponent_errors": opponent.errors}));
        return Ok(());
    };
    let (player, opponent) = if report.hero_seat == 0 {
        (hero_deck, opponent_deck)
    } else {
        (opponent_deck, hero_deck)
    };
    let decks = DeckList {
        player: player.clone(),
        opponent: opponent.clone(),
        ..DeckList::default()
    };
    let payload = resolve_deck_list(db, &decks);
    if let Err(error) = verify_resolved(db, &decks.player, &payload.player)
        .and_then(|()| verify_resolved(db, &decks.opponent, &payload.opponent))
    {
        report.outcome = Outcome::InputRejected;
        report.failure = Some(json!({"message": error}));
        return Ok(());
    }
    let mut replay = ReplayLog::new(ReplayHeader {
        format_config: FormatConfig::freeform(),
        match_config: MatchConfig::default(),
        player_count: 2,
        first_player: None,
        seed: report.seed,
        deck_data: Some(decks),
    });
    let mut state = GameState::new(replay.header.format_config.clone(), 2, report.seed);
    state.set_match_config(replay.header.match_config);
    // Match ReplayPlayer's canonical initialization, without granting any debug
    // action: Freeform does not enable allow_debug_actions.
    state.debug_mode = true;
    load_and_hydrate_decks(&mut state, &payload, Some(db));
    engine::game::install_card_db(&mut state, Arc::clone(db));
    let decks = replay
        .header
        .deck_data
        .as_ref()
        .expect("recorded input decks");
    if let Err(message) = verify_loaded(db, &state, PlayerId(0), &decks.player)
        .and_then(|()| verify_loaded(db, &state, PlayerId(1), &decks.opponent))
    {
        report.outcome = Outcome::InputRejected;
        report.failure = Some(json!({"message": message}));
        checkpoint(checkpoints, report.game, 0, &state, "input_loss")?;
        return Ok(());
    }
    start_game(&mut state);
    let config = create_config_for_players(options.difficulty, Platform::Native, 2)
        .into_measurement(report.seed);
    let session = AiSession::arc_from_game(&state);
    let mut rng = StdRng::seed_from_u64(report.seed);
    let mut invariants = Invariants::default();
    checkpoint(checkpoints, report.game, 0, &state, "started")?;
    let mut initial_failure = invariants.check(&state).err();
    loop {
        if let Some(message) = initial_failure.take() {
            report.outcome = Outcome::InvariantFailure;
            report.failure = Some(json!({"message": message, "after_step": report.steps}));
            break;
        }
        if let Some(outcome) =
            terminal_or_cap(&state, report.steps, options.action_cap, options.turn_cap)
        {
            report.outcome = outcome;
            break;
        }
        if matches!(state.waiting_for, WaitingFor::ResolveAllReady { .. }) {
            let requester = state.players.iter().map(|player| player.id).find(|player| {
                resolve_all_ready_access(&state, *player) == ResolveAllReadyAccess::Admitted
            });
            let Some(requester) = requester else {
                report.outcome = Outcome::NoEligibleActor;
                break;
            };
            // This is the engine-owned atomic consumer, not manufactured passes.
            // ReplayPlayer consumes precisely the same boundary. Its internal
            // reducer work is intentionally outside the submitted-action cap.
            resolve_all_ready_prefix(&mut state, requester);
            replay.push_resolve_all_boundary(requester);
            report.steps += 1;
            report.resolve_all_boundaries += 1;
            initial_failure = invariants.check(&state).err();
            checkpoint(
                checkpoints,
                report.game,
                report.steps,
                &state,
                "resolve_all_boundary",
            )?;
            continue;
        }
        let mut owners = state.waiting_for.acting_players();
        owners.sort_unstable();
        let Some(owner) = owners.into_iter().find(|owner| {
            state
                .players
                .iter()
                .any(|player| player.id == *owner && !player.is_eliminated)
        }) else {
            report.outcome = Outcome::NoEligibleActor;
            break;
        };
        let contract = AiDecisionContract::issue(&state, owner);
        let actor = contract.authorized_actor;
        if !state
            .players
            .iter()
            .any(|player| player.id == actor && !player.is_eliminated)
        {
            report.outcome = Outcome::NoEligibleActor;
            break;
        }
        let count = match candidate_domain(&contract) {
            Ok(count) => count,
            Err(outcome) => {
                report.outcome = outcome;
                break;
            }
        };
        let action = match options.policy {
            Policy::UniformIssuedCandidate => {
                let pick = rng.random_range(0..count);
                contract
                    .candidates
                    .iter()
                    .filter(|candidate| !matches!(candidate.action, GameAction::Concede { .. }))
                    .nth(pick)
                    .expect("counted finite issued domain")
                    .action
                    .clone()
            }
            Policy::PhaseAi => {
                match choose_action_with_session(&state, owner, &config, &mut rng, &session) {
                    Some(action) => action,
                    None => {
                        report.outcome = Outcome::PolicyNoAction;
                        break;
                    }
                }
            }
        };
        if matches!(action, GameAction::Concede { .. }) {
            report.outcome = Outcome::PolicyConcede;
            report.failure =
                Some(json!({"semantic_owner": owner, "authorized_actor": actor, "action": action}));
            break;
        }
        let turn = state.turn_number;
        let phase = state.phase;
        let prompt = state.waiting_for.variant_name();
        let was_pass = matches!(action, GameAction::PassPriority);
        // Clone only the consumed action: application takes ownership, and the
        // accepted-action replay needs that exact proposal after success.
        match apply_ai_action_proposal(&mut state, &contract, actor, action.clone()) {
            AiProposalApplication::AppliedStackPass { result } => {
                replay.push_verified_ai_priority_pass(actor, owner);
                report.accepted_actions += 1;
                report.steps += 1;
                initial_failure = invariants.check(&state).err();
                if initial_failure.is_some()
                    || !result.events.is_empty()
                    || turn != state.turn_number
                    || phase != state.phase
                    || prompt != state.waiting_for.variant_name()
                {
                    checkpoint(
                        checkpoints,
                        report.game,
                        report.steps,
                        &state,
                        "verified_ai_stack_pass",
                    )?;
                }
            }
            AiProposalApplication::AppliedAction { result } => {
                replay.push_action(actor, action);
                report.accepted_actions += 1;
                report.steps += 1;
                initial_failure = invariants.check(&state).err();
                if initial_failure.is_some()
                    || !was_pass
                    || !result.events.is_empty()
                    || turn != state.turn_number
                    || phase != state.phase
                    || prompt != state.waiting_for.variant_name()
                {
                    checkpoint(
                        checkpoints,
                        report.game,
                        report.steps,
                        &state,
                        "accepted_action",
                    )?;
                }
            }
            AiProposalApplication::Stale => {
                report.outcome = Outcome::StaleContract;
                report.failure = Some(
                    json!({"semantic_owner": owner, "authorized_actor": actor, "action": action,
                    "issued_revision": contract.state_revision, "current_revision": state.state_revision}),
                );
                break;
            }
            AiProposalApplication::Rejected { rejection } => {
                report.outcome = Outcome::RejectedAction;
                report.failure = Some(
                    json!({"semantic_owner": owner, "authorized_actor": actor, "action": action,
                    "rejection": rejection}),
                );
                break;
            }
        }
    }
    report.turn = state.turn_number;
    report.winner = match state.waiting_for {
        WaitingFor::GameOver { winner } => winner,
        _ => None,
    };
    checkpoint(
        checkpoints,
        report.game,
        report.steps,
        &state,
        report.outcome.label(),
    )?;
    let replay_path = format!("replays/game-{}.json", report.game);
    json_file(&options.output.join(&replay_path), &replay)?;
    report.replay = Some(replay_path);
    if !report.outcome.completed() && report.failure.is_none() {
        report.failure = Some(
            json!({"prompt": state.waiting_for.variant_name(), "after_step": report.steps,
            "turn": state.turn_number, "semantic_owners": state.waiting_for.acting_players()}),
        );
    }
    Ok(())
}

fn run(options: &Options) -> Result<bool, String> {
    let requests: Vec<Value> = read_json(&options.requests)?;
    if requests.is_empty() {
        return Err("requests corpus is empty".to_owned());
    }
    let metadata: Value = read_json(&options.metadata)?;
    if !metadata.is_object() {
        return Err("run metadata must be a JSON object".to_owned());
    }
    let expected = (requests.len() as u64)
        .checked_mul(options.seeds.len() as u64)
        .and_then(|count| count.checked_mul(2))
        .ok_or("schedule size overflow")?;
    let selected = selected_game_range(options.start_game, options.end_game, expected)?;
    let selected_expected = selected.end - selected.start;
    let db = Arc::new(
        CardDatabase::from_export(&options.cards.join("card-data.json"))
            .map_err(|error| format!("card database: {error}"))?,
    );
    fs::create_dir(&options.output).map_err(|error| {
        format!(
            "create new output directory {}: {error}",
            options.output.display()
        )
    })?;
    fs::create_dir(options.output.join("replays"))
        .map_err(|error| format!("create replay directory: {error}"))?;
    json_file(
        &options.output.join("run.json"),
        &json!({
            "schema_version": 1, "options": options, "input_count": requests.len(), "expected_games": expected,
            "selected_start_game": selected.start, "selected_end_game": selected.end,
            "selected_expected_games": selected_expected,
            "mechanism": "historical Freeform mechanism replay", "format_config": FormatConfig::freeform(),
            "policy_eligibility": "external/separate", "provenance": metadata,
            "provenance_verification": "caller-supplied; not verified by runner",
            "uniform_domain": "finite stable engine-issued candidates excluding Concede, not all combinatorial actions"
        }),
    )?;
    let mut input_writer = new_writer(&options.output.join("inputs.jsonl"))?;
    let mut inputs = Vec::with_capacity(requests.len());
    for (index, raw) in requests.into_iter().enumerate() {
        // Preserve the original DTO alongside the resolved-count verdict.
        let input = prepare_input(&db, &raw);
        json_line(
            &mut input_writer,
            &json!({"input_index": index, "request": raw,
            "main_copies": input.main_copies, "sideboard_copies": input.sideboard_copies,
            "faithfully_resolved": input.deck.is_some(), "errors": input.errors}),
        )?;
        inputs.push(input);
    }
    flush(&mut input_writer)?;
    let mut games = new_writer(&options.output.join("games.jsonl"))?;
    let mut checkpoints = new_writer(&options.output.join("checkpoints.jsonl"))?;
    let mut counts = new_writer(&options.output.join("counts.jsonl"))?;
    let mut totals = BTreeMap::<&str, u64>::new();
    let mut game = 0;
    let mut recorded = 0;
    let mut completed = 0;
    let mut first_failure = None;
    for &seed in &options.seeds {
        for hero in 0..inputs.len() {
            let opponent = match options.schedule {
                Schedule::Mirror => hero,
                Schedule::Pair => (hero + 1) % inputs.len(),
            };
            let selected_legs = (game..game + 2)
                .filter(|index| selected.contains(index))
                .count();
            if selected_legs == 0 {
                game += 2;
                continue;
            }
            let mut outcomes = BTreeMap::<&str, u64>::new();
            for hero_seat in 0..2 {
                if !selected.contains(&game) {
                    game += 1;
                    continue;
                }
                let mut report = GameReport {
                    game,
                    hero_index: hero,
                    opponent_index: opponent,
                    hero_seat,
                    seed,
                    outcome: Outcome::NoEligibleActor,
                    winner: None,
                    turn: 0,
                    steps: 0,
                    accepted_actions: 0,
                    resolve_all_boundaries: 0,
                    replay: None,
                    failure: None,
                };
                play_game(&db, &inputs, options, &mut report, &mut checkpoints)?;
                if report.outcome.completed() {
                    completed += 1;
                } else if first_failure.is_none() {
                    first_failure = Some(game);
                }
                *outcomes.entry(report.outcome.label()).or_default() += 1;
                *totals.entry(report.outcome.label()).or_default() += 1;
                json_line(&mut games, &report)?;
                flush(&mut games)?;
                flush(&mut checkpoints)?;
                game += 1;
                recorded += 1;
            }
            json_line(
                &mut counts,
                &json!({"input_index": hero, "seed": seed, "expected_games": selected_legs,
                "full_pair_games": 2, "recorded_games": outcomes.values().sum::<u64>(), "outcomes": outcomes}),
            )?;
            flush(&mut counts)?;
        }
    }
    let all_completed = recorded == selected_expected && completed == selected_expected;
    json_file(
        &options.output.join("summary.json"),
        &json!({
            "input_count": inputs.len(), "seed_count": options.seeds.len(), "expected_games": expected,
            "selected_start_game": selected.start, "selected_end_game": selected.end,
            "selected_expected_games": selected_expected,
            "recorded_games": recorded, "completed_games": completed, "stopped_or_failed_games": recorded - completed,
            "outcomes": totals, "all_selected_games_completed": all_completed,
            "all_scheduled_games_completed": all_completed && selected.start == 0 && selected.end == expected,
            "first_failure_game": first_failure,
            "policy_eligibility": "external/separate", "gameplay_correctness": "not established by completion alone"
        }),
    )?;
    Ok(all_completed)
}

fn main() -> ExitCode {
    match parse_options(std::env::args_os().skip(1)) {
        Ok(None) => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        Ok(Some(options)) => match run(&options) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::from(1),
            Err(error) => {
                eprintln!("deck-corpus: {error}");
                ExitCode::from(2)
            }
        },
        Err(error) => {
            eprintln!("deck-corpus: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> CardDatabase {
        CardDatabase::from_json_str(include_str!(
            "../../../engine/tests/fixtures/runtime_card_export_fixture.json"
        ))
        .unwrap()
    }

    #[test]
    fn continuation_range_keeps_the_first_unwritten_leg_and_excludes_its_end() {
        assert_eq!(selected_game_range(193, None, 1366).unwrap(), 193..1366);
        let range = selected_game_range(193, Some(195), 1366).unwrap();
        assert!(!range.contains(&192));
        assert!(range.contains(&193));
        assert!(range.contains(&194));
        assert!(!range.contains(&195));
        assert_eq!(selected_game_range(0, Some(1), 1366).unwrap(), 0..1);
    }

    #[test]
    fn continuation_refuses_empty_reversed_and_out_of_schedule_windows() {
        for (start, end) in [
            (1366, None),
            (0, Some(0)),
            (194, Some(193)),
            (193, Some(1367)),
            (u64::MAX, None),
        ] {
            assert!(selected_game_range(start, end, 1366).is_err());
        }
    }

    #[test]
    fn continuation_cli_refuses_signed_or_overflowing_game_indices() {
        for (flag, value) in [
            ("--start-game", "-1"),
            ("--end-game", "-1"),
            ("--start-game", "18446744073709551616"),
        ] {
            let args = [
                "cards",
                "--requests",
                "requests.json",
                "--output",
                "new",
                "--seed",
                "7",
                "--policy",
                "uniform-issued-candidate",
                "--schedule",
                "pair",
                "--action-cap",
                "10000",
                "--turn-cap",
                "100",
                "--metadata",
                "metadata.json",
                flag,
                value,
            ];
            assert!(parse_options(args.into_iter().map(OsString::from)).is_err());
        }
    }

    #[test]
    fn rejects_unknown_names_and_lossy_composite_suffixes_in_both_zones() {
        let db = db();
        for zone in ["main_deck", "sideboard"] {
            for name in [
                "Not a printed card",
                "Forest // Not a printed card",
                "Forest // Lightning Bolt",
            ] {
                let mut request = json!({"main_deck": ["Forest"]});
                request[zone] = json!([name]);
                let input = prepare_input(&db, &request);
                assert!(
                    input.deck.is_none(),
                    "uncertain {zone} copy must not launch"
                );
            }
        }
    }

    #[test]
    fn rejects_nonordinary_and_unrepresentable_zones_without_discarding_copies() {
        let db = db();
        for zone in [
            "commander",
            "companion",
            "planar_deck",
            "scheme_deck",
            "signature_spell",
            "attraction_deck",
            "contraption_deck",
            "unknown_zone",
        ] {
            let mut request = json!({"main_deck": vec!["Forest"; 35]});
            request[zone] = json!(["Forest"]);
            let input = prepare_input(&db, &request);
            assert!(
                input.deck.is_none(),
                "{zone} copies may not disappear into an ordinary duel"
            );
            assert_eq!(input.main_copies, 35);
        }
    }

    #[test]
    fn preserves_historical_sizes_alias_copies_and_sideboard_partition_through_loading() {
        let db = db();
        for size in [35, 40] {
            let names: Vec<_> = (0..size)
                .map(|copy| if copy % 2 == 0 { "Forest" } else { "forest" })
                .collect();
            let input = prepare_input(
                &db,
                &json!({"main_deck": names, "sideboard": vec!["Lightning Bolt"; 15]}),
            );
            let deck = input
                .deck
                .expect("known alias spellings preserve physical copies");
            let payload = resolve_player_deck_list(&db, &deck);
            assert_eq!(payload.main_deck.len(), 1);
            assert_eq!(payload.main_deck[0].count, size);
            assert_eq!(payload.sideboard[0].count, 15);
            let decks = DeckList {
                player: deck.clone(),
                opponent: deck,
                ..DeckList::default()
            };
            let payload = resolve_deck_list(&db, &decks);
            let mut state = GameState::new(FormatConfig::freeform(), 2, 9);
            load_and_hydrate_decks(&mut state, &payload, Some(&db));
            for player in &state.players {
                assert_eq!(player.library.len(), size as usize);
                assert!(player
                    .library
                    .iter()
                    .all(|id| state.objects[id].name == "Forest"));
            }
            assert_eq!(
                state.objects.len(),
                2 * size as usize,
                "sideboard must not become main-deck objects"
            );
            let pool = state
                .deck_pools
                .iter()
                .find(|pool| pool.player == PlayerId(0))
                .unwrap();
            assert_eq!(pool.registered_sideboard[0].count, 15);
        }
    }

    #[test]
    fn stopped_games_never_become_draws_and_real_completion_wins_cap_boundary() {
        let mut state = GameState::new(FormatConfig::freeform(), 2, 9);
        state.waiting_for = WaitingFor::Priority {
            player: PlayerId(0),
        };
        state.turn_number = 100;
        assert_eq!(
            terminal_or_cap(&state, 20, 20, 100),
            Some(Outcome::ActionCap)
        );
        assert_eq!(terminal_or_cap(&state, 19, 20, 100), None);
        state.turn_number = 101;
        assert_eq!(terminal_or_cap(&state, 19, 20, 100), Some(Outcome::TurnCap));
        state.waiting_for = WaitingFor::GameOver { winner: None };
        assert_eq!(
            terminal_or_cap(&state, 20, 20, 100),
            Some(Outcome::CompletedDraw)
        );
        state.waiting_for = WaitingFor::GameOver {
            winner: Some(PlayerId(1)),
        };
        assert_eq!(
            terminal_or_cap(&state, 20, 20, 100),
            Some(Outcome::CompletedWin)
        );
        for outcome in [
            Outcome::ActionCap,
            Outcome::TurnCap,
            Outcome::NoEligibleActor,
            Outcome::EmptyDomain,
            Outcome::ConcedeOnly,
            Outcome::PolicyNoAction,
            Outcome::PolicyConcede,
            Outcome::RejectedAction,
            Outcome::StaleContract,
            Outcome::InvariantFailure,
            Outcome::InputRejected,
        ] {
            assert!(!outcome.completed());
        }
    }
}
