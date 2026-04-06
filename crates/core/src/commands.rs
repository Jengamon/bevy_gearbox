use std::time::Duration;

use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;

use crate::components::*;
use crate::messages::{GearboxMessage, MessageEdge};

/// Extension trait for spawning substates with less boilerplate.
pub trait SpawnSubstate {
    type Out<'a>
    where
        Self: 'a;
    fn spawn_substate<B: Bundle>(&mut self, parent: Entity, bundle: B) -> Self::Out<'_>;
}

impl SpawnSubstate for Commands<'_, '_> {
    type Out<'a>
        = EntityCommands<'a>
    where
        Self: 'a;
    fn spawn_substate<B: Bundle>(&mut self, parent: Entity, bundle: B) -> EntityCommands<'_> {
        self.spawn((SubstateOf(parent), bundle))
    }
}

impl SpawnSubstate for ChildSpawnerCommands<'_> {
    type Out<'a>
        = EntityCommands<'a>
    where
        Self: 'a;
    fn spawn_substate<B: Bundle>(&mut self, parent: Entity, bundle: B) -> EntityCommands<'_> {
        self.spawn((SubstateOf(parent), bundle))
    }
}

/// Extension trait for spawning transitions.
pub trait SpawnTransition {
    type Out<'a>
    where
        Self: 'a;
    fn spawn_transition<M: GearboxMessage>(
        &mut self,
        source: Entity,
        target: Entity,
    ) -> Self::Out<'_>;
    fn spawn_transition_always(&mut self, source: Entity, target: Entity) -> Self::Out<'_>;
}

impl SpawnTransition for Commands<'_, '_> {
    type Out<'a>
        = EntityCommands<'a>
    where
        Self: 'a;

    fn spawn_transition<M: GearboxMessage>(
        &mut self,
        source: Entity,
        target: Entity,
    ) -> EntityCommands<'_> {
        self.spawn((Source(source), Target(target), MessageEdge::<M>::default()))
    }

    fn spawn_transition_always(&mut self, source: Entity, target: Entity) -> EntityCommands<'_> {
        self.spawn((Source(source), Target(target), AlwaysEdge))
    }
}

impl SpawnTransition for ChildSpawnerCommands<'_> {
    type Out<'a>
        = EntityCommands<'a>
    where
        Self: 'a;

    fn spawn_transition<M: GearboxMessage>(
        &mut self,
        source: Entity,
        target: Entity,
    ) -> EntityCommands<'_> {
        self.spawn((Source(source), Target(target), MessageEdge::<M>::default()))
    }

    fn spawn_transition_always(&mut self, source: Entity, target: Entity) -> EntityCommands<'_> {
        self.spawn((Source(source), Target(target), AlwaysEdge))
    }
}

/// Builder for always-transitions with deferred component inserts.
pub struct TransitionBuilder {
    pub(crate) deferred: Vec<Box<dyn FnOnce(&mut EntityCommands) + Send + Sync>>,
}

impl TransitionBuilder {
    pub(crate) fn new() -> Self {
        Self {
            deferred: Vec::new(),
        }
    }

    pub fn insert<B: Bundle>(&mut self, bundle: B) -> &mut Self {
        self.deferred.push(Box::new(move |ec| {
            ec.insert(bundle);
        }));
        self
    }

    pub fn with_delay(&mut self, duration: Duration) -> &mut Self {
        self.deferred.push(Box::new(move |ec| {
            ec.insert(Delay { duration });
        }));
        self
    }

    pub fn with_name(&mut self, name: impl Into<String> + Send + Sync + 'static) -> &mut Self {
        self.deferred.push(Box::new(move |ec| {
            ec.insert(Name::new(name.into()));
        }));
        self
    }
}

/// Extension trait for building guarded always-transitions.
pub trait BuildTransition {
    type Out<'a>
    where
        Self: 'a;
    fn build_transition_always(
        &mut self,
        source: Entity,
        target: Entity,
        configure: impl FnOnce(&mut TransitionBuilder),
    ) -> Self::Out<'_>;
}

impl BuildTransition for Commands<'_, '_> {
    type Out<'a>
        = EntityCommands<'a>
    where
        Self: 'a;

    fn build_transition_always(
        &mut self,
        source: Entity,
        target: Entity,
        configure: impl FnOnce(&mut TransitionBuilder),
    ) -> EntityCommands<'_> {
        let mut builder = TransitionBuilder::new();
        configure(&mut builder);
        let mut ec = self.spawn((Source(source), Target(target), AlwaysEdge));
        for f in builder.deferred {
            f(&mut ec);
        }
        ec
    }
}

impl BuildTransition for ChildSpawnerCommands<'_> {
    type Out<'a>
        = EntityCommands<'a>
    where
        Self: 'a;

    fn build_transition_always(
        &mut self,
        source: Entity,
        target: Entity,
        configure: impl FnOnce(&mut TransitionBuilder),
    ) -> EntityCommands<'_> {
        let mut builder = TransitionBuilder::new();
        configure(&mut builder);
        let mut ec = self.spawn((Source(source), Target(target), AlwaysEdge));
        for f in builder.deferred {
            f(&mut ec);
        }
        ec
    }
}

// ---------------------------------------------------------------------------
// Branching transitions
// ---------------------------------------------------------------------------

/// Builder for configuring branch arms.
pub struct BranchBuilder {
    arms: Vec<BranchArmBuilder>,
    otherwise: Option<Entity>,
}

struct BranchArmBuilder {
    target: Entity,
    configure: Box<dyn FnOnce(&mut TransitionBuilder) + Send + Sync>,
}

impl BranchBuilder {
    fn new() -> Self {
        Self {
            arms: Vec::new(),
            otherwise: None,
        }
    }

    /// Add a conditional arm. The `configure` closure sets up guards on the arm
    /// (same API as [`TransitionBuilder`]). Arms are evaluated in registration order.
    pub fn when(
        &mut self,
        target: Entity,
        configure: impl FnOnce(&mut TransitionBuilder) + Send + Sync + 'static,
    ) -> &mut Self {
        self.arms.push(BranchArmBuilder {
            target,
            configure: Box::new(configure),
        });
        self
    }

    /// Set the fallback target if no arm's guards pass.
    pub fn otherwise(&mut self, target: Entity) -> &mut Self {
        self.otherwise = Some(target);
        self
    }
}

/// Extension trait for spawning branching transitions.
pub trait SpawnBranch {
    type Out<'a>
    where
        Self: 'a;

    /// Spawn a branching always-edge from `source`.
    fn spawn_branch_always(
        &mut self,
        source: Entity,
        configure: impl FnOnce(&mut BranchBuilder),
    ) -> Self::Out<'_>;

    /// Spawn a branching message-edge from `source`.
    fn spawn_branch<M: GearboxMessage>(
        &mut self,
        source: Entity,
        configure: impl FnOnce(&mut BranchBuilder),
    ) -> Self::Out<'_>;
}

impl SpawnBranch for ChildSpawnerCommands<'_> {
    type Out<'a>
        = EntityCommands<'a>
    where
        Self: 'a;

    fn spawn_branch_always(
        &mut self,
        source: Entity,
        configure: impl FnOnce(&mut BranchBuilder),
    ) -> EntityCommands<'_> {
        let mut builder = BranchBuilder::new();
        configure(&mut builder);

        let otherwise = builder.otherwise.expect("BranchBuilder requires an otherwise() target");

        let arms: Vec<BranchArm> = builder
            .arms
            .into_iter()
            .map(|arm_builder| {
                let mut tb = TransitionBuilder::new();
                (arm_builder.configure)(&mut tb);
                // Spawn an entity for the arm (deferred inserts apply here)
                let arm_entity = self.spawn_empty().id();
                let commands = self.commands_mut();
                let mut ec = commands.entity(arm_entity);
                for f in tb.deferred {
                    f(&mut ec);
                }
                BranchArm {
                    target: arm_builder.target,
                    guard: arm_entity,
                }
            })
            .collect();

        self.spawn((
            Source(source),
            Target(otherwise),
            AlwaysEdge,
            BranchTransition { arms, otherwise },
        ))
    }

    fn spawn_branch<M: GearboxMessage>(
        &mut self,
        source: Entity,
        configure: impl FnOnce(&mut BranchBuilder),
    ) -> EntityCommands<'_> {
        let mut builder = BranchBuilder::new();
        configure(&mut builder);

        let otherwise = builder.otherwise.expect("BranchBuilder requires an otherwise() target");

        let arms: Vec<BranchArm> = builder
            .arms
            .into_iter()
            .map(|arm_builder| {
                let mut tb = TransitionBuilder::new();
                (arm_builder.configure)(&mut tb);
                let arm_entity = self.spawn_empty().id();
                let commands = self.commands_mut();
                let mut ec = commands.entity(arm_entity);
                for f in tb.deferred {
                    f(&mut ec);
                }
                BranchArm {
                    target: arm_builder.target,
                    guard: arm_entity,
                }
            })
            .collect();

        self.spawn((
            Source(source),
            Target(otherwise),
            MessageEdge::<M>::default(),
            BranchTransition { arms, otherwise },
        ))
    }
}

/// Extension methods for transition entities.
pub trait TransitionExt {
    fn with_delay(&mut self, duration: Duration) -> &mut Self;
    fn with_name(&mut self, name: impl Into<String>) -> &mut Self;
}

impl TransitionExt for EntityCommands<'_> {
    fn with_delay(&mut self, duration: Duration) -> &mut Self {
        self.insert(Delay { duration })
    }
    fn with_name(&mut self, name: impl Into<String>) -> &mut Self {
        self.insert(Name::new(name.into()))
    }
}

/// Extension for initializing a state machine on an existing entity.
pub trait InitStateMachine {
    fn init_state_machine(&mut self, initial_state: impl Into<Option<Entity>>) -> &mut Self;
}

impl InitStateMachine for EntityCommands<'_> {
    fn init_state_machine(&mut self, initial_state: impl Into<Option<Entity>>) -> &mut Self {
        if let Some(state) = initial_state.into() {
            self.insert((StateMachine::new(), InitialState(state)))
        } else {
            self.insert(StateMachine::new())
        }
    }
}

// ---------------------------------------------------------------------------
// Chart lookup helpers (find machine by marker component)
// ---------------------------------------------------------------------------

/// Commands helper to interact with a state machine found by a marker component.
///
/// This is useful when you know a machine carries a marker `M` but don't have
/// the `Entity` at hand.
pub trait GearboxCommandsExt {
    /// Trigger an [`Event`] on the machine root found by marker `M`.
    ///
    /// The closure receives the root `Entity` and returns the event to trigger.
    ///
    /// ```rust,ignore
    /// commands.emit_to_chart::<PlayerMarker>(|root| MyEvent::new(root));
    /// ```
    fn emit_to_chart<M>(&mut self, make: impl BuildEntityEvent + Send + 'static)
    where
        M: Component;

    /// Write a [`Message`] addressed to the machine root found by marker `M`.
    ///
    /// ```rust,ignore
    /// commands.write_message_to_chart::<PlayerMarker, _>(|root| Attack { machine: root });
    /// ```
    fn write_message_to_chart<M, Msg>(&mut self, make: impl FnOnce(Entity) -> Msg + Send + 'static)
    where
        M: Component,
        Msg: Message + Send + 'static;
}

impl GearboxCommandsExt for Commands<'_, '_> {
    fn emit_to_chart<M>(&mut self, make: impl BuildEntityEvent + Send + 'static)
    where
        M: Component,
    {
        self.queue(move |world: &mut World| {
            let mut q = world.query_filtered::<Entity, With<M>>();
            if let Ok(root) = q.single(world) {
                make.trigger_into_world(world, root);
            }
        });
    }

    fn write_message_to_chart<M, Msg>(
        &mut self,
        make: impl FnOnce(Entity) -> Msg + Send + 'static,
    ) where
        M: Component,
        Msg: Message + Send + 'static,
    {
        self.queue(move |world: &mut World| {
            let mut q = world.query_filtered::<Entity, With<M>>();
            if let Ok(root) = q.single(world) {
                let msg = make(root);
                world.write_message(msg);
            }
        });
    }
}

/// Helper trait to build and trigger an event given a root entity.
///
/// Blanket-implemented for closures `FnOnce(Entity) -> E` where `E: Event`.
/// The event type is inferred from the closure return type, so callers only
/// need to specify the marker:
///
/// ```rust,ignore
/// commands.emit_to_chart::<PlayerMarker>(|root| MyEntityEvent { target: root });
/// ```
pub trait BuildEntityEvent {
    fn trigger_into_world(self, world: &mut World, root: Entity);
}

impl<F, E> BuildEntityEvent for F
where
    F: FnOnce(Entity) -> E + Send + 'static,
    E: Event,
    for<'a> <E as Event>::Trigger<'a>: Default,
{
    fn trigger_into_world(self, world: &mut World, root: Entity) {
        let event = self(root);
        world.trigger(event);
    }
}
