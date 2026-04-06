//! Schedule-based state machine resolution.
//!
//! Instead of observers that resolve transitions recursively in one shot,
//! this crate uses a dedicated [`GearboxSchedule`] that runs in a loop:
//!
//! 1. Systems read pending [`TransitionMessage`]s via [`MessageReader`]
//! 2. They compute exits/entries, update [`StateMachine`], and insert/remove [`Active`]
//! 3. Commands flush — [`Active`] changes are visible to subsequent phases
//! 4. User systems in [`GearboxPhase::ExitPhase`] / [`GearboxPhase::EntryPhase`] react
//!    via `Added<Active>` and `RemovedComponents<Active>` queries
//! 5. [`check_always_edges`] may produce *new* messages (e.g. AlwaysEdge becoming eligible)
//! 6. The schedule runs again until no new messages are produced or [`IterationCap`] is hit
//!
//! ```text
//! GearboxSchedule (loops until converged):
//!   TransitionPhase <- resolve_transitions (inserts/removes Active)
//!   apply_deferred  <- flush commands so Active is visible
//!   ExitPhase       <- user systems reacting to RemovedComponents<Active>
//!   EntryPhase      <- user systems reacting to Added<Active>
//!   GaugeSync       <- (gauge feature) sync WriteBack + AttributeDerived components
//!   EdgeCheckPhase  <- check_always_edges (internal)
//! ```
//!
//! After the schedule converges, [`EnterState`] / [`ExitState`] entity events
//! are triggered for observer-based consumers.
//!
//! Timer ticking, parameter sync, and other per-frame work belongs in
//! [`Update`] ordered relative to [`GearboxSet`].
//!
//! This is analogous to how Avian runs a physics schedule multiple times per frame.

pub mod components;
pub mod history;
pub mod state_component;
pub mod resolve;
pub mod messages;
pub mod parameters;
pub mod delay;
pub mod commands;
pub mod registration;
pub mod prelude;
pub mod helpers;

#[cfg(feature = "gauge")]
pub mod gauge;

use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;

use resolve::PendingCount;

// ---------------------------------------------------------------------------
// Re-exports — preserve original public API
// ---------------------------------------------------------------------------

pub use components::{
    Active, TerminalState,
    StateMachine, InitialState, SubstateOf, Substates, Source, Transitions, Target,
    AlwaysEdge, EdgeKind, Guards, Guard, GuardProvider, Delay, EdgeTimer,
    ResetEdge, ResetScope,
};
pub use history::{History, HistoryState};
pub use state_component::{
    StateComponent, StateInactiveComponent,
    state_component_enter, state_component_exit,
    state_inactive_component_enter, state_inactive_component_exit,
};
pub use resolve::{
    TransitionMessage,
    EnterState, ExitState,
};
pub use messages::{
    GearboxMessage, MessageValidator, AcceptAll, MessageEdge, Matched,
    SideEffect, produce_side_effects, message_edge_listener,
    Done, emit_terminal_done,
};
pub use parameters::{
    FloatParam, IntParam, BoolParam,
    FloatParamBinding, IntParamBinding, BoolParamBinding,
    sync_float_param, sync_int_param, sync_bool_param,
    FloatInRange, IntInRange, BoolEquals,
    apply_float_param_guards, apply_int_param_guards, apply_bool_param_guards,
    init_float_param_guard_on_add, init_int_param_guard_on_add, init_bool_param_guard_on_add,
};
pub use commands::{
    SpawnSubstate, SpawnTransition, BuildTransition, TransitionBuilder,
    TransitionExt, InitStateMachine,
    GearboxCommandsExt, BuildEntityEvent,
};
pub use registration::{
    InstalledTransitions, InstalledStateComponents,
    InstalledFloatParams, InstalledIntParams, InstalledBoolParams,
    InstalledFloatParamBindings, InstalledIntParamBindings, InstalledBoolParamBindings,
    InstalledStateBridges,
    RegistrationAppExt, gearbox_auto_register_plugin,
    TransitionInstaller, StateInstaller,
    FloatParamInstaller, IntParamInstaller, BoolParamInstaller,
    FloatParamBindingInstaller, IntParamBindingInstaller, BoolParamBindingInstaller,
    StateBridgeInstaller,
    DeferEvent, replay_deferred_messages, bridge_to_bevy_state,
};

// ---------------------------------------------------------------------------
// Schedule, sets & phases
// ---------------------------------------------------------------------------

/// The schedule that resolves state machine transitions. Runs N times per
/// frame inside [`run_gearbox_schedule`].
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub struct GearboxSchedule;

/// System set in [`Update`] that contains the gearbox schedule runner.
/// Use this for ordering user systems relative to gearbox resolution:
///
/// ```rust,ignore
/// app.add_systems(Update, my_trigger_system.before(GearboxSet));
/// ```
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct GearboxSet;

/// System sets within [`GearboxSchedule`], executed in order each iteration.
///
/// Commands are flushed between `TransitionPhase` and `ExitPhase` so that
/// [`Active`] component changes are visible to user systems.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum GearboxPhase {
    /// Internal: reads transition messages, updates [`StateMachine`],
    /// and inserts/removes [`Active`] components.
    TransitionPhase,
    /// User systems that react to states being exited.
    /// Query `RemovedComponents<Active>` to detect exits.
    ExitPhase,
    /// User systems that react to states being entered.
    /// Query `Added<Active>` to detect entries.
    EntryPhase,
    /// Syncs gauge [`WriteBack`](bevy_gauge::prelude::WriteBack) and
    /// [`AttributeDerived`](bevy_gauge::prelude::AttributeDerived) components
    /// so that derived values are current before edge checks.
    #[cfg(feature = "gauge")]
    GaugeSync,
    /// Internal: checks AlwaysEdge eligibility and writes new messages.
    EdgeCheckPhase,
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Maximum number of iterations the schedule will run per frame.
/// If hit, a warning is logged — this likely indicates a transition loop.
#[derive(Resource)]
pub struct IterationCap(pub u32);

impl Default for IterationCap {
    fn default() -> Self {
        Self(32)
    }
}

// ---------------------------------------------------------------------------
// Initialization system (runs in Update, writes init messages)
// ---------------------------------------------------------------------------

/// Detect newly-added StateMachine components and write initialization messages.
fn enqueue_machine_init(
    q_new_machines: Query<(Entity, &InitialState), Added<StateMachine>>,
    mut writer: MessageWriter<TransitionMessage>,
) {
    for (entity, initial) in &q_new_machines {
        writer.write(TransitionMessage {
            machine: entity,
            source: entity,
            target: initial.0,
            edge: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Schedule runner (the "substep loop")
// ---------------------------------------------------------------------------

/// Runs [`GearboxSchedule`] in a loop until no new messages are produced or cap is hit.
fn run_gearbox_schedule(world: &mut World) {
    let cap = world
        .get_resource::<IterationCap>()
        .map(|c| c.0)
        .unwrap_or(32);

    for iteration in 0..cap {
        world.run_schedule(GearboxSchedule);

        let produced = world
            .get_resource::<PendingCount>()
            .map(|p| p.0)
            .unwrap_or(0);

        if produced == 0 {
            if iteration > 0 {
                debug!(
                    "GearboxSchedule converged after {} iteration(s)",
                    iteration + 1
                );
            }
            return;
        }
    }

    warn!(
        "GearboxSchedule hit iteration cap ({cap}). Possible transition loop!"
    );
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct GearboxPlugin;

impl Plugin for GearboxPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TransitionMessage>()
            .init_resource::<PendingCount>()
            .init_resource::<IterationCap>();

        let mut schedule = Schedule::new(GearboxSchedule);
        #[cfg(not(feature = "gauge"))]
        schedule.configure_sets((
            GearboxPhase::TransitionPhase,
            GearboxPhase::ExitPhase,
            GearboxPhase::EntryPhase,
            GearboxPhase::EdgeCheckPhase,
        ).chain());
        #[cfg(feature = "gauge")]
        schedule.configure_sets((
            GearboxPhase::TransitionPhase,
            GearboxPhase::ExitPhase,
            GearboxPhase::EntryPhase,
            GearboxPhase::GaugeSync,
            GearboxPhase::EdgeCheckPhase,
        ).chain());
        app.add_schedule(schedule);

        // NOTE: `register_transition::<Done>` must come AFTER `add_schedule`.
        // `register_transition` calls `add_systems(GearboxSchedule, ..)`,
        // which lazily creates a `GearboxSchedule` entry if none exists. If
        // called before `add_schedule`, that lazy entry gets clobbered by
        // `add_schedule`'s `insert` — losing the Done listener — while the
        // dedup resource still thinks the registration succeeded.
        app.register_transition::<Done>();

        #[cfg(feature = "gauge")]
        {
            bevy_gauge::derived::add_gauge_sync_to_schedule(app, GearboxSchedule);
            app.configure_sets(
                GearboxSchedule,
                (
                    bevy_gauge::prelude::WriteBackSet
                        .in_set(GearboxPhase::GaugeSync),
                    bevy_gauge::prelude::AttributeDerivedSet
                        .in_set(GearboxPhase::GaugeSync),
                ),
            );
        }

        app.add_systems(
            GearboxSchedule,
            (
                resolve::resolve_transitions.in_set(GearboxPhase::TransitionPhase),
                // Flush Active insert/remove commands so ExitPhase/EntryPhase
                // systems see the changes.
                ApplyDeferred
                    .after(GearboxPhase::TransitionPhase)
                    .before(GearboxPhase::ExitPhase),
                delay::cancel_delay_timers.in_set(GearboxPhase::ExitPhase),
                delay::start_delay_timers.in_set(GearboxPhase::EntryPhase),
                // Flush deferred commands from ExitPhase/EntryPhase (e.g.
                // StateComponent insert/remove) so they are visible to
                // EdgeCheckPhase and to the convergence check before the
                // loop potentially exits.
                ApplyDeferred
                    .after(GearboxPhase::EntryPhase)
                    .before(GearboxPhase::EdgeCheckPhase),
                messages::emit_terminal_done
                    .in_set(GearboxPhase::EdgeCheckPhase)
                    .before(resolve::check_always_edges),
                resolve::check_always_edges.in_set(GearboxPhase::EdgeCheckPhase),
            ),
        );

        // Outer driver: detect new machines, tick delay timers (so their
        // transition messages are in the buffer before the loop runs), run
        // the loop, then flush entity events for observer users.
        //
        // Ticking delays before the loop means a delay that finishes this
        // frame gets its transition applied in the same frame, and any
        // cascade it triggers resolves in the same frame too. Ticking after
        // the loop (the old layout) would leak a one-frame latency on every
        // delayed transition.
        app.add_systems(
            Update,
            (
                enqueue_machine_init,
                delay::tick_delay_timers,
                run_gearbox_schedule,
            )
                .chain()
                .in_set(GearboxSet),
        );
        app.add_systems(
            Update,
            resolve::flush_state_events
                .in_set(GearboxSet)
                .after(run_gearbox_schedule),
        );
    }
}
