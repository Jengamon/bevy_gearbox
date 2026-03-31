//! Optional bevy_gauge integration — behind the `gauge` feature.
//!
//! Provides:
//! - [`AttributeDerived`] for [`Delay`] — reads `"Cooldown"` from attributes
//! - [`GuardProvider`] for [`AttributeRequirements`]

use std::time::Duration;

use bevy_gauge::prelude::{AttributeDerived, Attributes, AttributeRequirements};

use crate::components::{Delay, GuardProvider};

// ---------------------------------------------------------------------------
// AttributeDerived for Delay
// ---------------------------------------------------------------------------

impl AttributeDerived for Delay {
    fn should_update(&self, attrs: &Attributes) -> bool {
        let cooldown = attrs.value("Cooldown");
        cooldown > 0.0 && (self.duration.as_secs_f32() - cooldown).abs() > f32::EPSILON
    }

    fn update_from_attributes(&mut self, attrs: &Attributes) {
        let cooldown = attrs.value("Cooldown");
        if cooldown > 0.0 {
            self.duration = Duration::from_secs_f32(cooldown);
        }
    }
}

bevy_gauge::register_derived!(Delay);

// ---------------------------------------------------------------------------
// GuardProvider for AttributeRequirements
// ---------------------------------------------------------------------------

impl GuardProvider for AttributeRequirements {
    fn guard_name() -> &'static str {
        "stat_req_unmet"
    }
}
