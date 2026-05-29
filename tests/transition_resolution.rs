//! Tests for core transition resolution mechanics:
//! - AlwaysEdge chains resolving across multiple schedule iterations
//! - Manual TransitionMessages combining with AlwaysEdge chains
//! - Stale source handling
//! - Hierarchical drill-down on initialization
//! - Internal vs external edge kinds

use bevy::prelude::*;
use bevy_gearbox::prelude::*;
use bevy_gearbox::{GearboxPlugin, IterationCap};

/// A -> B -> C -> D via AlwaysEdges should all resolve in a single `update()`.
#[test]
fn always_edge_chain_resolves_in_single_update() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let a = world.spawn(SubstateOf(machine)).id();
    let b = world.spawn(SubstateOf(machine)).id();
    let c = world.spawn(SubstateOf(machine)).id();
    let d = world.spawn(SubstateOf(machine)).id();

    world.spawn((Source(a), Target(b), AlwaysEdge));
    world.spawn((Source(b), Target(c), AlwaysEdge));
    world.spawn((Source(c), Target(d), AlwaysEdge));

    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(a)));

    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(state.active_leaves.contains(&d), "Should reach D in one update");
    assert_eq!(state.active_leaves.len(), 1);
    assert!(!state.active.contains(&a));
    assert!(!state.active.contains(&b));
    assert!(!state.active.contains(&c));
}

/// A manual TransitionMessage followed by an AlwaysEdge should chain within
/// a single `update()`.
#[test]
fn manual_transition_chains_with_always_edge() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let a = world.spawn(SubstateOf(machine)).id();
    let b = world.spawn(SubstateOf(machine)).id();
    let c = world.spawn(SubstateOf(machine)).id();

    // B -> C fires automatically once B is entered
    world.spawn((Source(b), Target(c), AlwaysEdge));

    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(a)));

    app.update();
    assert!(app.world().get::<StateMachine>(machine).unwrap().is_active(&a));

    // Manually push A -> B; the AlwaysEdge should carry on to C
    app.world_mut().write_message(TransitionMessage {
        machine,
        source: a,
        target: b,
        edge: None,
        blocked: false,
    });
    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(
        state.active_leaves.contains(&c),
        "Should chain through B to C, got leaves: {:?}",
        state.active_leaves
    );
    assert!(!state.active_leaves.contains(&b));
}

/// A TransitionMessage whose source is no longer active should be silently
/// dropped.
#[test]
fn transition_from_inactive_source_is_ignored() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let a = world.spawn(SubstateOf(machine)).id();
    let b = world.spawn(SubstateOf(machine)).id();
    let c = world.spawn(SubstateOf(machine)).id();

    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(a)));

    app.update();

    // b is not active — this message should be ignored
    app.world_mut().write_message(TransitionMessage {
        machine,
        source: b,
        target: c,
        edge: None,
        blocked: false,
    });
    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(state.active_leaves.contains(&a), "Should still be in A");
    assert!(!state.active_leaves.contains(&c));
}

/// Self-transition exits and re-enters the state, causing EntryPhase to run
/// again.
#[test]
fn self_transition_exits_and_reenters() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));

    #[derive(Resource, Default)]
    struct EntryCount(u32);

    #[derive(Component)]
    struct Marker;

    fn count_entries(
        q_machine: Query<&StateMachine, With<Marker>>,
        mut count: ResMut<EntryCount>,
    ) {
        for machine in &q_machine {
            if !machine.active_leaves.is_empty() {
                count.0 += 1;
            }
        }
    }

    app.init_resource::<EntryCount>();
    app.add_systems(
        GearboxSchedule,
        count_entries.in_set(GearboxPhase::EntryPhase),
    );

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let a = world.spawn(SubstateOf(machine)).id();

    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(a), Marker));

    app.update();
    let count_after_init = app.world().resource::<EntryCount>().0;

    app.world_mut().write_message(TransitionMessage {
        machine,
        source: a,
        target: a,
        edge: None,
        blocked: false,
    });
    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(state.active_leaves.contains(&a));

    let count_after_self = app.world().resource::<EntryCount>().0;
    assert!(
        count_after_self > count_after_init,
        "EntryPhase should have run again for the self-transition \
         (init={count_after_init}, after={count_after_self})"
    );
}

/// Machine -> P (InitialState=Q) -> Q (InitialState=leaf)
/// Initialization should drill all the way down to the leaf.
#[test]
fn hierarchical_drill_down_on_init() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let p = world.spawn(SubstateOf(machine)).id();
    let q = world.spawn(SubstateOf(p)).id();
    let leaf = world.spawn(SubstateOf(q)).id();

    world.entity_mut(q).insert(InitialState(leaf));
    world.entity_mut(p).insert(InitialState(q));
    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(p)));

    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(state.active_leaves.contains(&leaf), "Should drill down to leaf");
    assert!(state.active.contains(&p), "P should be active (ancestor)");
    assert!(state.active.contains(&q), "Q should be active (ancestor)");
    assert_eq!(state.active_leaves.len(), 1);
}

/// An internal edge whose source IS the LCA should not exit/re-enter the LCA.
#[test]
fn internal_edge_preserves_lca() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let p = world.spawn(SubstateOf(machine)).id();
    let a = world.spawn(SubstateOf(p)).id();
    let b = world.spawn(SubstateOf(p)).id();

    world.spawn((Source(a), Target(b), AlwaysEdge, EdgeKind::Internal));

    world.entity_mut(p).insert(InitialState(a));
    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(p)));

    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(state.active_leaves.contains(&b));
    assert!(
        state.active.contains(&p),
        "P (LCA) should remain active with internal edge"
    );
}

/// A transition sourced from a composite (non-leaf) parent should fire when
/// any of its descendant leaves are active.
#[test]
fn transition_from_composite_parent() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let p = world.spawn(SubstateOf(machine)).id();
    let a = world.spawn(SubstateOf(p)).id();
    let _b = world.spawn(SubstateOf(p)).id();
    let d = world.spawn(SubstateOf(machine)).id();

    world.spawn((Source(p), Target(d), AlwaysEdge));

    world.entity_mut(p).insert(InitialState(a));
    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(p)));

    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(state.active_leaves.contains(&d));
    assert!(!state.active.contains(&p));
}

/// A machine with no AlwaysEdges stays in its initial state.
#[test]
fn stable_state_no_extra_iterations() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let a = world.spawn(SubstateOf(machine)).id();
    let b = world.spawn(SubstateOf(machine)).id();

    // Edge without AlwaysEdge marker — should not fire automatically
    world.spawn((Source(a), Target(b)));

    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(a)));

    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(state.is_active(&a));
    assert!(!state.is_active(&b));
}

/// Active component is inserted on entered states and removed on exited
/// states, with the correct machine reference.
#[test]
fn active_component_tracks_entries_and_exits() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let a = world.spawn(SubstateOf(machine)).id();
    let b = world.spawn(SubstateOf(machine)).id();

    world.spawn((Source(a), Target(b), AlwaysEdge));

    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(a)));

    app.update();

    assert!(
        app.world().get::<Active>(b).is_some(),
        "B should have Active component"
    );
    assert!(
        app.world().get::<Active>(a).is_none(),
        "A should NOT have Active component (it was exited)"
    );
    let active = app.world().get::<Active>(b).unwrap();
    assert_eq!(active.machine, machine);
}

/// With IterationCap(2), a chain A -> B -> C should only get to B.
#[test]
fn iteration_cap_limits_resolution_depth() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GearboxPlugin::default()));
    app.insert_resource(IterationCap(2));

    let world = app.world_mut();
    let machine = world.spawn_empty().id();
    let a = world.spawn(SubstateOf(machine)).id();
    let b = world.spawn(SubstateOf(machine)).id();
    let c = world.spawn(SubstateOf(machine)).id();

    world.spawn((Source(a), Target(b), AlwaysEdge));
    world.spawn((Source(b), Target(c), AlwaysEdge));

    world
        .entity_mut(machine)
        .insert((StateMachine::new(), InitialState(a)));

    app.update();

    let state = app.world().get::<StateMachine>(machine).unwrap();
    assert!(state.is_active(&b), "Cap=2 should only reach B");
    assert!(!state.active_leaves.contains(&c));
}
