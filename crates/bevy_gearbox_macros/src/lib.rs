use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Item};

/// Derive macro for simple events that don't need phase-specific payloads.
/// 
/// This macro implements `TransitionEvent` for simple events by setting all
/// associated types to `NoEvent` and returning `None` for all phase methods.
/// 
/// # Example
///  
/// ```rust
/// use bevy::prelude::*;
/// use bevy_gearbox_macros::SimpleTransition;
/// 
/// #[derive(Event, Clone, SimpleTransition)]
/// struct MySimpleEvent;
/// ```
/// 
/// This is equivalent to manually implementing:
/// 
/// ```rust
/// impl TransitionEvent for MySimpleEvent {
///     type ExitEvent = NoEvent;
///     type EffectEvent = NoEvent;
///     type EntryEvent = NoEvent;
///     
///     fn to_exit_event(&self) -> Option<Self::ExitEvent> { None }
///     fn to_effect_event(&self) -> Option<Self::EffectEvent> { None }
///     fn to_entry_event(&self) -> Option<Self::EntryEvent> { None }
/// }
/// ```
#[proc_macro_derive(SimpleTransition)]
pub fn derive_simple_transition(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    
    let expanded = quote! {
        impl bevy_gearbox::TransitionEvent for #name {
            type ExitEvent = bevy_gearbox::NoEvent;
            type EffectEvent = bevy_gearbox::NoEvent;
            type EntryEvent = bevy_gearbox::NoEvent;
            
            fn to_exit_event(&self) -> Option<Self::ExitEvent> { None }
            fn to_effect_event(&self) -> Option<Self::EffectEvent> { None }
            fn to_entry_event(&self) -> Option<Self::EntryEvent> { None }
        }

        impl bevy_gearbox::transitions::RegisteredTransitionEvent for #name {}

        bevy_gearbox::inventory::submit! {
            bevy_gearbox::transitions::TransitionInstaller { install: bevy_gearbox::transitions::register_transition::<#name> }
        }
    };
    
    TokenStream::from(expanded)
}

/// Apply to the event type definition. It implements the marker and submits an installer.
#[proc_macro_attribute]
pub fn register_transition(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let parsed: Item = syn::parse(item.clone()).expect("#[register_transition] must be applied to a type item");
    let name = match &parsed {
        Item::Struct(s) => &s.ident,
        Item::Enum(e) => &e.ident,
        _ => panic!("#[register_transition] supports only structs or enums"),
    };

    let expanded = quote! {
        #parsed

        impl bevy_gearbox::transitions::RegisteredTransitionEvent for #name {}

        bevy_gearbox::inventory::submit! {
            bevy_gearbox::transitions::TransitionInstaller { install: bevy_gearbox::transitions::register_transition::<#name> }
        }
    };
    TokenStream::from(expanded)
}
