use bevy::prelude::*;
use bevy::platform::collections::HashSet;
use bevy::ecs::component::Mutable;
use std::any::TypeId;

use crate::{parameter, state_component::StateComponentAppExt};
use crate::transitions::TransitionEvent;
use crate::transitions::{edge_event_listener, PhaseEvents, tick_after_event_timers, cancel_pending_event_on_exit, replay_deferred_event};

/// Marker trait implemented by macros for events that are auto-registered.
pub trait RegisteredTransitionEvent: 'static {}

/// Internal resource to dedupe per-event installation.
#[derive(Resource, Default)]
pub struct InstalledTransitions(pub HashSet<TypeId>);

// Dedupe resources to avoid double registration
#[derive(Resource, Default)]
pub struct InstalledStateComponents(pub HashSet<TypeId>);

#[derive(Resource, Default)]
pub struct InstalledFloatParams(pub HashSet<TypeId>);

#[derive(Resource, Default)]
pub struct InstalledIntParams(pub HashSet<TypeId>);

#[derive(Resource, Default)]
pub struct InstalledBoolParams(pub HashSet<TypeId>);

// Installer records collected via `inventory`
// Transition installers are declared under the `transitions` module path for macro stability
pub struct TransitionInstaller { pub install: fn(&mut App) }
inventory::collect!(TransitionInstaller);

pub struct StateInstaller { pub install: fn(&mut App) }
inventory::collect!(StateInstaller);

pub struct FloatParamInstaller { pub install: fn(&mut App) }
inventory::collect!(FloatParamInstaller);

pub struct IntParamInstaller { pub install: fn(&mut App) }
inventory::collect!(IntParamInstaller);

pub struct BoolParamInstaller { pub install: fn(&mut App) }
inventory::collect!(BoolParamInstaller);

// Param binding installers (sync from source component T into Param<P>)
pub struct FloatParamBindingInstaller { pub install: fn(&mut App) }
inventory::collect!(FloatParamBindingInstaller);

pub struct IntParamBindingInstaller { pub install: fn(&mut App) }
inventory::collect!(IntParamBindingInstaller);

pub struct BoolParamBindingInstaller { pub install: fn(&mut App) }
inventory::collect!(BoolParamBindingInstaller);

// Public helpers for macros to call
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

pub fn register_state_component<T>(app: &mut App)
where
    T: Component<Mutability = Mutable> + Clone + 'static,
{
    if !app.world().contains_resource::<InstalledStateComponents>() {
        app.insert_resource(InstalledStateComponents(HashSet::new()));
    }
    let mut installed = app.world_mut().resource_mut::<InstalledStateComponents>();
    let already = !installed.0.insert(TypeId::of::<T>());
    drop(installed);
    if already { return; }

    app.add_state_component::<T>();
}

pub fn register_float_param<P>(app: &mut App)
where
    P: Send + Sync + 'static,
{
    if !app.world().contains_resource::<InstalledFloatParams>() {
        app.insert_resource(InstalledFloatParams(HashSet::new()));
    }
    let mut installed = app.world_mut().resource_mut::<InstalledFloatParams>();
    let already = !installed.0.insert(TypeId::of::<P>());
    drop(installed);
    if already { return; }

    app.add_systems(Update, parameter::apply_float_param_guards::<P>);
}

pub fn register_int_param<P>(app: &mut App)
where
    P: Send + Sync + 'static,
{
    if !app.world().contains_resource::<InstalledIntParams>() {
        app.insert_resource(InstalledIntParams(HashSet::new()));
    }
    let mut installed = app.world_mut().resource_mut::<InstalledIntParams>();
    let already = !installed.0.insert(TypeId::of::<P>());
    drop(installed);
    if already { return; }

    app.add_systems(Update, parameter::apply_int_param_guards::<P>);
}

pub fn register_bool_param<P>(app: &mut App)
where
    P: Send + Sync + 'static,
{
    if !app.world().contains_resource::<InstalledBoolParams>() {
        app.insert_resource(InstalledBoolParams(HashSet::new()));
    }
    let mut installed = app.world_mut().resource_mut::<InstalledBoolParams>();
    let already = !installed.0.insert(TypeId::of::<P>());
    drop(installed);
    if already { return; }

    app.add_systems(Update, parameter::apply_bool_param_guards::<P>);
}

// Deduping for (T, P) bindings
#[derive(Resource, Default)]
pub struct InstalledFloatParamBindings(pub HashSet<(TypeId, TypeId)>);

#[derive(Resource, Default)]
pub struct InstalledIntParamBindings(pub HashSet<(TypeId, TypeId)>);

#[derive(Resource, Default)]
pub struct InstalledBoolParamBindings(pub HashSet<(TypeId, TypeId)>);

pub fn register_float_param_binding<T, P>(app: &mut App)
where
    T: Component + 'static,
    P: parameter::FloatParamBinding<T> + Send + Sync + 'static,
{
    if !app.world().contains_resource::<InstalledFloatParamBindings>() {
        app.insert_resource(InstalledFloatParamBindings(HashSet::new()));
    }
    let mut installed = app.world_mut().resource_mut::<InstalledFloatParamBindings>();
    let already = !installed.0.insert((TypeId::of::<T>(), TypeId::of::<P>()));
    drop(installed);
    if already { return; }

    app.add_systems(Update, parameter::sync_float_param::<T, P>);
}

pub fn register_int_param_binding<T, P>(app: &mut App)
where
    T: Component + 'static,
    P: parameter::IntParamBinding<T> + Send + Sync + 'static,
{
    if !app.world().contains_resource::<InstalledIntParamBindings>() {
        app.insert_resource(InstalledIntParamBindings(HashSet::new()));
    }
    let mut installed = app.world_mut().resource_mut::<InstalledIntParamBindings>();
    let already = !installed.0.insert((TypeId::of::<T>(), TypeId::of::<P>()));
    drop(installed);
    if already { return; }

    app.add_systems(Update, parameter::sync_int_param::<T, P>);
}

pub fn register_bool_param_binding<T, P>(app: &mut App)
where
    T: Component + 'static,
    P: parameter::BoolParamBinding<T> + Send + Sync + 'static,
{
    if !app.world().contains_resource::<InstalledBoolParamBindings>() {
        app.insert_resource(InstalledBoolParamBindings(HashSet::new()));
    }
    let mut installed = app.world_mut().resource_mut::<InstalledBoolParamBindings>();
    let already = !installed.0.insert((TypeId::of::<T>(), TypeId::of::<P>()));
    drop(installed);
    if already { return; }

    app.add_systems(Update, parameter::sync_bool_param::<T, P>);
}

pub fn run_auto_installers(app: &mut App) {
    // Existing transition auto-registration
    for installer in inventory::iter::<TransitionInstaller> {
        (installer.install)(app);
    }
    // New: state components and parameters
    for installer in inventory::iter::<StateInstaller> {
        (installer.install)(app);
    }
    for installer in inventory::iter::<FloatParamInstaller> {
        (installer.install)(app);
    }
    for installer in inventory::iter::<IntParamInstaller> {
        (installer.install)(app);
    }
    for installer in inventory::iter::<BoolParamInstaller> {
        (installer.install)(app);
    }
    // Param bindings
    for installer in inventory::iter::<FloatParamBindingInstaller> {
        (installer.install)(app);
    }
    for installer in inventory::iter::<IntParamBindingInstaller> {
        (installer.install)(app);
    }
    for installer in inventory::iter::<BoolParamBindingInstaller> {
        (installer.install)(app);
    }
}

/// Function-style plugin to run inventory-based auto-registrations without the full GearboxPlugin
pub fn gearbox_auto_register_plugin(app: &mut App) {
    run_auto_installers(app);
}

