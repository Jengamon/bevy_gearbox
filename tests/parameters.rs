//! Tests for parameter-driven guards:
//! - BoolParam with BoolEquals guard on AlwaysEdge
//! - FloatParam with FloatInRange guard
//! - IntParam with IntInRange guard
//! - Parameter value changes dynamically unblocking transitions

use bevy::prelude::*;
use bevy_gearbox::prelude::*;
use bevy_gearbox::GearboxPlugin;

struct IsDead;
struct Health;
struct Score;

/// BoolEquals<IsDead>(true) should block the AlwaysEdge while the param is
/// false, then fire once it becomes true.
#[test]
fn bool_param_guards_always_edge() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin));
    app.register_bool_param::<IsDead>();

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let alive = world.spawn(SubstateOf(machine)).id();
    let dead = world.spawn(SubstateOf(machine)).id();

    world.spawn((
        Source(alive),
        Target(dead),
        AlwaysEdge,
        BoolEquals::<IsDead>::new(true),
    ));

    world.entity_mut(machine).insert((
        StateMachine::new(),
        InitialState(alive),
        BoolParam::<IsDead>::default(), // false
    ));

    // Param is false → guard blocks → stays in alive
    app.update();
    assert!(
        app.world()
            .get::<StateMachine>(machine)
            .unwrap()
            .is_active(&alive)
    );
    assert!(
        !app.world()
            .get::<StateMachine>(machine)
            .unwrap()
            .is_active(&dead)
    );

    // Set param to true
    app.world_mut()
        .get_mut::<BoolParam<IsDead>>(machine)
        .unwrap()
        .set(true);

    // PreUpdate clears guard → GearboxSchedule fires the AlwaysEdge
    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(
        state.active_leaves.contains(&dead),
        "Should transition to dead when IsDead becomes true, got leaves: {:?}",
        state.active_leaves
    );
}

/// While the BoolParam stays at the non-matching value, the edge should never
/// fire — even across multiple updates.
#[test]
fn bool_param_false_keeps_guard_active() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin));
    app.register_bool_param::<IsDead>();

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let alive = world.spawn(SubstateOf(machine)).id();
    let dead = world.spawn(SubstateOf(machine)).id();

    world.spawn((
        Source(alive),
        Target(dead),
        AlwaysEdge,
        BoolEquals::<IsDead>::new(true),
    ));

    world.entity_mut(machine).insert((
        StateMachine::new(),
        InitialState(alive),
        BoolParam::<IsDead>::default(),
    ));

    // Several updates with param remaining false
    for _ in 0..5 {
        app.update();
    }

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(
        state.active_leaves.contains(&alive),
        "Should stay in alive when IsDead remains false"
    );
}

/// FloatInRange<Health>(0.0, 20.0) should allow the transition when the param
/// value is within range.
#[test]
fn float_param_in_range_allows_transition() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin));
    app.register_float_param::<Health>();

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let normal = world.spawn(SubstateOf(machine)).id();
    let critical = world.spawn(SubstateOf(machine)).id();

    world.spawn((
        Source(normal),
        Target(critical),
        AlwaysEdge,
        FloatInRange::<Health>::new(0.0, 20.0, 0.0),
    ));

    // Health defaults to 0.0 which IS in [0, 20]
    world.entity_mut(machine).insert((
        StateMachine::new(),
        InitialState(normal),
        FloatParam::<Health>::default(),
    ));

    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(
        state.active_leaves.contains(&critical),
        "Should transition when health (0.0) is in [0, 20], got leaves: {:?}",
        state.active_leaves
    );
}

/// FloatInRange<Health>(0.0, 20.0) should block the transition when the param
/// value is outside the range.
#[test]
fn float_param_out_of_range_blocks_transition() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin));
    app.register_float_param::<Health>();

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let normal = world.spawn(SubstateOf(machine)).id();
    let critical = world.spawn(SubstateOf(machine)).id();

    world.spawn((
        Source(normal),
        Target(critical),
        AlwaysEdge,
        FloatInRange::<Health>::new(0.0, 20.0, 0.0),
    ));

    let mut health = FloatParam::<Health>::default();
    health.set(100.0);

    world.entity_mut(machine).insert((
        StateMachine::new(),
        InitialState(normal),
        health,
    ));

    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(
        state.active_leaves.contains(&normal),
        "Should stay in normal when health=100 is outside [0, 20]"
    );
}

/// Dynamically changing a FloatParam from out-of-range to in-range should
/// unblock the transition on the next update.
#[test]
fn float_param_dynamic_change_unblocks_edge() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin));
    app.register_float_param::<Health>();

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let normal = world.spawn(SubstateOf(machine)).id();
    let critical = world.spawn(SubstateOf(machine)).id();

    world.spawn((
        Source(normal),
        Target(critical),
        AlwaysEdge,
        FloatInRange::<Health>::new(0.0, 20.0, 0.0),
    ));

    let mut health = FloatParam::<Health>::default();
    health.set(100.0);

    world.entity_mut(machine).insert((
        StateMachine::new(),
        InitialState(normal),
        health,
    ));

    app.update();
    assert!(
        app.world()
            .get::<StateMachine>(machine)
            .unwrap()
            .is_active(&normal)
    );

    // Drop health into the critical range
    app.world_mut()
        .get_mut::<FloatParam<Health>>(machine)
        .unwrap()
        .set(10.0);

    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(
        state.active_leaves.contains(&critical),
        "Should transition after health drops to 10 (in [0, 20])"
    );
}

/// IntInRange<Score>(100, MAX) should fire when the score reaches the
/// threshold.
#[test]
fn int_param_in_range_allows_transition() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin));
    app.register_int_param::<Score>();

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let playing = world.spawn(SubstateOf(machine)).id();
    let winning = world.spawn(SubstateOf(machine)).id();

    world.spawn((
        Source(playing),
        Target(winning),
        AlwaysEdge,
        IntInRange::<Score>::new(100, i32::MAX, 0),
    ));

    let mut score = IntParam::<Score>::default();
    score.set(150);

    world.entity_mut(machine).insert((
        StateMachine::new(),
        InitialState(playing),
        score,
    ));

    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(
        state.active_leaves.contains(&winning),
        "Should transition when score=150 is in [100, MAX]"
    );
}

/// IntInRange should block when the value is below the range.
#[test]
fn int_param_below_range_blocks_transition() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin));
    app.register_int_param::<Score>();

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let playing = world.spawn(SubstateOf(machine)).id();
    let winning = world.spawn(SubstateOf(machine)).id();

    world.spawn((
        Source(playing),
        Target(winning),
        AlwaysEdge,
        IntInRange::<Score>::new(100, i32::MAX, 0),
    ));

    let mut score = IntParam::<Score>::default();
    score.set(50);

    world.entity_mut(machine).insert((
        StateMachine::new(),
        InitialState(playing),
        score,
    ));

    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(
        state.active_leaves.contains(&playing),
        "Should stay in playing when score=50 is below [100, MAX]"
    );
}
