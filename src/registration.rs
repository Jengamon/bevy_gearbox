use bevy::prelude::*;
use bevy::platform::collections::HashSet;
use bevy::ecs::component::Mutable;
use std::any::TypeId;

use crate::parameter;
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

/// Helper trait to register components and systems to an App.
pub trait RegistrationAppExt {
    fn register_transition<E>(&mut self) -> &mut Self
    where
        E: TransitionEvent + RegisteredTransitionEvent + Clone + 'static + bevy::reflect::TypePath,
        for<'a> <E as Event>::Trigger<'a>: Default,
        for<'a> <<E as TransitionEvent>::ExitEvent as Event>::Trigger<'a>: Default,
        for<'a> <<E as TransitionEvent>::EffectEvent as Event>::Trigger<'a>: Default,
        for<'a> <<E as TransitionEvent>::EntryEvent as Event>::Trigger<'a>: Default,
        <E as TransitionEvent>::Validator: bevy::reflect::TypePath + bevy::reflect::FromReflect + bevy::reflect::GetTypeRegistration + bevy::reflect::Typed;

    fn register_state_component<T>(&mut self) -> &mut Self
    where
        T: Component<Mutability = Mutable> + Clone + Reflect + FromReflect + bevy::reflect::TypePath + bevy::reflect::GetTypeRegistration + bevy::reflect::Typed + 'static;

    fn register_float_param<P>(&mut self) -> &mut Self
    where
        P: Send + Sync + 'static;

    fn register_int_param<P>(&mut self) -> &mut Self
    where
        P: Send + Sync + 'static;


    fn register_float_param_binding<T, P>(&mut self) -> &mut Self
    where
        T: Component + 'static,
        P: parameter::FloatParamBinding<T> + Send + Sync + 'static;

    fn register_int_param_binding<T, P>(&mut self) -> &mut Self
    where
        T: Component + 'static,
        P: parameter::IntParamBinding<T> + Send + Sync + 'static;

    fn register_bool_param<P>(&mut self) -> &mut Self
    where
        P: Send + Sync + 'static;

    fn run_auto_installers(&mut self);
}

impl RegistrationAppExt for App {
    fn register_transition<E>(&mut self) -> &mut Self
    where
        E: TransitionEvent + RegisteredTransitionEvent + Clone + 'static + bevy::reflect::TypePath,
        for<'a> <E as Event>::Trigger<'a>: Default,
        for<'a> <<E as TransitionEvent>::ExitEvent as Event>::Trigger<'a>: Default,
        for<'a> <<E as TransitionEvent>::EffectEvent as Event>::Trigger<'a>: Default,
        for<'a> <<E as TransitionEvent>::EntryEvent as Event>::Trigger<'a>: Default,
        <E as TransitionEvent>::Validator: bevy::reflect::TypePath + bevy::reflect::FromReflect + bevy::reflect::GetTypeRegistration + bevy::reflect::Typed 
    {
        if !self.world().contains_resource::<InstalledTransitions>() {
            self.insert_resource(InstalledTransitions(HashSet::new()));
        }
    
        let mut installed = self.world_mut().resource_mut::<InstalledTransitions>();
        let already = !installed.0.insert(TypeId::of::<E>());
        drop(installed);
        if already { return self; }
    
        // Ensure reflect registrations for EventEdge<E> and validator type are present for scene I/O
        self.register_type::<crate::transitions::EventEdge<E>>();
        self.register_type::<<E as TransitionEvent>::Validator>();
    
        self.add_observer(edge_event_listener::<E>)
            .add_observer(crate::transition_observer::<PhaseEvents<E::ExitEvent, E::EffectEvent, E::EntryEvent>>)
            .add_systems(Update, tick_after_event_timers::<E>)
            .add_observer(cancel_pending_event_on_exit::<E>)
            .add_observer(replay_deferred_event::<E>);
        self
    }
    
    fn register_state_component<T>(&mut self) -> &mut Self
    where
        T: Component<Mutability = Mutable> + Clone + Reflect + FromReflect + bevy::reflect::TypePath + bevy::reflect::GetTypeRegistration + bevy::reflect::Typed + 'static 
    {
        if !self.world().contains_resource::<InstalledStateComponents>() {
            self.insert_resource(InstalledStateComponents(HashSet::new()));
        }
        let mut installed = self.world_mut().resource_mut::<InstalledStateComponents>();
        let already = !installed.0.insert(TypeId::of::<T>());
        drop(installed);
        if already { return self; }
    
        // Reflect register inner T and the component wrappers for scene I/O
        self.register_type::<T>();
        self.register_type::<crate::state_component::StateComponent<T>>();
        self.register_type::<crate::state_component::StateInactiveComponent<T>>();
        self.add_observer(crate::prelude::state_component_enter::<T>);
        self.add_observer(crate::prelude::state_component_exit::<T>);
        self.add_observer(crate::prelude::state_inactive_component_enter::<T>);
        self.add_observer(crate::prelude::state_inactive_component_exit::<T>);
        self
    }

    fn register_float_param<P>(&mut self) -> &mut Self
    where
        P: Send + Sync + 'static,
    {
        if !self.world().contains_resource::<InstalledFloatParams>() {
            self.insert_resource(InstalledFloatParams(HashSet::new()));
        }
        let mut installed = self.world_mut().resource_mut::<InstalledFloatParams>();
        let already = !installed.0.insert(TypeId::of::<P>());
        drop(installed);
        if already { return self; }
    
        self.add_systems(Update, parameter::apply_float_param_guards::<P>);
        self
    }

    fn register_int_param<P>(&mut self) -> &mut Self
    where
        P: Send + Sync + 'static,
    {
        if !self.world().contains_resource::<InstalledIntParams>() {
            self.insert_resource(InstalledIntParams(HashSet::new()));
        }
        let mut installed = self.world_mut().resource_mut::<InstalledIntParams>();
        let already = !installed.0.insert(TypeId::of::<P>());
        drop(installed);
        if already { return self; }
    
        self.add_systems(Update, parameter::apply_int_param_guards::<P>);
        self
    }

    fn register_bool_param<P>(&mut self) -> &mut Self
    where
        P: Send + Sync + 'static,
    {
        if !self.world().contains_resource::<InstalledBoolParams>() {
            self.insert_resource(InstalledBoolParams(HashSet::new()));
        }
        let mut installed = self.world_mut().resource_mut::<InstalledBoolParams>();
        let already = !installed.0.insert(TypeId::of::<P>());
        drop(installed);
        if already { return self; }
    
        self.add_systems(Update, parameter::apply_bool_param_guards::<P>);
        self
    }

    fn register_float_param_binding<T, P>(&mut self) -> &mut Self
    where
        T: Component + 'static,
        P: parameter::FloatParamBinding<T> + Send + Sync + 'static,
    {
        if !self.world().contains_resource::<InstalledFloatParamBindings>() {
            self.insert_resource(InstalledFloatParamBindings(HashSet::new()));
        }
        let mut installed = self.world_mut().resource_mut::<InstalledFloatParamBindings>();
        let already = !installed.0.insert((TypeId::of::<T>(), TypeId::of::<P>()));
        drop(installed);
        if already { return self; }
    
        self.add_systems(Update, parameter::sync_float_param::<T, P>);
        self
    }

    fn register_int_param_binding<T, P>(&mut self) -> &mut Self
    where
        T: Component + 'static,
        P: parameter::IntParamBinding<T> + Send + Sync + 'static,
    {
        if !self.world().contains_resource::<InstalledIntParamBindings>() {
            self.insert_resource(InstalledIntParamBindings(HashSet::new()));
        }
        let mut installed = self.world_mut().resource_mut::<InstalledIntParamBindings>();
        let already = !installed.0.insert((TypeId::of::<T>(), TypeId::of::<P>()));
        drop(installed);
        if already { return self; }
    
        self.add_systems(Update, parameter::sync_int_param::<T, P>);
        self
    }

    fn run_auto_installers(&mut self) {
        for installer in inventory::iter::<TransitionInstaller> {
            (installer.install)(self);
        }
        for installer in inventory::iter::<StateInstaller> {
            (installer.install)(self);
        }
        for installer in inventory::iter::<FloatParamInstaller> {
            (installer.install)(self);
        }
        for installer in inventory::iter::<IntParamInstaller> {
            (installer.install)(self);
        }
        for installer in inventory::iter::<BoolParamInstaller> {
            (installer.install)(self);
        }
        for installer in inventory::iter::<FloatParamBindingInstaller> {
            (installer.install)(self);
        }
        for installer in inventory::iter::<IntParamBindingInstaller> {
            (installer.install)(self);
        }
        for installer in inventory::iter::<BoolParamBindingInstaller> {
            (installer.install)(self);
        }
    }
}

// Free-function wrappers for macro-driven installers expecting `fn(&mut App)` symbols
// These delegate to the corresponding `RegistrationAppExt` methods.
pub fn register_transition<E>(app: &mut App)
where
    E: TransitionEvent + RegisteredTransitionEvent + Clone + 'static + bevy::reflect::TypePath,
    for<'a> <E as Event>::Trigger<'a>: Default,
    for<'a> <<E as TransitionEvent>::ExitEvent as Event>::Trigger<'a>: Default,
    for<'a> <<E as TransitionEvent>::EffectEvent as Event>::Trigger<'a>: Default,
    for<'a> <<E as TransitionEvent>::EntryEvent as Event>::Trigger<'a>: Default,
    <E as TransitionEvent>::Validator: bevy::reflect::TypePath + bevy::reflect::FromReflect + bevy::reflect::GetTypeRegistration + bevy::reflect::Typed,
{
    RegistrationAppExt::register_transition::<E>(app);
}

pub fn register_float_param<P>(app: &mut App)
where
    P: Send + Sync + 'static,
{
    RegistrationAppExt::register_float_param::<P>(app);
}

pub fn register_int_param<P>(app: &mut App)
where
    P: Send + Sync + 'static,
{
    RegistrationAppExt::register_int_param::<P>(app);
}

pub fn register_bool_param<P>(app: &mut App)
where
    P: Send + Sync + 'static,
{
    RegistrationAppExt::register_bool_param::<P>(app);
}

pub fn register_float_param_binding<T, P>(app: &mut App)
where
    T: Component + 'static,
    P: parameter::FloatParamBinding<T> + Send + Sync + 'static,
{
    RegistrationAppExt::register_float_param_binding::<T, P>(app);
}

pub fn register_int_param_binding<T, P>(app: &mut App)
where
    T: Component + 'static,
    P: parameter::IntParamBinding<T> + Send + Sync + 'static,
{
    RegistrationAppExt::register_int_param_binding::<T, P>(app);
}

// Deduping for (T, P) bindings
#[derive(Resource, Default)]
pub struct InstalledFloatParamBindings(pub HashSet<(TypeId, TypeId)>);

#[derive(Resource, Default)]
pub struct InstalledIntParamBindings(pub HashSet<(TypeId, TypeId)>);

#[derive(Resource, Default)]
pub struct InstalledBoolParamBindings(pub HashSet<(TypeId, TypeId)>);

/// Function-style plugin to run inventory-based auto-registrations without the full GearboxPlugin
pub fn gearbox_auto_register_plugin(app: &mut App) {
    app.run_auto_installers();
}