//! Regression tests for delayed message edges.
//!
//! A `MessageEdge<M>` with a `Delay` component should:
//! 1. NOT fire the transition immediately when the message arrives
//! 2. Start an `EdgeTimer` on the edge entity when the message first matches
//! 3. Fire the transition when the timer expires (via `tick_delay_timers`)
//! 4. Cancel the timer if the source state is exited before the delay elapses

use std::time::Duration;

use bevy::prelude::*;
use bevy_gearbox::prelude::*;
use bevy_gearbox::{EdgeTimer, GearboxPlugin};

#[derive(Message, Clone, Reflect)]
struct Go {
    target: Entity,
}

impl GearboxMessage for Go {
    type Validator = AcceptAll;
    fn target(&self) -> Entity {
        self.target
    }
}

/// A delayed message edge should NOT fire immediately. The message is consumed
/// and an EdgeTimer is created, but the state remains unchanged.
#[test]
fn delayed_message_edge_does_not_fire_immediately() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));
    app.register_transition::<Go>();

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let a = world.spawn(SubstateOf(machine)).id();
    let b = world.spawn(SubstateOf(machine)).id();

    let edge = world
        .spawn((
            Source(a),
            Target(b),
            MessageEdge::<Go>::default(),
            Delay::new(Duration::from_secs(5)),
        ))
        .id();

    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(a)));

    // First update: machine initializes, state A is entered.
    app.update();
    assert!(app.world().get::<StateMachine>(machine).unwrap().is_active(&a));

    // Send the message.
    app.world_mut().write_message(Go { target: machine });
    app.update();

    // State should still be A — the delay hasn't elapsed.
    assert!(
        app.world().get::<StateMachine>(machine).unwrap().is_active(&a),
        "Delayed message edge should not fire immediately"
    );

    // But an EdgeTimer should have been created on the edge.
    assert!(
        app.world().get::<EdgeTimer>(edge).is_some(),
        "EdgeTimer should be created when a message first matches a delayed edge"
    );
}

/// A delayed message edge should fire after the delay elapses.
/// Uses Duration::ZERO so the timer finishes on the next tick.
#[test]
fn delayed_message_edge_fires_after_delay() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));
    app.register_transition::<Go>();

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let a = world.spawn(SubstateOf(machine)).id();
    let b = world.spawn(SubstateOf(machine)).id();

    world.spawn((
        Source(a),
        Target(b),
        MessageEdge::<Go>::default(),
        Delay::new(Duration::ZERO),
    ));

    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(a)));

    // Frame 1: initialize machine.
    app.update();
    assert!(app.world().get::<StateMachine>(machine).unwrap().is_active(&a));

    // Frame 2: send message — timer is created (Duration::ZERO).
    app.world_mut().write_message(Go { target: machine });
    app.update();

    // Frame 3: tick_delay_timers fires the transition.
    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(
        state.is_active(&b),
        "Delayed message edge should fire after the delay elapses, got active: {:?}",
        state.active_leaves
    );
}

/// If the source state is exited before the delayed message edge's timer
/// fires, the timer should be cancelled and the transition should not occur.
#[test]
fn delayed_message_edge_cancelled_on_exit() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));
    app.register_transition::<Go>();

    #[derive(Message, Clone, Reflect)]
    struct Interrupt {
        target: Entity,
    }
    impl GearboxMessage for Interrupt {
        type Validator = AcceptAll;
        fn target(&self) -> Entity { self.target }
    }
    app.register_transition::<Interrupt>();

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let a = world.spawn(SubstateOf(machine)).id();
    let b = world.spawn(SubstateOf(machine)).id();
    let c = world.spawn(SubstateOf(machine)).id();

    // Delayed: A --[Go, 5s]--> B
    let delayed_edge = world
        .spawn((
            Source(a),
            Target(b),
            MessageEdge::<Go>::default(),
            Delay::new(Duration::from_secs(5)),
        ))
        .id();

    // Instant: A --[Interrupt]--> C
    world.spawn((
        Source(a),
        Target(c),
        MessageEdge::<Interrupt>::default(),
    ));

    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(a)));

    app.update();

    // Send Go — starts the delayed timer.
    app.world_mut().write_message(Go { target: machine });
    app.update();
    assert!(app.world().get::<EdgeTimer>(delayed_edge).is_some());

    // Send Interrupt — exits A immediately, goes to C.
    app.world_mut().write_message(Interrupt { target: machine });
    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(
        state.is_active(&c),
        "Interrupt should have moved to C"
    );
    // The delayed edge's timer should be cancelled.
    assert!(
        app.world().get::<EdgeTimer>(delayed_edge).is_none(),
        "EdgeTimer should be cancelled when the source state exits"
    );
}

/// Sending the same message repeatedly while the timer is running should
/// NOT restart the timer — the first message consumes the edge.
#[test]
fn repeated_messages_do_not_restart_delay() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));
    app.register_transition::<Go>();

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let a = world.spawn(SubstateOf(machine)).id();
    let b = world.spawn(SubstateOf(machine)).id();

    let edge = world
        .spawn((
            Source(a),
            Target(b),
            MessageEdge::<Go>::default(),
            Delay::new(Duration::from_secs(5)),
        ))
        .id();

    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(a)));

    app.update();

    // First message starts the timer.
    app.world_mut().write_message(Go { target: machine });
    app.update();
    assert!(app.world().get::<EdgeTimer>(edge).is_some());

    // Subsequent messages should not affect the timer.
    app.world_mut().write_message(Go { target: machine });
    app.update();
    app.world_mut().write_message(Go { target: machine });
    app.update();

    // State should still be A — delay hasn't elapsed.
    assert!(
        app.world().get::<StateMachine>(machine).unwrap().is_active(&a),
        "Repeated messages should not cause early firing"
    );
}
