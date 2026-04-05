use bevy::prelude::*;

use crate::components::*;
use crate::resolve::{PendingCount, TransitionMessage};

/// Start timers for AlwaysEdge+Delay edges when their source state is entered.
/// Runs in [`GearboxPhase::EntryPhase`](crate::GearboxPhase::EntryPhase).
pub(crate) fn start_delay_timers(
    q_newly_active: Query<Entity, Added<Active>>,
    q_transitions: Query<&Transitions>,
    q_always: Query<(), With<AlwaysEdge>>,
    q_delay: Query<&Delay>,
    mut commands: Commands,
) {
    for state in &q_newly_active {
        let Ok(transitions) = q_transitions.get(state) else {
            continue;
        };
        for &edge in transitions {
            if q_always.get(edge).is_ok() {
                if let Ok(delay) = q_delay.get(edge) {
                    commands
                        .entity(edge)
                        .insert(EdgeTimer(Timer::new(delay.duration, TimerMode::Once)));
                }
            }
        }
    }
}

/// Cancel timers for edges whose source state was exited.
/// Runs in [`GearboxPhase::ExitPhase`](crate::GearboxPhase::ExitPhase).
pub(crate) fn cancel_delay_timers(
    mut removed: RemovedComponents<Active>,
    q_transitions: Query<&Transitions>,
    q_delay: Query<(), With<Delay>>,
    mut commands: Commands,
) {
    for state in removed.read() {
        let Ok(transitions) = q_transitions.get(state) else {
            continue;
        };
        for &edge in transitions {
            if q_delay.get(edge).is_ok() {
                commands.entity(edge).try_remove::<EdgeTimer>();
            }
        }
    }
}

/// Tick all active delay timers and write TransitionMessages when they fire.
/// Runs in [`Update`] before the per-frame [`GearboxSchedule`](crate::GearboxSchedule)
/// loop, so any transition a delay fires this frame is resolved — and any
/// cascade it triggers is fully processed — inside the same frame's loop.
pub(crate) fn tick_delay_timers(
    time: Res<Time>,
    q_transitions: Query<(Entity, &Transitions)>,
    mut q_timer: Query<&mut EdgeTimer>,
    q_delay: Query<&Delay>,
    q_always: Query<(), With<AlwaysEdge>>,
    q_guards: Query<&Guards>,
    q_target: Query<&Target>,
    q_substate_of: Query<&SubstateOf>,
    q_machine: Query<&StateMachine>,
    mut writer: MessageWriter<TransitionMessage>,
    mut pending: ResMut<PendingCount>,
) {
    for (source, transitions) in &q_transitions {
        let root = q_substate_of.root_ancestor(source);
        let Ok(machine) = q_machine.get(root) else {
            continue;
        };
        if !machine.is_active(&source) {
            continue;
        }
        for &edge in transitions {
            if q_delay.get(edge).is_err() || q_always.get(edge).is_err() {
                continue;
            }
            let Ok(mut timer) = q_timer.get_mut(edge) else {
                continue;
            };
            timer.0.tick(time.delta());
            if !timer.0.just_finished() {
                continue;
            }
            // Check guards
            if let Ok(guards) = q_guards.get(edge) {
                if !guards.is_empty() {
                    continue;
                }
            }
            let Ok(target) = q_target.get(edge) else {
                continue;
            };

            info!(
                "tick_delay_timers: timer fired on edge={:?} source={:?} target={:?} (machine={:?})",
                edge, source, target.0, root
            );
            writer.write(TransitionMessage {
                machine: root,
                source,
                target: target.0,
                edge: Some(edge),
            });
            pending.0 += 1;
            break; // one delayed transition per source per frame
        }
    }
}
