# Native deck compatibility

`cargo deck-check CARD-DATA.JSON REQUESTS.JSON` exposes the engine's existing
`DeckCompatibilityRequest` / `DeckCompatibilityResult` protocol. After building,
the `deck-check` executable takes the same two paths. Supply a matching generated
card-data export. Stdout is an ordered JSON result array; stderr carries input or
output failures. Exit 0 means evaluation succeeded, including incompatible decks;
exit 2 means the operation failed. An empty request array produces `[]`.

Regenerate the complete export with the matching parser after a parser or
serialization-schema change. An older export is not a compatibility format;
do not patch tagged enum fields by hand or replace it with a reduced card pool.

For example, save this as `requests.json`:

```json
[
  {
    "main_deck": ["Forest"],
    "commander": ["Lathril, Blade of the Elves"],
    "selected_format": "Commander",
    "player_count": 4,
    "summary_only": false
  }
]
```

This deliberately incomplete deck receives the engine's invalid-deck verdict,
not a CLI failure. Card copies are repeated names. Format values use the existing
DTO's case-sensitive spelling. All supported request fields and result semantics
remain owned by `crates/engine/src/game/deck_validation.rs`; the executable does
not add legality or coverage rules.

Names resolve against the export's actual face and layout metadata. A valid
multi-face spelling such as `Front // Back` or `Front / Back` identifies one
card. Resolving single-slash split shorthand from only an indexed front
requires that front's split-card metadata. Hidden storage aliases for the same Oracle ID,
face position and printed name remain searchable but do not add a third face
to a two-faced card's canonical name.

POD-Lab's `pod_lab.fields.validate` is the external consumer: it validates complete
candidate/opponent decks against the exact gameplay dataset before freezing a run.
The Cargo alias supplies an in-repository entry point. `set-check --deck` already
provides coverage for parser development; use it for per-deck coverage, AST hashes
and snapshot differences. This adapter exists for the additional format-legality
and zoned-deck request protocol, rather than introducing that protocol into the
parser-audit CLI. It does not claim that parser support certifies correct gameplay.

Reproducible checks live inline in `deck_check.rs`. They cover DTO/result parity,
ordered mixed requests, empty batches, malformed input, missing/non-ASCII paths,
and output failure. With Tilt unavailable, run the `deck-check` binary tests in
the `phase-engine` package with the `cli` feature. Follow normal Tilt instructions
when it is running; do not launch competing builds.

## Native deck corpus runs

`deck-check` answers whether a deck is admissible. The `deck-corpus` executable
in `phase-ai` plays bounded native games with name decks and records what
happened. It does not audit format policy:

```bash
cargo run --locked --profile tool -p phase-ai --bin deck-corpus -- \
  CARDS-DIRECTORY --requests REQUESTS.JSON --output NEW-DIRECTORY \
  --seed 1 [--seed 2 ...] --policy phase-ai|uniform-issued-candidate \
  --schedule mirror|pair --action-cap N --turn-cap N --metadata METADATA.JSON \
  [--difficulty VeryEasy|Easy|Medium|Hard|VeryHard|CEDH] \
  [--start-game U64] [--end-game U64]
```

- `CARDS-DIRECTORY` must contain a `card-data.json` export that matches the
  parser. `--requests` uses the `DeckCompatibilityRequest[]` JSON shown above,
  in manifest order. Physical copies stay repeated names.
- Every request is the hero once for each seed, with two seat legs. `mirror`
  puts the same deck in both seats. `pair` uses the next request and wraps at
  the end. Both legs use the explicit seed and the engine's own
  starting-player contest. A repeated seed is rejected.
- `--start-game` is inclusive and defaults to zero; `--end-game` is exclusive
  and defaults to the complete schedule size. Bounds must select a non-empty
  in-schedule range. Keep the complete requests in their original order: global
  game indices remain seed → hero → seat, not a renumbered request slice.
  Summaries retain `expected_games` for the full schedule and report the selected
  range separately. A completed partial range cannot set
  `all_scheduled_games_completed`; it sets only `all_selected_games_completed`.
- Every game uses `FormatConfig::freeform()`, so historical 35- and 40-card
  decks keep their size. Admission here does not mean a deck is legal in
  Standard, Premodern or an old-border format.
- `--turn-cap N` allows turns 1 through N. `--action-cap` counts submitted
  actions and atomic Resolve All boundaries, not internal reducer work.
  `uniform-issued-candidate` samples uniformly over the engine-issued
  candidates, in a stable order, except voluntary Concede. It is not uniform
  over every combinatorial action. `phase-ai` uses PhaseAI's native
  measurement configuration and defaults to `Medium`.
- `--metadata` is a JSON object holding the caller's source, card and deck SHA
  bindings plus any other provenance. It is stored as given and not checked.
- `--output` must be a new directory. It receives `run.json`, `inputs.jsonl`,
  `games.jsonl`, `checkpoints.jsonl`, `counts.jsonl`, `summary.json` and
  `replays/game-N.json` (engine `ReplayLog` v3). Only one replay is held in
  memory at a time. Checkpoints are trusted, unredacted diagnostic projections.
  They are not full states and not a cross-engine oracle.
- Exit 0 means every selected game completed, not that an omitted schedule
  prefix or suffix was exercised. Exit 1 covers lost input, a stopped game,
  a rejection or a failed invariant. Exit 2 covers CLI and I/O errors.
  `deck-corpus --help` prints the same contract.

### Evidence boundary (2026-10-04)

- Three real starter decks were run for six games, and every game finished
  with a winner.
- All 683 corrected physical duel decklists kept their exact card counts
  through admission. The matching 1366 games ran with `--action-cap 1` on
  purpose. That shows the decks are admitted and a game can start, not that a
  game can finish.
- The first retained randomized C0 run stopped at game 834 with an
  `OptionalEffect`/`GameOver` frame invariant failure. Its 834 written reports
  contain 669 wins, 3 draws, 161 rejected actions and 1 turn-cap stop. The
  rejections still require engine-versus-harness attribution; neither those
  reports nor the interrupted run establish the full 683-deck gameplay gate.
- Whole-deck gameplay correctness and a Forge differential comparison have
  not been demonstrated.

## Old-border mechanism batches

`crates/patina-old-border-smoke` is the compact, database-free scenario suite
for each mechanism batch. It complements card-data regeneration, full Phase
integration and Forge differential runs; it does not replace them. The C0 batch
(`battlefield_semantics`, `keyword_family_loss`, `revealed_color_quantity` and
`legacy_mechanisms`, 16 tests) passed on the retained Linux worker. The C1
batch's 12 linked-return and 6 face-only coin consumer regressions passed
focused retained-Linux stages. The combined workspace gate, fresh unfiltered
card export and native gameplay remain distinct checks.

**Source-linked battlefield returns (CR 607.2c, CR 400.7).** For example,
Diabolic Servitude: "the creature put onto the battlefield with this
enchantment".

- A link is recorded only when a source whose ability reads a linked consumer
  puts an object onto the battlefield. The link pairs the exact source
  incarnation with the exact recipient incarnation
  (`GameState.battlefield_return_links`).
- A linked phrase is an automatic context reference, not a player-chosen
  target (CR 115.1); it claims no interactive target slot.
- When a trigger is placed, the relation is frozen into incarnation-pinned
  targets.
- A death instruction acts only on the graveyard successor recorded for that
  death. It never acts on a later object in the same graveyard or at the same
  id.
- A source-departure instruction keeps the battlefield pin, so a recipient
  that has since been blinked is a new object and is left alone.
- When the source leaves the battlefield, its links are dropped, so a blinked
  source starts with no links.
- An empty or illegal ETB target records no link. It never falls back to the
  source, another graveyard card or a bystander.
- If source and recipient leave simultaneously, either controller-chosen
  trigger order preserves their old identities. Only the death instruction
  follows the immediate graveyard successor; unrelated objects are untouched.

**Face-only coin flips (CR 705.2).** For example: "Each player flips a coin.
Each player whose coin comes up tails sacrifices a creature of their choice."

- The flip is marked face-only (`Effect::FlipCoin.result_is_face`) and records
  `Heads` or `Tails` for each flipping player in a per-round ledger.
- No player wins or loses a face-only flip, so "whenever you win/lose a coin
  flip" does not trigger. An unfiltered "whenever you flip a coin" still does.
- Every APNAP flip, Krark's Thumb keep choice and CR 616.1 replacement
  ordering finishes before anyone chooses a sacrifice.
- Only a player whose own kept coin came up tails sacrifices, and only a
  creature they control.
- A prevented or replaced flip records no result. A later resolution cannot
  reuse an earlier result.
- Called win/lose flips behave as before and allocate nothing in the ledger.

The client displays `Heads` and `Tails` as the flipping player's actual face,
without announcing a win or loss. Only `Won` and `Lost` use the existing
win/lose announcements. Mixed-player results stay ordered in the overlay FIFO;
Escape advances to the next result or dismisses the last one.

**Player-loss timing during resolution (CR 704.4).** A player whose life reaches
zero or less still answers the resolving spell's choices. The shared SBA pause
guard runs before player-loss and object checks, including the terminal safety
net; ordinary SBAs resume once resolution completes. The real-Oracle Chain
Lightning regression and native smoke retain P1's optional payment at -2 life,
then decline it and retire the frame/carrier normally before one loss and one
terminal outcome. The four-player mechanism regression also preserves the
pending commander-zone choice until resolution completes.

**Announcement and activation payment contracts.** All 161 original seed-7 C0
issued-action rejections now reproduce against the exact retained C0 library
and export: 157 new native diagnostics and four reused immutable witnesses,
covering 39,272 accepted prefix actions with no other outcomes. Each state
comes from a fresh accepted replay prefix, not a restored diagnostic projection.
Equal reducer-error strings are classifications, not proof of a shared cause.
The four repaired mechanisms—Thunderclap's printed-cost decline, Spell Blast's
sparse X, Blighted Shaman's tap/self-sacrifice and Night Soil's exile count—
pass current parsed-Oracle native payment/resolution consumers, plus 6
announcement and 11 activation regressions. This is not a repaired outcome for
every historical replay or proof of whole-deck gameplay. Remaining native
casting and division witnesses include Culling the Weak, Wax/Wane,
Circular Logic and Fireball.

Fixed exile payments require the complete declared count. A cost saying
"from a single graveyard" retains a whole-selection zone-owner constraint
through ordinary activation, mana activation and a paused resolution payment.
Issued selections stay within one payable pile; mixed-pile payloads are
rejected before mutation. The grouping constraint does not widen an
activation's default payer-only scope. An explicitly graveyard-scoped filter
can still permit an opponent's qualifying pile.

The source-bound v15-r2 native probe refused a mixed-graveyard resolution
payment that the preceding library had committed, then completed the same-pile
payment. Its fresh unfiltered export contains 35,815 faces and validates; this
is input integrity evidence, not a functionality percentage. The v16 retry
passes the complete engine library (23,335 passed, zero failed) after migrating
the public escape-life consumer and deleting obsolete AST-copy snapshots.
The workspace then reaches integration tests (9,145 passed, 18 failed); those
failures are being classified and repaired without re-pinning incidental
source/AST assertions. Integration remains pending until the complete gate and
remaining gameplay checks pass.

**Casting and division continuation batch.** The source-bound v17-r3 native
observer uses a fresh compiler-artifact-selected test-support library, not a
retained library guessed by filename. Fireball's complete Oracle text pays
8 mana for X=5 with three targets, 6 mana for X=4 with two targets, 4 mana
for X=1 with three targets, and 1 mana for X=0 with no targets. Division is
determined at resolution over still-legal targets: the X=4 survivor receives
4 damage; X=1 with three surviving targets deals zero to each. The remainder
is lost. This follows the
[2017-11-17 WotC ruling](https://api.scryfall.com/cards/df45a43e-a5b7-4fd4-873b-7b3c021be198/rulings).
Controller-chosen divisions over nonempty target sets instead retain their
original shares and must cover every declared target exactly once, with
positive amounts and the announced total. A legal zero-target declaration
has no division prompt; the native Violent Eruption consumer pays 4 mana,
resolves without damage and moves the spell to the graveyard.

The same batch passes eight casting-authority consumer regressions for
required sacrifice and added mana, payable alternative faces, target-correct
Wax/Wane election, full-cost madness and Prohibit's inherited base target.
Fireball's issued target menu excludes unaffordable target-count surcharges
while retaining an affordable completion. Its rejected declaration preserves
the full serialized state and all floating mana. The integration run passed
10 Fireball-related tests and rejected an incidental assertion that an
announced object was absent from the stack; that assertion is deleted, not
re-pinned. The corrected suite and complete workspace gate remain pending.

Vohar's prior-discard condition now guards the compound body outside opponent
fan-out, after parser scope rewriting: the native land-discard consumer leaves
life20/20, and the instant-discard consumer gives life21/19. Yenna still owes
its scry choice before player-loss SBAs run. These mechanism results do not
establish repaired outcomes for all 161 historical replays, all 683 current
decks, or Forge differential equivalence.

**V18 combined gate boundary.** Formatting, workspace Clippy, fresh unfiltered
export and export validation exit 0. The export contains 35,815 faces, with
SHA-256 `0825b7af4606ee069b133a1d4c8294b3bd9785bb7ed6866a970deeb25a0aa059`.
This is serialization/admission evidence, not gameplay coverage. The workspace
runtime stops in Phase AI at 2,630 passed / 1 failed / 10 ignored, before the full
Phase Engine library and integration suites run. Compilation is not their
pass. The failed zero-Harvest fixture assumes an unpayable required
graveyard-exile cast is issued. Its migration preserves the actual policy:
zero-effect Harvest can enter the graveyard and enable that subsequent cost;
removing the held spell's cost removes that reason to retain Harvest.
Engine preflight is not weakened. The migrated focused fixture passes its
actual exile payment and draw resolution; the full workspace remains pending.

A new official-startup Forge capture runs physical repaired requests 79/80
(`standard/manticore.dck` against `standard/blackwizard_hard_liliana.dck`) at
seed 7 with Forge AI. Both fresh JVMs complete one game with 315 decisions and
25 action-outcome records. The JSONL bytes are identical, SHA-256
`99106d05e4a240cbde6e206e4dcda01da02fe34d588e838082667af58a57e638`.
This proves a reproducible real-deck reference workload, not Phase equivalence.
No captured state or choice contains Fireball. Forge's activator-unset warnings
for Mountain, Forest and Masticore remain in the retained logs and are not
attributed to Phase.

**Mandatory grouped-target authority.** A fresh v18 native Nullmage Advocate
consumer reproduces issuance with no eligible opponent graveyard cards,
despite three in its controller's graveyard. The simple singleton fast path
ignored the unsatisfied two-card prefix and accepted only the destroy tail.
Grouped abilities now use the canonical fallible target-slot builder; a
failed group cannot become an untargeted execute body either. Fresh v19 native
proof passes unavailable/payable cases in both seats: return two opponent
cards, destroy Sol Ring, and leave three controller-graveyard decoys untouched.
The mechanism regression checks zero, one and two eligible cards; the
announcement suite passes all seven consumers, including both-seat Nullmage.
Manual division passes four integration consumers, Fireball eleven and Vohar
four.

**V19-r2 to V20 complete gate.** V19-r2 reached the complete Engine library
but failed one Engine integration consumer: a folded graveyard blitz option
with three mana, three starting life and a required two-life cost. Ordinary
preflight priced the printed four-mana total before the payable three-mana
blitz alternative. The shared folded-route correction uses the existing
complete Blitz/Bestow cost authority; it does not waive required costs.
Fresh V20 native proof issues and pays three mana plus two life, leaving one
life and the spell on the stack; insufficient mana or life refuses issuance.
All 56 blitz integration consumers pass.

V20 formatting, Clippy and fresh unfiltered export/validation pass. The
complete workspace reaches all later suites and doc-tests: 74 result rows,
37,338 passed, zero failed and 58 ignored. Phase AI's library passes 2,631
(10 ignored), the Engine library 23,344 (8 ignored), and Engine integration
9,166 (4 ignored). The export contains 35,815 faces, SHA-256
`0825b7af4606ee069b133a1d4c8294b3bd9785bb7ed6866a970deeb25a0aa059`.
Source/export-bound gameplay across all 683 corrected decks was launched with
seed 7, pair scheduling (1,366 expected legs), uniform finite issued candidates
and independent invariants. Its build passed, but SSH exited 255 after the
worker stopped; no completed gameplay result was retrieved. Vast reports
instance 54078223 exited/intended stopped, zero credit and a negative balance.
The stop cause is not confirmed. Stopped-instance host transfer recovered
193 recorded winning legs and 135,003 accepted actions, with no recorded
failures; no final runtime returncode or summary exists in the recovered
artifacts. These are partial results, not completion of all 1,366 legs.
Recover remaining artifacts before any replay; do not restart the matrix
from zero. Gate, interruption and cold-recovery evidence are retained in
`c1-v20-complete-gate-evidence.json`, `c1-v20-corpus-provider-stop.json`
and `cost-safety-cold-corpus-recovery.json` under
`/Fast/Shared/artifacts/analyses/phase-provider-zen5-20261003/`.

**Bounded Fireball oracle cross-validation.** The maintained Forge puzzle
state loader and real game-thread stack resolution produce the same four
observed division results as Phase's native consumer: X=5 over three legal
targets gives one each; X=4 over one survivor of two targets gives four; X=1
over three targets gives zero; X=0 with no targets resolves without damage.
The fresh Forge observer exits zero and moves Fireball to the graveyard.
These are independently constructed legal high-toughness target fixtures,
not identical full-game snapshots. Forge's direct stack ingress does not
exercise casting or payment. The Phase native observation is bound to its
recorded V17-r3 engine; the current V20 Fireball integration consumers also
pass. Retained comparison:
`c1-v20-forge-fireball-resolution-cross-validation.json` in the same analysis
directory. Neither the four-case result nor same-seed reference-game logs
establish paired all-deck transition equivalence.

**Schema cutover.** `GameEvent::CoinFlipped` now carries
`result: Won|Lost|Heads|Tails` in place of `won: bool`. There is no
compatibility decoder. `TargetFilter` gains `LinkedBattlefieldReturn`;
`AbilityCost::Exile` and paused `EffectZoneChoice` payments retain
`same_zone_owner`. The unreleased C1 batch moves the full-game protocol to 104
and the P2P wire to 86; the lobby protocol does not change. Regenerate
`card-data.json` before using these mechanisms. An older export cannot recover
the discarded single-graveyard qualifier and is not a compatibility format.

**Bounded indexed continuation (2026-10-04).** The fresh native source snapshot
`112b1cbc9f426a93ce0c70de5a58d49feed78abc59e6ea87ef3c00e7434b3566`
verified all 5,214 source bindings on the 32-GB-quoted Ryzen 5 5600X worker
(31.26 GiB physical host memory). All three `continuation_` boundary regressions
passed. The actual compiler-artifact-selected `deck-corpus` binary built with
SHA-256 `af424f786e6164531743e2fe7da7f11df30eff7578026b9b6537e7e971b29ebc`.
Both operations used one Cargo worker; cold test/build wall times were
181.9 / 147.8 seconds. Native production source outside this runner matches
the reconstructed V20 snapshot; two obsolete test-only source files and old
snapshots are absent, and no native source files were added.

The complete 683 requests, 41,654 main copies and 420 sideboard copies stayed
unchanged. Actual `--start-game 193 --end-game 194` at seed 7 selects hero 96,
opponent 97, seat 1, preserving the first unwritten global leg. That leg did
not complete within the bounded job: the remote timeout returned 124 after
1,500 seconds including the two compilation stages. Its 525 complete
checkpoints end at step 524, turn 24, `TargetSelection`, state revision 526,
exactly matching the earlier interrupted run. The next record is a 10,335-byte
partial projection. Checkpoints use a buffered writer, so those trailing bytes
do not locate the current instruction or establish whether the stall is
reporting, candidate generation or reducer work.

The job recovered its evidence before the allocation was confirmed exited /
intended stopped. Subsequent bounded debugger starts were rejected twice by
the provider before the debugger controller or target ran; the reason is not
confirmed. Both retained allocations remain paused with paid execution
disabled. There is no completed new leg, final summary, native stack/profile
capture, whole-corpus completion or new Forge-equivalence claim. Retained proof
under the same analysis directory: `c1-v21-native-continuation-bounded-result-20261004.json`,
`c1-v20-v21-source-compatibility-20261004.json`,
`game193-native-timeout-checkpoint-proof.json` and
`game193-native-debug-rejected-start-20261004.json`.

**Native global193 repair (2026-10-05).** Two real debugger samples locate the
stall in `TargetCompletionWalk` / `PendingTargetCostVisitor`, not checkpoint
reporting. Memoized mana payment still enumerated permutations of optional
Fireball targets after a legal completion was already unaffordable. The shared
walker now distinguishes target-validation refusal from payment refusal. Only
the latter may prune adding-target branches when the existing cost-axis analysis
proves a fixed non-negative target surcharge and no target-dependent discount.
An already chosen X is fixed; battlefield and player-wide transient rebates
retain exhaustive search. There is no target cap, Fireball-name exception,
payment bypass or new price computation.

The real Fireball announcement consumer times out after 20 seconds on the
baseline. On freshly rebuilt fixed engine artifacts, all ten announcement
consumers pass in 0.23 seconds, including initially unaffordable prefixes made
payable by a later battlefield or transient target-dependent rebate. Preserved
archive mtimes initially caused Cargo to reuse the baseline executable; that
attempt is not fix evidence. Verification refreshes changed Rust input mtimes
without changing content and requires a non-fresh engine compiler artifact.

The original global193 leg then completes in 25.6 seconds: unchanged seed 7,
hero 96 / opponent 97 / seat 1, all 683 requests and the same unfiltered card
export. It finishes at turn 27 with 601 accepted actions, winner 1 and no
rejection or invariant failure. Native source bindings: 5,214. Actual corpus
binary SHA-256:
`955ea972b85bd3870085fa3cfdb46c0558b66bae6b153dc4dc6e6d5d7c0ebb1f`.
The qualified Ryzen 7 5800X worker has 31.88 GiB allocated memory and 46.97 GiB
physical host memory. The guarded job automatically stopped; paid execution
was disabled afterwards. All three retained worker disks remain intact.

The bounded summary certifies only `[193,194)`, not all 1,366 legs or Forge
equivalence. All 193 earlier completed-leg ReplayLog files have now been
recovered from the stopped original worker without starting compute; reapplying
them against the fixed engine remains pending. The current full workspace gate,
remaining corpus legs and paired Forge meaningful-state proof also remain
pending. New automated model dispatch/review remains under Root's explicit
hold; native CPU work is not held. Retained native proof:
`game193-target-cost-native-r3/game193-cost-fix-evidence-r3/proof.json`.
Cold replay recovery:
`c1-v20-cold-native-replay-recovery-20261005.json`, under the analysis directory
above.

### Review-repair checkpoint (2026-10-07)

The flip-until-lose safety budget counts attempts, including prevented flips,
not accumulated wins. Exhausting the budget cannot manufacture a losing flip,
award deferred win payoffs, or emit successful completion. The per-effect
executor returns an unresolved-loop diagnostic. The existing generic chain
driver ignores per-effect errors: the public native probe returns `Ok(())`
after 8 ms, with life still 20 and no flip or completion events. This proves
bounded execution and no fabricated payoff, not end-to-end fault reporting or
complete mandatory-loop rules coverage.

Restored escape parser and cumulative-upkeep library-exile regressions pass in
the engine library batch: 23,352 passed, eight ignored, zero failed. The three
focused public consumer suites add 27 passing tests. The exile-payment consumer
now exercises positive and zero any-number payments from one graveyard and
refuses mixed-owner payment without committing resources. No production exile
filter change was needed: the any-number sentinel is already converted to a
zero minimum before per-owner eligibility filtering.

The maintained Forge `ComputerUtil.playStack` path additionally verified six
Fireball payment/resolution fixtures: five accepted casts paid their complete
X-plus-target surcharge, including zero targets and a target leaving before
resolution. An unaffordable second target was refused with the original hand,
mana and empty stack intact. This extends the earlier direct-stack resolution
probe to real cost-gated casting; it is still bounded mechanism evidence, not
paired all-deck transition equivalence.

Retained source-bound records in the Patina analysis directory:
`review-repair-m4-record-20261007.json`,
`coin-loop-public-smoke-20261007.log`, and
`forge-fireball-payment-20261007.result.json`. The whole-corpus completion gate
and paired Forge meaningful-state proof remain separate acceptance criteria;
no new functionality percentage is inferred from these results.

The subsequent current-source M4 workspace gate passes fmt and clippy, then
all 74 suites: 37,345 passed, 65 ignored, zero failed. It includes the resumed
1,000-wins coin regression. One serialized cold gate takes 1,675.16 seconds;
invocation-only symbol stripping keeps the final target at 12 GiB and leaves
20 GiB free. The 2,822 Rust/config source bindings remain unchanged after
verification. Receipt: `full-stripped-m4-gate-record-20261007.json`.

### Ordered graveyard exile and complete alternative costs (C2, 2026-10-07)

A graveyard `AbilityCost::Exile` with `from_top: true` pays from the caster's
own graveyard, taking the newest matching cards rather than the newest physical
cards. Spinning Darkness retains its fixed count of three and Black filter;
interleaved nonmatching cards and the opponent's graveyard are not payment.
The public prompt fixes both minimum and maximum to three and refuses an older
matching set without committing resources. Omitted/false `from_top` keeps
ordinary exile semantics; the existing physical-top library behavior is unchanged.

Electing a nonmana alternative replaces the printed mana base with zero, not
the complete cost with zero. Canonical total-cost computation still applies
live increases, reductions and floors on every payment continuation. The
timing-required route shares this election and repricing: the public Primal
Prayers/Sphere probe previously paid one energy but left its one tax mana
untouched; after repair it pays both and resolves the creature. Initial cast
admission checks the actual prepared cast's alternative-total and ordinary
mana routes. An empty casting-method menu is not an affordability verdict:
ordinary casts deliberately need no casting-method election.

Prepared admission prioritizes the latched timing-granted alternative exactly
as the payment continuation does, rather than pricing an unrelated self-option.
The public Primal/Sphere fixture with a competing self-mana option previously
refused a payable energy-plus-tax cast; after repair the issued cast pays both
and resolves. This does not expand the existing multi-alternative choice menu.

Shoal-style pitch costs bind X from the pitched card's mana value using the
spell's printed X and elected-alternative context. The elected zero mana base
and a tax-inclusive payable total cannot serve as the printed-X predicate.
The preserved Nourishing Shoal regression and actual public reducer now cover
bare and Sphere-taxed payment: X is three before resolution, the pitched card
is exiled, the tax is paid when present, and life increases from 20 to 23.

Full-game protocol is 105 and P2P is 87; lobby 15 and draft 30 are unchanged.
Exact-match refusal prevents an old gameplay peer from interpreting ordered
costs as arbitrary exile. Cost labels distinguish the top matching cards in
eight locales; the formatter does not yet spell out the color filter.
The real optional-cost modal passed English desktop and German 390px mobile
dispatch/clipping checks. Real WebSocket handshake refused full104 before
ClientHello, accepted full105, and retained lobby15 compatibility with full104.
The actual guest setup/reconnect handlers refused P2P86/88 using an in-memory
session; this is not a PeerJS-network or initialized-WASM proof.

The source8 focused M4 batch passes 165 tests: two existing Shoal consumers
(including bare/taxed X binding), 14 public announcement contracts,
11 activation contracts and the existing 101 manabrew contracts, plus the
remaining old-border consumer contracts.

The complete source8 M4 workspace gate passes 37,349 tests, with zero failures
and 65 ignored across 74 suite results. Fmt/clippy passed before a 3600-second
transport interruption during test compilation; the unchanged-source test
continuation completed with exit 0. The failed source7 Shoal assertion was
preserved, not re-pinned. Runtime mechanism probes and this gate do not qualify
an entire deck for labeling.

Five actual Phase/Forge runtime fixtures agree on admission, ordered exile,
complete mana payment, damage, life and source zone: both seats resolve the
untaxed spell, one mana pays Sphere's tax, and insufficient own Black cards
or an unpaid tax refuse without mutation. This is bounded mechanism evidence,
not complete-deck or all-683 Forge equivalence.

The three saved selections in games 699–701 now issue, resume payment to the
stack, and resolve through real priority passes, gaining three life. Their
actual runtime export is `0825b7af…`, matching the retained C1 v20/v21 input
receipts; `run.json`'s unverified caller metadata still names the older `f184…`.
That legacy export loses Black, fixed cardinality and top order, so these
resumed variable-size selections prove payment continuation only. Regenerate
the complete canonical export before exercising corrected deck legality;
never patch just Spinning Darkness or infer a functionality percentage here.

The fresh unfiltered canonical export is
`febe761967ead1670372d6ca8960c55d97044ee6f1ffdb9ef59798edfb14dda5`.
The actual `oracle-gen` binary is
`47f266d344d23f54cee7fd5589d8e7a4f9fa7e05bd4ca40bc83c3c929663ebab`;
the production `deck-corpus` is
`ce0d0df66e91cd73a17de8563f3c0c640cfe49ad99b7871ee9a1a848dee2710b`.
The engine producer features are `cli` and `tracing-subscriber`, not
`test-support`. Its actual Spinning Darkness definition retains fixed count3,
Black filtering, graveyard zone and `from_top: true`.

All 339 canonical inputs (663,765,907 bytes) match their original SHA256/size
manifest before and after export. SMB made `CON.json` inaccessible and yielded
an uncertified 333-set attempt; the complete Mac-local POSIX snapshot reads all
334 sets. The full snapshot was transferred over SSH, not patched per card,
and removed under an idle guard after proof. Export runtime requires the
existing `RUST_MIN_STACK=67108864` setting; the launcher omission aborted
before output and is preserved as negative evidence. No legacy `0825`, `f184`
metadata, failed SMB output or mechanism verdict transfers to this new export.

The actual production consumer then ran fresh corrected-data global legs
699–701 (half-open range699–702), seed7, pair schedule and uniform-issued
candidates. All three reached terminal wins with 1,698 accepted actions and no
recorded failure. This is not the legacy replay, complete-corpus execution,
gameplay correctness from completion, or paired Forge deck qualification.
Receipt: `ordered-exile-corrected-corpus-smoke-record-20261007.json`.

Receipts in the Patina analysis directory:
`ordered-exile-m4-focused8-record-20261007.json`,
`ordered-exile-m4-full8-record-20261007.json`,
`ordered-exile-m4-workspace-resume8-record-20261007.json`,
`spinning-darkness-phase-observer-after8-20261007-record.json`,
`primal-prayers-timing-selector-phase-observer-before-20261007-record.json`,
`primal-prayers-timing-selector-phase-observer-after8-20261007-record.json`,
`nourishing-shoal-x-tax-phase-observer-after8-20261007-record.json`,
`corpus-selection-replay-after-20261007-record.json`,
`spinning-darkness-paired-source8-runtime-20261007.json`,
`ordered-exile-ui-proof/proof.json`,
`ordered-exile-canonical-local-export-after-stack64-record-20261007.json`,
`ordered-exile-canonical-local-inputs-before-after-20261007.json`, and
`ordered-exile-canonical-output-manifest-20261007.json`.
Complete workspace verification is recorded above; fresh complete-corpus
acceptance remains a separate gate.
All Vast instances have been destroyed; no paid worker is retained or restarted
for this M4-only gate. Collector qualification and launch remain independent:
the first genuinely qualified deck-hash-bound subset suffices, and these
mechanism fixtures are not a blanket admission.
