use bevy::platform::collections::HashSet;
use bevy::prelude::*;

use crate::components::*;
use crate::helpers::*;
use crate::history::*;

// ---------------------------------------------------------------------------
// TransitionMessage / PendingCount
// ---------------------------------------------------------------------------

/// A pending transition to be resolved by the schedule.
#[derive(Message, Debug, Clone)]
pub struct TransitionMessage {
    pub machine: Entity,
    pub source: Entity,
    pub target: Entity,
    /// The edge entity that triggered this transition (if known).
    /// Used for [`EdgeKind`] and [`ResetEdge`] checks.
    pub edge: Option<Entity>,
}

/// Tracks how many transition messages were queued during the current
/// schedule iteration. The outer loop checks this after each iteration to
/// decide whether to run another one. Reset at the top of
/// [`resolve_transitions`] and incremented by every system that writes a
/// [`TransitionMessage`] inside [`GearboxSchedule`](crate::GearboxSchedule)
/// (message listeners, [`check_always_edges`]).
#[derive(Resource, Default)]
pub struct PendingCount(pub usize);

// ---------------------------------------------------------------------------
// EnterState / ExitState entity events
// ---------------------------------------------------------------------------

/// Triggered on a state entity after the schedule converges.
/// Use `On<EnterState>` observers on state entities to react.
///
/// For schedule-phase or Update-phase systems, prefer querying
/// [`Added<Active>`](crate::components::Active) instead.
#[derive(EntityEvent, Clone, Debug)]
pub struct EnterState {
    #[event_target]
    pub state: Entity,
    pub machine: Entity,
}

/// Triggered on a state entity after the schedule converges.
/// Use `On<ExitState>` observers on state entities to react.
///
/// For schedule-phase or Update-phase systems, prefer querying
/// [`Active`](crate::components::Active) with `RemovedComponents`.
#[derive(EntityEvent, Clone, Debug)]
pub struct ExitState {
    #[event_target]
    pub state: Entity,
    pub machine: Entity,
}

/// Flush system: triggers [`EnterState`] / [`ExitState`] as entity events
/// from the [`Active`] component changes made during the schedule loop.
/// Runs in [`Update`] after the schedule loop.
pub(crate) fn flush_state_events(
    q_newly_active: Query<(Entity, &Active), Added<Active>>,
    mut removed: RemovedComponents<Active>,
    q_substate_of: Query<&SubstateOf>,
    q_machine: Query<(), With<StateMachine>>,
    mut commands: Commands,
) {
    // Fire ExitState for states that lost Active this frame
    for entity in removed.read() {
        // Walk up SubstateOf to find the machine root
        let machine = q_substate_of.root_ancestor(entity);
        // Only fire if the root is actually a state machine
        if q_machine.contains(machine) {
            commands.trigger(ExitState { state: entity, machine });
        }
    }
    // Fire EnterState for states that gained Active this frame
    for (state, active) in &q_newly_active {
        commands.trigger(EnterState { state, machine: active.machine });
    }
}

// ---------------------------------------------------------------------------
// Systems (run inside GearboxSchedule)
// ---------------------------------------------------------------------------

/// Resolve all pending transition messages: compute exits, entries, update
/// StateMachine, save history, handle ResetEdge, and insert/remove [`Active`].
///
/// Resets [`PendingCount`] at the top of the iteration. Any system that
/// writes a [`TransitionMessage`] later in the same iteration (message
/// listeners, always-edge checks, delay timers) increments the counter so
/// the outer loop knows whether to iterate again.
pub(crate) fn resolve_transitions(
    mut reader: MessageReader<TransitionMessage>,
    mut q_machine: Query<&mut StateMachine>,
    q_substates: Query<&Substates>,
    q_substate_of: Query<&SubstateOf>,
    q_initial: Query<&InitialState>,
    q_history: Query<&History>,
    mut q_history_state: Query<&mut HistoryState>,
    q_edge_kind: Query<&EdgeKind>,
    q_reset_edge: Query<&ResetEdge>,
    mut pending: ResMut<PendingCount>,
    mut commands: Commands,
) {
    // New iteration: clear the counter so later systems' writes reflect
    // only transitions queued during *this* iteration.
    pending.0 = 0;

    for msg in reader.read() {
        let Ok(mut machine) = q_machine.get_mut(msg.machine) else {
            info!("resolve_transitions: no machine component on {:?}", msg.machine);
            continue;
        };

        // --- Initialization (no active leaves yet) ---
        if machine.active_leaves.is_empty() {
            let leaves = get_all_leaf_states(
                msg.target,
                &q_initial,
                &q_substates,
                &q_history,
                &q_history_state,
            );
            machine.active_leaves.extend(&leaves);
            machine.active =
                compute_active_from_leaves(&machine.active_leaves, &q_substate_of);
            machine.active.insert(msg.machine);

            // Insert Active on all newly active states
            for &state in &machine.active {
                commands.entity(state).insert(Active { machine: msg.machine });
            }
            continue;
        }

        // --- Normal transition ---

        // Skip if the source is no longer active.
        if !machine.active.contains(&msg.source) {
            continue;
        }

        let exit_path = path_to_root(msg.source, &q_substate_of);
        let enter_path = path_to_root(msg.target, &q_substate_of);

        // LCA
        let mut lca_depth = exit_path
            .iter()
            .rev()
            .zip(enter_path.iter().rev())
            .take_while(|(a, b)| a == b)
            .count();

        // EdgeKind::Internal: don't exit/re-enter the LCA itself.
        // EdgeKind::External (default): if source IS the LCA, bump lca_depth
        // down so the source is exited and re-entered.
        let is_internal = msg
            .edge
            .and_then(|e| q_edge_kind.get(e).ok())
            .map(|k| matches!(k, EdgeKind::Internal))
            .unwrap_or(false);

        if !is_internal {
            let lca_entity = if lca_depth > 0 {
                Some(exit_path[exit_path.len() - lca_depth])
            } else {
                None
            };
            if lca_entity == Some(msg.source) {
                lca_depth = lca_depth.saturating_sub(1);
            }
        }

        // Collect the set of ancestors being exited.
        let exit_upto = exit_path.len() - lca_depth;
        let exited_ancestors: HashSet<Entity> =
            exit_path[..exit_upto].iter().copied().collect();

        // Collect exited leaves BEFORE modifying active_leaves (needed for history).
        let exited_leaves: Vec<Entity> = machine
            .active_leaves
            .iter()
            .copied()
            .filter(|leaf| {
                exited_ancestors.contains(leaf)
                    || q_substate_of
                        .iter_ancestors(*leaf)
                        .any(|a| exited_ancestors.contains(&a))
            })
            .collect();

        // Save history for any exited ancestor that has a History component.
        for &ancestor in &exited_ancestors {
            if let Ok(history) = q_history.get(ancestor) {
                let states_to_save = match history {
                    History::Shallow => {
                        let mut saved = HashSet::new();
                        for &leaf in &exited_leaves {
                            let mut prev = leaf;
                            for anc in q_substate_of.iter_ancestors(leaf) {
                                if anc == ancestor {
                                    saved.insert(prev);
                                    break;
                                }
                                prev = anc;
                            }
                        }
                        saved
                    }
                    History::Deep => {
                        exited_leaves
                            .iter()
                            .copied()
                            .filter(|leaf| {
                                *leaf == ancestor
                                    || q_substate_of
                                        .iter_ancestors(*leaf)
                                        .any(|a| a == ancestor)
                            })
                            .collect()
                    }
                };

                if let Ok(mut existing) = q_history_state.get_mut(ancestor) {
                    existing.0 = states_to_save;
                } else {
                    commands
                        .entity(ancestor)
                        .insert(HistoryState(states_to_save));
                }
            }
        }

        // Remove exited leaves.
        for &leaf in &exited_leaves {
            machine.active_leaves.remove(&leaf);
        }

        // Handle ResetEdge: clear history and active state under the reset scope.
        if let Some(edge) = msg.edge {
            if let Ok(reset) = q_reset_edge.get(edge) {
                let reset_roots: Vec<Entity> = match reset.0 {
                    ResetScope::Source => vec![msg.source],
                    ResetScope::Target => vec![msg.target],
                    ResetScope::Both => vec![msg.source, msg.target],
                };
                for &root in &reset_roots {
                    // Clear history under this subtree
                    let mut stack = vec![root];
                    while let Some(e) = stack.pop() {
                        if let Ok(mut hs) = q_history_state.get_mut(e) {
                            hs.0.clear();
                        }
                        if let Ok(children) = q_substates.get(e) {
                            stack.extend(children.into_iter().copied());
                        }
                    }
                    // Remove any remaining active leaves under this root
                    machine.active_leaves.retain(|leaf| {
                        *leaf != root
                            && !q_substate_of
                                .iter_ancestors(*leaf)
                                .any(|a| a == root)
                    });
                }
            }
        }

        // Enter: drill down to leaf from target.
        let new_leaves = get_all_leaf_states(
            msg.target,
            &q_initial,
            &q_substates,
            &q_history,
            &q_history_state,
        );
        machine.active_leaves.extend(new_leaves);

        // Recompute active set.
        let old_active = std::mem::take(&mut machine.active);
        machine.active =
            compute_active_from_leaves(&machine.active_leaves, &q_substate_of);
        machine.active.insert(msg.machine);

        // Build the full exited set: every state that was active before but
        // is no longer active. This catches intermediate states (e.g. a parent
        // with StateComponent) that sit between the exited leaves and the
        // transition source.
        let exited_all: Vec<Entity> = old_active
            .iter()
            .copied()
            .filter(|e| !machine.active.contains(e))
            .collect();

        // Remove Active from exited states
        for &state in &exited_all {
            commands.entity(state).remove::<Active>();
        }

        // Insert Active on newly entered states, or re-insert on states that
        // stayed active but are the target of a transition (triggers Changed<Active>
        // without triggering RemovedComponents<Active>).
        for &state in &machine.active {
            if !old_active.contains(&state) || exited_all.contains(&state) {
                // New or re-entered: insert (triggers Added<Active>)
                commands.entity(state).insert(Active { machine: msg.machine });
            } else if state == msg.target {
                // Target stayed active (e.g. child→parent): re-insert to
                // trigger Changed<Active> so systems can detect re-entry.
                commands.entity(state).insert(Active { machine: msg.machine });
            }
        }
    }
}

/// Resolve the target entity for an edge. If the edge has a [`BranchTransition`],
/// walk its arms in order and take the first whose guards pass; otherwise use
/// the fallback. If no branch is present, fall back to the plain [`Target`] component.
pub(crate) fn resolve_edge_target(
    edge: Entity,
    q_branch: &Query<&BranchTransition>,
    q_target: &Query<&Target>,
    q_guards: &Query<&Guards>,
) -> Option<Entity> {
    if let Ok(branch) = q_branch.get(edge) {
        for arm in &branch.arms {
            if let Ok(guards) = q_guards.get(arm.guard) {
                if !guards.is_empty() {
                    continue;
                }
            }
            return Some(arm.target);
        }
        return Some(branch.otherwise);
    }
    q_target.get(edge).ok().map(|t| t.0)
}

/// After transitions resolve, check if any AlwaysEdge is now eligible on
/// states that were just entered or re-entered (Changed<Active>).
///
/// Increments [`PendingCount`] for each transition it queues. The counter
/// is reset once per iteration by [`resolve_transitions`]; every system
/// that writes a [`TransitionMessage`] in the same iteration must
/// increment here to keep the outer loop driven correctly.
pub(crate) fn check_always_edges(
    mut writer: MessageWriter<TransitionMessage>,
    mut pending: ResMut<PendingCount>,
    q_active: Query<(Entity, &Active)>,
    q_transitions: Query<&Transitions>,
    q_always: Query<(), With<AlwaysEdge>>,
    q_target: Query<&Target>,
    q_branch: Query<&BranchTransition>,
    q_source: Query<&Source>,
    q_guards: Query<&Guards>,
    q_substate_of: Query<&SubstateOf>,
    q_delay: Query<(), With<Delay>>,
) {
    let mut states_to_check: Vec<(Entity, Entity)> = Vec::new(); // (state, machine)
    for (state, active) in &q_active {
        states_to_check.push((state, active.machine));
    }
    states_to_check.dedup();

    let mut fired_machines: HashSet<Entity> = HashSet::new();

    for (state, machine_entity) in states_to_check {
        if fired_machines.contains(&machine_entity) {
            continue;
        }
        let Ok(transitions) = q_transitions.get(state) else {
            continue;
        };
        for &edge in transitions {
            if q_always.get(edge).is_err() {
                continue;
            }
            // Skip delayed edges — the timer system handles them
            if q_delay.get(edge).is_ok() {
                continue;
            }
            // For non-branch edges, check guards on the edge itself
            if q_branch.get(edge).is_err() {
                if let Ok(guards) = q_guards.get(edge) {
                    if !guards.is_empty() {
                        continue;
                    }
                }
            }
            let Some(target) = resolve_edge_target(edge, &q_branch, &q_target, &q_guards) else {
                continue;
            };

            let source_state = q_source.get(edge).map(|s| s.0).unwrap_or(state);

            writer.write(TransitionMessage {
                machine: machine_entity,
                source: source_state,
                target,
                edge: Some(edge),
            });
            pending.0 += 1;
            fired_machines.insert(machine_entity);
            break;
        }
    }
}
