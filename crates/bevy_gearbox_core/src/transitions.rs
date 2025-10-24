use std::marker::PhantomData;
use std::time::Duration;

use bevy::prelude::*;
use bevy::platform::collections::HashSet;
use std::any::TypeId;

use crate::StateChildren;
use crate::{guards::Guards, EnterState, Transition, active::Active, StateChildOf, StateMachine, ExitState, Parallel};
use crate::state_component::Reset;

/// Outbound transitions from a source state. Order defines priority (first match wins).
#[derive(Component, Default, Debug, PartialEq, Eq, Reflect)]
#[relationship_target(relationship = Source, linked_spawn)]
#[reflect(Component, FromWorld, Default)]
pub struct Transitions(Vec<Entity>);

impl<'a> IntoIterator for &'a Transitions {
    type Item = <Self::IntoIter as Iterator>::Item;

    type IntoIter = std::slice::Iter<'a, Entity>;

    #[inline(always)]
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl Transitions {
    pub fn new() -> Self {
        Self(Vec::new())
    }
}

#[derive(Component, Clone, PartialEq, Eq, Debug, Reflect)]
#[relationship(relationship_target = Transitions)]
#[reflect(Component, PartialEq, Debug, FromWorld, Clone)]
pub struct Source(#[entities] pub Entity);

impl FromWorld for Source {
    #[inline(always)]
    fn from_world(_world: &mut World) -> Self {
        Source(Entity::PLACEHOLDER)
    }
}

/// Target for an edge transition.
#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
pub struct Target(#[entities] pub Entity);

/// Whether the transition should be treated as External (default) or Internal.
#[derive(Component, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Component, Default)]
pub enum EdgeKind { 
    #[default]
    External,
    Internal,
}

/// Marker for a transition that should fire on entering the source state (no event).
#[derive(Component, Reflect, Default, Debug)]
#[reflect(Component)]
#[require(EdgeKind)]
pub struct AlwaysEdge;

/// Delayed transition configuration: fire after `duration` has elapsed while the source is active.
#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
pub struct After {
    pub duration: Duration,
}

impl After {
    #[inline]
    pub fn new(duration: Duration) -> Self { Self { duration } }

    pub fn from_f32(duration: f32) -> Self { Self { duration: Duration::from_secs_f32(duration) } }
}

#[derive(Component)]
pub struct EdgeTimer(pub Timer);

/// Pending event stored on an edge awaiting its After timer
#[derive(Component)]
pub struct PendingEvent<E: EntityEvent + Clone> {
    pub event: E,
}

/// Marker event to represent absence of a payload
#[derive(EntityEvent, Reflect, Clone)]
#[reflect(Default)]
pub struct NoEvent(Entity);

impl Default for NoEvent {
    fn default() -> Self {
        Self(Entity::PLACEHOLDER)
    }
}

/// Derive macro for simple events that don't need phase-specific payloads
pub use bevy_gearbox_macros::SimpleTransition;

fn cleanup_edge_timer_and_pending<E: EntityEvent + Clone + 'static>(
    commands: &mut Commands,
    edge: Entity,
) {
    commands.entity(edge).remove::<EdgeTimer>().remove::<PendingEvent<E>>();
}

/// Trait implemented by events that can provide phase-specific payloads
/// for the transition lifecycle. All associated types default to NoEvent,
/// and all getters default to None.
pub trait TransitionEvent: EntityEvent + Clone {
    type ExitEvent: EntityEvent + Clone = NoEvent;
    type EffectEvent: EntityEvent + Clone = NoEvent;
    type EntryEvent: EntityEvent + Clone = NoEvent;

    fn to_exit_event(&self) -> Option<Self::ExitEvent> { None }
    fn to_effect_event(&self) -> Option<Self::EffectEvent> { None }
    fn to_entry_event(&self) -> Option<Self::EntryEvent> { None }
}

/// Marker trait implemented by macros for events that are auto-registered.
pub trait RegisteredTransitionEvent: 'static {}

/// Internal resource to dedupe per-event installation.
#[derive(Resource, Default)]
pub struct InstalledTransitions(pub HashSet<TypeId>);

/// Installer record collected via `inventory` for auto-registration of transition events.
pub struct TransitionInstaller {
    pub install: fn(&mut App),
}

inventory::collect!(TransitionInstaller);

pub fn register_transition<E>(app: &mut App)
where
    E: TransitionEvent + RegisteredTransitionEvent + Clone + 'static,
    for<'a> <E as Event>::Trigger<'a>: Default,
    for<'a> <<E as TransitionEvent>::ExitEvent as Event>::Trigger<'a>: Default,
    for<'a> <<E as TransitionEvent>::EffectEvent as Event>::Trigger<'a>: Default,
    for<'a> <<E as TransitionEvent>::EntryEvent as Event>::Trigger<'a>: Default,
{
    if !app.world().contains_resource::<InstalledTransitions>() {
        app.insert_resource(InstalledTransitions(HashSet::new()));
    }

    let mut installed = app.world_mut().resource_mut::<InstalledTransitions>();
    let already = !installed.0.insert(TypeId::of::<E>());
    drop(installed);
    if already { return; }

    app.add_observer(edge_event_listener::<E>)
        .add_observer(crate::transition_observer::<PhaseEvents<E::ExitEvent, E::EffectEvent, E::EntryEvent>>)
        .add_systems(Update, tick_after_event_timers::<E>)
        .add_observer(cancel_pending_event_on_exit::<E>)
        .add_observer(replay_deferred_event::<E>);
}


// Note: No blanket impl for TransitionEvent to avoid conflicting impls in downstream crates.

/// A typed phase payload holder built from a TransitionEvent
#[derive(Clone, Default)]
pub struct PhaseEvents<Exit: EntityEvent + Clone = NoEvent, Effect: EntityEvent + Clone = NoEvent, Entry: EntityEvent + Clone = NoEvent> {
    pub exit: Option<Exit>,
    pub effect: Option<Effect>,
    pub entry: Option<Entry>,
}

/// Phase callbacks the transition observer will invoke at microsteps
pub trait PhasePayload: Clone + Send + Sync + 'static {
    fn on_exit(&self, _commands: &mut Commands, _source: Entity, _children: &Query<&StateChildren>, _state_machine: &StateMachine) {}
    fn on_effect(&self, _commands: &mut Commands, _edge: Entity, _children: &Query<&StateChildren>, _state_machine: &StateMachine) {}
    fn on_entry(&self, _commands: &mut Commands, _target: Entity, _children: &Query<&StateChildren>, _state_machine: &StateMachine) {}
}

impl PhasePayload for () {}

impl<Exit, Effect, Entry> PhasePayload for PhaseEvents<Exit, Effect, Entry>
where
    Exit: EntityEvent + Clone,
    Effect: EntityEvent + Clone,
    Entry: EntityEvent + Clone,
    for<'a> <Exit as Event>::Trigger<'a>: Default,
    for<'a> <Effect as Event>::Trigger<'a>: Default,
    for<'a> <Entry as Event>::Trigger<'a>: Default,
{
    fn on_exit(&self, commands: &mut Commands, source: Entity, children: &Query<&StateChildren>, state_machine: &StateMachine) {
        if let Some(mut ev) = self.exit.clone() {
            // target the source
            *ev.event_target_mut() = source;
            commands.trigger(ev);
        }
        for child in children.iter_descendants(source) {
            if !state_machine.is_active(&child) { continue; }
            if let Some(mut ev) = self.exit.clone() {
                // target each active child (replacement for trigger_targets)
                *ev.event_target_mut() = child;
                commands.trigger(ev);
            }
        }
    }

    fn on_effect(&self, commands: &mut Commands, edge: Entity, children: &Query<&StateChildren>, state_machine: &StateMachine) {
        if let Some(mut ev) = self.effect.clone() {
            *ev.event_target_mut() = edge;
            commands.trigger(ev);
        }
        for child in children.iter_descendants(edge) {
            if !state_machine.is_active(&child) { continue; }
            if let Some(mut ev) = self.effect.clone() {
                *ev.event_target_mut() = child;
                commands.trigger(ev);
            }
        }
    }

    fn on_entry(&self, commands: &mut Commands, target: Entity, children: &Query<&StateChildren>, state_machine: &StateMachine) {
        if let Some(mut ev) = self.entry.clone() {
            *ev.event_target_mut() = target;
            commands.trigger(ev);
        }
        for child in children.iter_descendants(target) {
            if !state_machine.is_active(&child) { continue; }
            if let Some(mut ev) = self.entry.clone() {
                *ev.event_target_mut() = child;
                commands.trigger(ev);
            }
        }
    }
}

/// App extension to register transition event support


fn validate_edge_basic(
    edge: Entity,
    q_guards: &Query<&Guards>,
    q_target: &Query<&Target>,
) -> bool {
    // Check guards if present
    if let Ok(guards) = q_guards.get(edge) {
        if !guards.check() { return false; }
    }
    // Must have valid target
    q_target.get(edge).is_ok()
}

/// Generic edge firing logic for TransitionEvent
fn try_fire_first_matching_edge_generic<E: TransitionEvent + RegisteredTransitionEvent + Clone>(
    source: Entity,
    event: &E,
    q_transitions: &Query<&Transitions>,
    q_listener: &Query<&EventEdge<E>>, 
    q_edge_target: &Query<&Target>,
    q_guards: &Query<&Guards>,
    q_child_of: &Query<&StateChildOf>,
    q_defer: &mut Query<&mut DeferEvent<E>>,
    q_active: &Query<(), With<Active>>,
    q_after: &Query<&After>,
    q_timer: &mut Query<&mut EdgeTimer>,
    commands: &mut Commands,
) -> bool {
    // Check if this state should defer this event type
    if let Ok(mut defer_event) = q_defer.get_mut(source) {
        if q_active.get(source).is_ok() {
            defer_event.defer_event(event.clone());
            return false;
        }
    }

    let Ok(transitions) = q_transitions.get(source) else { return false; };

    for edge in transitions.into_iter().copied() {
        if q_listener.get(edge).is_err() { continue; }

        // Validate edge (guards and target) - skip if invalid
        if !validate_edge_basic(edge, q_guards, q_edge_target) { continue; }

        // If edge is delayed, schedule timer and store pending event
        if let Ok(after) = q_after.get(edge) {
            if let Ok(mut timer) = q_timer.get_mut(edge) {
                timer.0.set_duration(after.duration);
                timer.0.reset();
            } else {
                commands.entity(edge).insert(EdgeTimer(Timer::new(after.duration, TimerMode::Once)));
            }
            commands.entity(edge).insert(PendingEvent::<E> { event: event.clone() });
            return true;
        }

        let payload = PhaseEvents {
            exit: event.to_exit_event(),
            effect: event.to_effect_event(),
            entry: event.to_entry_event(),
        };
        let root = q_child_of.root_ancestor(source);
        commands.trigger(Transition { machine: root, source, edge, payload });
        return true;
    }
    false
}



/// Attach this to a transition entity to react to a specific event `E`.
#[derive(Reflect, Component)]
#[reflect(Component)]
#[require(EdgeKind)]
pub struct EventEdge<E: EntityEvent + RegisteredTransitionEvent> {
    #[reflect(ignore)]
    _marker: PhantomData<E>,
}

impl<E: EntityEvent + RegisteredTransitionEvent> Default for EventEdge<E> {
    fn default() -> Self {
        Self { _marker: PhantomData }
    }
}

/// A component that can be added to states to an event of a specific type.
/// Event of type `E` that arrive while this state is active will be stored
/// and replayed when the state is exited.
#[derive(Component)]
pub struct DeferEvent<E: EntityEvent + RegisteredTransitionEvent> {
    pub deferred: Option<E>,
}

impl<E: EntityEvent + RegisteredTransitionEvent> Default for DeferEvent<E> {
    fn default() -> Self {
        Self { deferred: None }
    }
}

impl<E: EntityEvent + RegisteredTransitionEvent> DeferEvent<E> {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn defer_event(&mut self, event: E) {
        self.deferred = Some(event);
    }
    
    pub fn take_deferred(&mut self) -> Option<E> {
        std::mem::take(&mut self.deferred)
    }
}

/// Marker to request reset of subtree(s) during TransitionActions phase
#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
pub struct ResetEdge(pub ResetScope);

#[derive(Reflect, Default, Clone, Copy)]
pub enum ResetScope { #[default] Source, Target, Both }

/// On EnterState(source), evaluate AlwaysEdge transitions listed in `Transitions(source)` in order.
/// Respects After components - transitions with After will be handled by the timer system instead.
pub fn always_edge_listener(
    enter_state: On<EnterState>,
    q_transitions: Query<&Transitions>,
    q_always: Query<(), With<AlwaysEdge>>,
    q_edge_target: Query<&Target>,
    q_guards: Query<&Guards>,
    q_after: Query<&After>,
    q_child_of: Query<&StateChildOf>,
    mut commands: Commands,
){
    let source = enter_state.target;
    let Ok(transitions) = q_transitions.get(source) else { return; };

    // Evaluate in order; fire the first allowed transition
    for edge in transitions.into_iter().copied() {
        if q_always.get(edge).is_err() { continue; }

        // Skip transitions with After component - let the timer system handle them
        if q_after.get(edge).is_ok() { continue; }

        // Validate edge (guards and target)
        if !validate_edge_basic(edge, &q_guards, &q_edge_target) { continue; }

        // Fire transition
        let root = q_child_of.root_ancestor(source);
        commands.trigger(Transition { machine: root, source, edge, payload: () });
        break;
    }
}

/// Helper function to find the parallel region root for a given state.
/// Returns the state itself if it's not under a parallel region.
fn find_parallel_region_root(
    state: Entity,
    q_child_of: &Query<&StateChildOf>,
    q_parallel: &Query<&Parallel>,
) -> Entity {
    // Walk up the hierarchy to find if we're under a parallel state
    let mut previous_ancestor = state;
    for ancestor in q_child_of.iter_ancestors(state) {
        if q_parallel.contains(ancestor) {
            return previous_ancestor;
        }
        previous_ancestor = ancestor;
    }

    // Not under a parallel state, return the state itself
    state
}

/// On event `E`, scan `Transitions` for a matching edge with `EventEdge<E>`, in priority order.
fn edge_event_listener<E: TransitionEvent + RegisteredTransitionEvent + Clone>(
    transition_event: On<E>,
    q_transitions: Query<&Transitions>,
    q_listener: Query<&EventEdge<E>>, 
    q_edge_target: Query<&Target>,
    q_guards: Query<&Guards>,
    q_child_of: Query<&StateChildOf>,
    q_sm: Query<&StateMachine>,
    mut q_defer: Query<&mut DeferEvent<E>>,
    q_active: Query<(), With<Active>>,
    q_parallel: Query<&Parallel>,
    q_after: Query<&After>,
    mut q_timer: Query<&mut EdgeTimer>,
    mut commands: Commands,
) {
    let event = transition_event.event();
    let machine_root = transition_event.event().event_target();
    
    // If the event target is a machine root, try leaves/branches first (statechart-like), then fall back to root
    if let Ok(current) = q_sm.get(machine_root) {
        let mut visited: HashSet<Entity> = HashSet::new();
        let mut fired_regions: HashSet<Entity> = HashSet::new();

        // Leaves-first: attempt to fire along each active branch (one per parallel region)
        for &leaf in current.active_leaves.iter() {
            let region_root = find_parallel_region_root(leaf, &q_child_of, &q_parallel);
            if fired_regions.contains(&region_root) { continue; }

            if try_fire_first_matching_edge_on_branch(
                leaf, event, machine_root,
                &q_transitions, &q_listener, &q_edge_target, &q_guards,
                &q_child_of, &mut q_defer, &q_active, &q_after,
                &mut q_timer, &mut visited, &mut commands,
            ) {
                fired_regions.insert(region_root);
            }
        }

        // If no branch consumed the event, fall back to root-level transitions
        if fired_regions.is_empty() {
            let _ = try_fire_first_matching_edge(
                machine_root, event, &q_transitions, &q_listener, &q_edge_target,
                &q_guards, &q_child_of, &mut q_defer, &q_active,
                &q_after, &mut q_timer, &mut commands,
            );
        }
        return;
    }

    // Otherwise, evaluate on the targeted state directly
    try_fire_first_matching_edge(
        machine_root, event, &q_transitions, &q_listener, &q_edge_target,
        &q_guards, &q_child_of, &mut q_defer, &q_active, 
        &q_after, &mut q_timer, &mut commands,
    );
}

fn try_fire_first_matching_edge<E: TransitionEvent + RegisteredTransitionEvent + Clone>(
    source: Entity,
    event: &E,
    q_transitions: &Query<&Transitions>,
    q_listener: &Query<&EventEdge<E>>, 
    q_edge_target: &Query<&Target>,
    q_guards: &Query<&Guards>,
    q_child_of: &Query<&StateChildOf>,
    q_defer: &mut Query<&mut DeferEvent<E>>,
    q_active: &Query<(), With<Active>>,
    q_after: &Query<&After>,
    q_timer: &mut Query<&mut EdgeTimer>,
    commands: &mut Commands,
) -> bool {
    try_fire_first_matching_edge_generic(
        source, event, q_transitions, q_listener, q_edge_target,
        q_guards, q_child_of, q_defer, q_active, q_after,
        q_timer, commands,
    )
}

fn try_fire_first_matching_edge_on_branch<E: EntityEvent + Clone + TransitionEvent + RegisteredTransitionEvent>(
    start: Entity,
    event: &E,
    machine_root: Entity,
    q_transitions: &Query<&Transitions>,
    q_listener: &Query<&EventEdge<E>>, 
    q_edge_target: &Query<&Target>,
    q_guards: &Query<&Guards>,
    q_child_of: &Query<&StateChildOf>,
    q_defer: &mut Query<&mut DeferEvent<E>>,
    q_active: &Query<(), With<Active>>,
    q_after: &Query<&After>,
    q_timer: &mut Query<&mut EdgeTimer>,
    visited: &mut HashSet<Entity>,
    commands: &mut Commands,
) -> bool {
    // Walk from leaf up to (but not beyond) the machine root
    let mut current = Some(start);
    while let Some(state) = current {
        // Skip states already checked across other branches
        if !visited.insert(state) {
            if state == machine_root { break; }
            current = q_child_of.get(state).ok().map(|rel| rel.0);
            continue;
        }
        if try_fire_first_matching_edge(
            state,
            event,
            q_transitions,
            q_listener,
            q_edge_target,
            q_guards,
            q_child_of,
            q_defer,
            q_active,
            q_after,
            q_timer,
            commands,
        ) {
            return true;
        }
        if state == machine_root { break; }
        current = q_child_of.get(state).ok().map(|rel| rel.0);
    }
    false
}


/// When guards on an Always edge change while its source state is active, re-check and fire if now allowed.
pub fn check_always_on_guards_changed(
    q_guards_changed: Query<(Entity, &Guards, &Source, Has<Target>), (Changed<Guards>, With<AlwaysEdge>)>, 
    q_transitions: Query<&Transitions>,
    q_child_of: Query<&StateChildOf>,
    q_active: Query<(), With<Active>>,
    q_after: Query<&After>,
    mut commands: Commands,
) {
    for (edge, guards, source, edge_target) in q_guards_changed.iter() {

        let source = source.0;

        if !q_active.contains(source) { continue; }

        // Only consider Always edges whose guard set changed to passing
        if !guards.check() { continue; }

        // Ensure this edge is actually listed on the source's transitions (priority set)
        let Ok(transitions) = q_transitions.get(source) else { continue; };
        if !transitions.into_iter().any(|&e| e == edge) { continue; }

        // Ensure edge has a valid target; then fire (or arm timer if delayed)
        if !edge_target { continue; }
        let root = q_child_of.root_ancestor(source);
        if q_after.get(edge).is_ok() {
            let after = q_after.get(edge).unwrap();
            commands.entity(edge).insert(EdgeTimer(Timer::new(after.duration, TimerMode::Once)));
        } else {
            commands.trigger(Transition { machine: root, source, edge, payload: () });
        }
    }
}

/// On EnterState(source), start timers for any After edges.
pub fn start_after_on_enter(
    enter_state: On<EnterState>,
    q_transitions: Query<&Transitions>,
    q_after: Query<&After>,
    q_always: Query<(), With<AlwaysEdge>>,
    mut commands: Commands,
) {
    let source = enter_state.target;
    let Ok(transitions) = q_transitions.get(source) else { return; };
    for edge in transitions.into_iter().copied() {
        if q_after.get(edge).is_ok() && q_always.get(edge).is_ok() {
            let after = q_after.get(edge).unwrap();
            commands.entity(edge).insert(EdgeTimer(Timer::new(after.duration, TimerMode::Once)));
        }
    }
}

/// On ExitState(source), cancel timers for any After edges.
pub fn cancel_after_on_exit(
    exit_state: On<crate::ExitState>,
    q_transitions: Query<&Transitions>,
    q_after: Query<&After>,
    mut commands: Commands,
) {
    let source = exit_state.target;
    let Ok(transitions) = q_transitions.get(source) else { return; };
    for edge in transitions.into_iter().copied() {
        if q_after.get(edge).is_ok() {
            commands.entity(edge).remove::<EdgeTimer>();
        }
    }
}

/// During TransitionActions, if an edge has ResetEdge, emit ResetSubtree for its scope
pub(crate) fn reset_on_transition_actions(
    transition_action: On<crate::TransitionActions>,
    q_reset_edge: Query<&ResetEdge>,
    q_edge: Query<(&Source, &Target)>,
    q_children: Query<&crate::StateChildren>,
    mut commands: Commands,
) {
    let edge = transition_action.target;
    let Ok(reset) = q_reset_edge.get(edge) else { return; };
    
    let Ok((Source(source), Target(target))) = q_edge.get(edge) else { return; };

    let mut entities = vec![];

    match reset.0 {
        ResetScope::Source => {
            entities.push(*source);
            entities.extend(q_children.iter_descendants(*source));
        }
        ResetScope::Target => {
            entities.push(*target);
            entities.extend(q_children.iter_descendants(*target));
        }
        ResetScope::Both => {
            entities.push(*source);
            entities.push(*target);
            entities.extend(q_children.iter_descendants(*source));
            entities.extend(q_children.iter_descendants(*target));
        }
    }

    for entity in entities {
        commands.trigger(Reset::new(entity));
    }
}

/// Tick After timers and fire the first due transition per active source, respecting Transitions order.
pub fn tick_after_system(
    time: Res<Time>,
    q_transitions: Query<(Entity, &Transitions), With<Active>>, // active source states only
    mut q_timer: Query<&mut EdgeTimer>,
    q_after: Query<&After>,
    q_always: Query<(), With<AlwaysEdge>>,
    q_guards: Query<&Guards>,
    q_edge_target: Query<&Target>,
    q_child_of: Query<&StateChildOf>,
    mut commands: Commands,
) {
    for (source, transitions) in q_transitions.iter() {
        // Walk edges in priority order; fire first eligible
        for edge in transitions.into_iter().copied() {
            if q_after.get(edge).is_err() { continue; }
            if q_always.get(edge).is_err() { continue; }
            let Ok(mut timer) = q_timer.get_mut(edge) else { continue; };
            timer.0.tick(time.delta());
            if !timer.0.just_finished() { continue; }

            // Validate edge (guards and target) before firing
            if !validate_edge_basic(edge, &q_guards, &q_edge_target) {
                // Cancel invalid timer
                commands.entity(edge).remove::<EdgeTimer>();
                continue;
            }

            // Cancel timer to avoid multiple firings if state persists
            commands.entity(edge).remove::<EdgeTimer>();

            // Emit transition to the machine root with empty payload
            let root = q_child_of.root_ancestor(source);
            commands.trigger(Transition { machine: root, source, edge, payload: () });
            break; // only one delayed transition per source per frame
        }
    }
}

/// Generic system to replay deferred event when a state exits.
pub fn replay_deferred_event<E: EntityEvent + RegisteredTransitionEvent + Clone>(
    exit_state: On<ExitState>,
    mut q_defer: Query<&mut DeferEvent<E>>,
    mut commands: Commands,
)
where
    for<'a> <E as Event>::Trigger<'a>: Default,
{
    let exited_state = exit_state.target;

    if let Ok(mut defer_event) = q_defer.get_mut(exited_state) {
        if let Some(deferred) = defer_event.take_deferred() {
            commands.trigger(deferred);
        }
    }
}

/// Timer system for event edges with After; fire when due
pub fn tick_after_event_timers<E: TransitionEvent + RegisteredTransitionEvent + Clone + 'static>(
    time: Res<Time>,
    mut q_timer: Query<(Entity, &mut EdgeTimer, &PendingEvent<E>), With<EventEdge<E>>>,
    q_after: Query<&After>,
    q_guards: Query<&Guards>,
    q_edge_target: Query<&Target>,
    q_edge_source: Query<&Source>,
    q_child_of: Query<&StateChildOf>,
    q_active: Query<(), With<Active>>,
    mut commands: Commands,
) {
    for (edge, mut timer, pending) in q_timer.iter_mut() {
        // Only consider edges that still have After
        if q_after.get(edge).is_err() { continue; }

        // If the source is no longer active, cancel the pending event
        let Ok(Source(source)) = q_edge_source.get(edge) else { continue; };
        if q_active.get(*source).is_err() {
            cleanup_edge_timer_and_pending::<E>(&mut commands, edge);
            continue;
        }

        timer.0.tick(time.delta());
        if !timer.0.just_finished() { continue; }

        // Validate edge (guards and target) before firing
        if !validate_edge_basic(edge, &q_guards, &q_edge_target) {
            // Cancel invalid timer/pending
            cleanup_edge_timer_and_pending::<E>(&mut commands, edge);
            continue;
        }

        let payload = PhaseEvents {
            exit: pending.event.to_exit_event(),
            effect: pending.event.to_effect_event(),
            entry: pending.event.to_entry_event(),
        };

        // Cleanup timer/pending and fire the transition to machine root
        cleanup_edge_timer_and_pending::<E>(&mut commands, edge);
        let root = q_child_of.root_ancestor(*source);
        commands.trigger(Transition { machine: root, source: *source, edge, payload });
    }
}



/// Cancel a pending delayed event for a source when it exits
pub fn cancel_pending_event_on_exit<E: EntityEvent + RegisteredTransitionEvent + Clone + 'static>(
    exit_state: On<ExitState>,
    q_transitions: Query<&Transitions>,
    q_pending: Query<&PendingEvent<E>>,
    mut commands: Commands,
){
    let source = exit_state.target;
    let Ok(transitions) = q_transitions.get(source) else { return; };
    for &edge in transitions.into_iter() {
        if q_pending.get(edge).is_ok() {
            cleanup_edge_timer_and_pending::<E>(&mut commands, edge);
        }
    }
}