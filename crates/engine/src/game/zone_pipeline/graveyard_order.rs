//! CR 404.3: owners arrange simultaneous graveyard arrivals before delivery.

use std::collections::VecDeque;

use crate::game::engine::EngineError;
use crate::types::events::GameEvent;
use crate::types::game_state::{
    BatchCompletion, GameState, GraveyardOrderMember, GraveyardOrderOwnerBatch,
    PendingBatchZoneChangeCause, PendingBatchZoneMoveRequest, PendingGraveyardOrderChoice,
    WaitingFor,
};
use crate::types::identifiers::{ObjectId, ObjectIncarnationRef};
use crate::types::player::PlayerId;
use crate::types::zones::Zone;

use super::ZoneMoveRequest;

#[cfg(test)]
mod tests;

fn candidate_owner(state: &GameState, id: ObjectId) -> Option<PlayerId> {
    state.objects.get(&id).and_then(|object| {
        (!object.is_token && !object.is_emblem && object.zone != Zone::Graveyard)
            .then_some(object.owner)
    })
}

/// The ordinary singleton/different-owner path allocates nothing. Only an
/// actual multi-card owner domain needs an ordering continuation.
pub(super) fn plan<I>(state: &GameState, ids: I) -> Option<VecDeque<GraveyardOrderOwnerBatch>>
where
    I: Iterator<Item = ObjectId> + Clone,
{
    let needs_order = ids.clone().enumerate().any(|(index, id)| {
        let Some(owner) = candidate_owner(state, id) else {
            return false;
        };
        ids.clone()
            .take(index)
            .any(|earlier| earlier != id && candidate_owner(state, earlier) == Some(owner))
    });
    if !needs_order {
        return None;
    }
    let batches = crate::game::players::apnap_order(state)
        .into_iter()
        .filter_map(|owner| {
            let members: Vec<_> = ids
                .clone()
                .filter(|id| candidate_owner(state, *id) == Some(owner))
                .collect();
            (members.len() > 1).then_some(GraveyardOrderOwnerBatch { owner, members })
        })
        .collect();
    Some(batches)
}

fn prompt(pending: &PendingGraveyardOrderChoice) -> WaitingFor {
    let source_id = pending
        .requests
        .iter()
        .find_map(|request| match &request.cause {
            PendingBatchZoneChangeCause::Effect { source }
            | PendingBatchZoneChangeCause::Destroy { source, .. }
            | PendingBatchZoneChangeCause::Cost { source } => Some(*source),
            _ => None,
        });
    WaitingFor::GraveyardOrderChoice {
        player: pending.current.owner,
        cards: pending.current.members.clone(),
        source_id,
    }
}

pub(super) fn pause(
    state: &mut GameState,
    mut batches: VecDeque<GraveyardOrderOwnerBatch>,
    requests: Vec<ZoneMoveRequest>,
    completion: Option<BatchCompletion>,
) {
    assert!(state.pending_graveyard_order_choice.is_none());
    let announced_members = requests
        .iter()
        .filter_map(|request| {
            state
                .objects
                .get(&request.object_id)
                .map(|object| GraveyardOrderMember {
                    identity: ObjectIncarnationRef::from_object(object),
                    origin: object.zone,
                    owner: object.owner,
                })
        })
        .collect();
    let pending = Box::new(PendingGraveyardOrderChoice {
        current: batches
            .pop_front()
            .expect("ordering plan has an owner domain"),
        remaining: batches,
        requests: requests
            .into_iter()
            .map(ZoneMoveRequest::into_pending)
            .collect(),
        announced_members,
        completion: completion.map(Box::new),
        resume_player: state.active_player,
    });
    state.waiting_for = prompt(&pending);
    state.pending_graveyard_order_choice = Some(pending);
}

/// A choice is a complete permutation of this owner's frozen incarnations,
/// never a subset, another owner's cards, or an arbitrary eligible object.
fn valid_selection(
    state: &GameState,
    pending: &PendingGraveyardOrderChoice,
    player: PlayerId,
    cards: &[ObjectId],
    chosen: &[ObjectId],
) -> bool {
    let batch = &pending.current;
    batch.owner == player
        && batch.members.len() > 1
        && cards.len() == batch.members.len()
        && chosen.len() == cards.len()
        && pending.announced_members.iter().all(|member| {
            state
                .objects
                .get(&member.identity.object_id)
                .is_some_and(|object| {
                    ObjectIncarnationRef::from_object(object) == member.identity
                        && object.zone == member.origin
                        && object.owner == member.owner
                })
        })
        && batch.members.iter().zip(cards).all(|(member, card)| {
            *member == *card
                && candidate_owner(state, *card) == Some(player)
                && pending
                    .requests
                    .iter()
                    .filter(|request| {
                        request.object_id == *card && request.destination == Zone::Graveyard
                    })
                    .count()
                    == 1
        })
        && chosen
            .iter()
            .enumerate()
            .all(|(index, card)| cards.contains(card) && !chosen[..index].contains(card))
}

/// Permute only this owner's announced requests. Existing graveyard cards,
/// other owners, non-graveyard deliveries and every request modifier stay intact.
fn apply_permutation(
    requests: &mut [PendingBatchZoneMoveRequest],
    batch: &GraveyardOrderOwnerBatch,
    chosen: &[ObjectId],
) {
    let mut next = 0;
    for slot in 0..requests.len() {
        if requests[slot].destination != Zone::Graveyard
            || !batch.members.contains(&requests[slot].object_id)
        {
            continue;
        }
        let desired = chosen[next];
        let current = requests
            .iter()
            .position(|request| {
                request.object_id == desired && request.destination == Zone::Graveyard
            })
            .expect("validated owner permutation has its exact request");
        requests.swap(slot, current);
        next += 1;
    }
}

pub(crate) fn submit(
    state: &mut GameState,
    player: PlayerId,
    cards: &[ObjectId],
    chosen: &[ObjectId],
    events: &mut Vec<GameEvent>,
) -> Result<WaitingFor, EngineError> {
    let pending = state
        .pending_graveyard_order_choice
        .as_deref()
        .ok_or_else(|| {
            EngineError::InvalidAction("graveyard order has no simultaneous instruction".into())
        })?;
    if !valid_selection(state, pending, player, cards, chosen) {
        return Err(EngineError::InvalidAction(
            "graveyard order must permute the owner's exact frozen cards".into(),
        ));
    }
    // Validation precedes taking the owner or changing any game state.
    let mut pending = state.pending_graveyard_order_choice.take().unwrap();
    apply_permutation(&mut pending.requests, &pending.current, chosen);
    if let Some(next) = pending.remaining.pop_front() {
        pending.current = next;
        state.waiting_for = prompt(&pending);
        state.pending_graveyard_order_choice = Some(pending);
        return Ok(state.waiting_for.clone());
    }
    Ok(finish_delivery(state, pending, events))
}

fn finish_delivery(
    state: &mut GameState,
    pending: Box<PendingGraveyardOrderChoice>,
    events: &mut Vec<GameEvent>,
) -> WaitingFor {
    let resume_player = if crate::game::players::is_alive(state, pending.resume_player) {
        pending.resume_player
    } else {
        state.active_player
    };
    state.waiting_for = WaitingFor::Priority {
        player: resume_player,
    };
    let result = super::move_owner_ordered_objects_simultaneously_then(
        state,
        pending
            .requests
            .into_iter()
            .map(ZoneMoveRequest::from_pending)
            .collect(),
        pending.completion.map(|completion| *completion),
        events,
    );
    if matches!(result, super::BatchMoveResult::NeedsChoice) {
        return state.waiting_for.clone();
    }
    crate::game::engine_resolution_choices::finish_with_continuation(state, resume_player, events)
}

/// CR 800.4a: a departed owner's cards cannot be delivered by the suspended
/// instruction. Prune only exact announced members after the leave sweep, then
/// continue surviving owner choices instead of replacing them with priority.
pub(crate) fn reconcile_after_elimination(state: &mut GameState, events: &mut Vec<GameEvent>) {
    if !matches!(state.waiting_for, WaitingFor::GraveyardOrderChoice { .. }) {
        return;
    }
    let Some(mut pending) = state.pending_graveyard_order_choice.take() else {
        return;
    };
    pending.announced_members.retain(|member| {
        crate::game::players::is_alive(state, member.owner)
            && state
                .objects
                .get(&member.identity.object_id)
                .is_some_and(|object| {
                    ObjectIncarnationRef::from_object(object) == member.identity
                        && object.zone == member.origin
                        && object.owner == member.owner
                })
    });
    pending.requests.retain(|request| {
        pending
            .announced_members
            .iter()
            .any(|member| member.identity.object_id == request.object_id)
    });
    if let Some(BatchCompletion::ZoneInstructionComplete {
        instruction_count: Some(count),
        ..
    }) = pending.completion.as_deref_mut()
    {
        *count = pending.requests.len() as i32;
    }
    let retained = |id: &ObjectId| {
        pending
            .announced_members
            .iter()
            .any(|member| member.identity.object_id == *id)
    };
    pending.current.members.retain(retained);
    for batch in &mut pending.remaining {
        batch.members.retain(retained);
    }
    pending.remaining.retain(|batch| batch.members.len() > 1);
    if pending.current.members.len() <= 1 {
        if let Some(next) = pending.remaining.pop_front() {
            pending.current = next;
        } else {
            finish_delivery(state, pending, events);
            return;
        }
    }
    state.waiting_for = prompt(&pending);
    state.pending_graveyard_order_choice = Some(pending);
}
