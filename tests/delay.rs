//! Tests for delayed transitions:
//! - Delayed AlwaysEdge creates a timer but does not fire immediately
//! - Timer is attached to the edge entity

use std::time::Duration;

use bevy::prelude::*;
use bevy_gearbox::prelude::*;
use bevy_gearbox::{EdgeTimer, GearboxPlugin};

/// A delayed AlwaysEdge should NOT fire in the same frame as entry. It should
/// create an EdgeTimer on the edge entity.
#[test]
fn delayed_always_edge_creates_timer_and_skips_immediate() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let a = world.spawn(SubstateOf(machine)).id();
    let b = world.spawn(SubstateOf(machine)).id();

    let edge = world
        .spawn((
            Source(a),
            Target(b),
            AlwaysEdge,
            Delay::new(Duration::from_secs(5)),
        ))
        .id();

    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(a)));

    app.update();

    assert!(
        app.world()
            .get::<StateMachine>(machine)
            .unwrap()
            .is_active(&a),
        "Should stay in A — delay not elapsed"
    );
    assert!(
        app.world().get::<EdgeTimer>(edge).is_some(),
        "EdgeTimer should exist on the delayed edge after source was entered"
    );
}
