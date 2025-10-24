use bevy::prelude::*;

use crate::{EnterState, ExitState, StateChildOf, StateMachine};

#[derive(Component, Default)]
pub struct Active;

#[derive(Component, Default)]
pub struct Inactive;

pub(crate) fn add_active(
    enter_state: On<EnterState>,
    mut commands: Commands,
) {
    let target = enter_state.target;
    commands.queue(move |world: &mut World| {
        let root = world.query::<&StateChildOf>().query(world).root_ancestor(target);
        let Some(state_machine) = world.entity(root).get::<StateMachine>() else { return; };
        if state_machine.active.contains(&target) {
            world.entity_mut(target).remove::<Inactive>().insert(Active);
        }
    });
}

pub(crate) fn add_inactive(
    exit_state: On<ExitState>,
    mut commands: Commands,
) {
    let target = exit_state.target;
    commands.queue(move |world: &mut World| {
        let root = world.query::<&StateChildOf>().query(world).root_ancestor(target);
        let Some(state_machine) = world.entity(root).get::<StateMachine>() else { return; };
        if !state_machine.active.contains(&target) {
            world.entity_mut(target).remove::<Active>().insert(Inactive);
        }
    });
}