use bevy::{prelude::*, reflect::Reflect};
use bevy::platform::collections::HashSet;

/// A component that holds a set of conditions that must be met for a transition to occur.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Guards {
    /// A set of string identifiers for the guards. For a transition to be allowed,
    /// this set must be empty.
    pub guards: HashSet<String>,
}

impl Guards {
    /// Creates a new, empty set of guards.
    pub fn new() -> Self {
        Self {
            guards: HashSet::new(),
        }
    }

    pub fn init(guards: impl IntoIterator<Item = impl Guard>) -> Self {
        Self {
            guards: guards.into_iter().map(|guard| guard.name()).collect(),
        }
    }

    pub fn has_guard(&self, guard: impl Guard) -> bool {
        self.guards.contains(&guard.name())
    }

    /// Adds a guard to the set. The guard is identified by its name.
    pub fn add_guard(&mut self, guard: impl Guard) {
        self.guards.insert(guard.name());
    }

    /// Removes a guard from the set.
    pub fn remove_guard(&mut self, guard: impl Guard) {
        self.guards.remove(&guard.name());
    }

    /// Checks if the guard conditions are met. Currently, this just checks if the set is empty.
    pub fn check(&self) -> bool {
        self.guards.is_empty()
    }
}

/// A trait for components that act as a guard. Guards are components that can be
/// added or removed from a `Guards` entity to dynamically enable or disable transitions.
pub trait Guard {
    /// Returns the unique string identifier for this guard type.
    fn name(&self) -> String;
}

impl Guard for String {
    fn name(&self) -> String {
        self.clone()
    }
}

impl Guard for &str {
    fn name(&self) -> String {
        self.to_string()
    }
}