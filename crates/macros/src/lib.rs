use proc_macro::TokenStream;
use quote::quote;
use syn::{Item, Fields};

/// Attribute macro that turns a struct into a gearbox transition message.
///
/// The struct must have a field named `machine` of type `Entity`.
/// This generates:
/// - `#[derive(Message, Clone)]` on the struct
/// - `impl GearboxMessage` with `type Validator = AcceptAll`
/// - An inventory auto-registration entry
///
/// # Example
///
/// ```rust
/// use bevy::prelude::*;
///
/// #[gearbox_message]
/// struct Attack {
///     machine: Entity,
///     damage: f32,
/// }
/// ```
#[proc_macro_attribute]
pub fn gearbox_message(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let parsed: Item = syn::parse(item)
        .expect("#[gearbox_message] must be applied to a struct");

    let name = match &parsed {
        Item::Struct(s) => {
            // Verify it has a `machine` field
            match &s.fields {
                Fields::Named(fields) => {
                    let has_machine = fields.named.iter().any(|f| {
                        f.ident.as_ref().map(|i| i == "machine").unwrap_or(false)
                    });
                    if !has_machine {
                        return syn::Error::new_spanned(
                            &s.ident,
                            "#[gearbox_message] requires a `machine: Entity` field",
                        )
                        .to_compile_error()
                        .into();
                    }
                }
                _ => {
                    return syn::Error::new_spanned(
                        &s.ident,
                        "#[gearbox_message] can only be applied to structs with named fields",
                    )
                    .to_compile_error()
                    .into();
                }
            }
            s.ident.clone()
        }
        _ => panic!("#[gearbox_message] can only be applied to structs"),
    };

    let expanded = quote! {
        #[derive(bevy::prelude::Message, Clone)]
        #parsed

        impl bevy_gearbox::GearboxMessage for #name {
            type Validator = bevy_gearbox::AcceptAll;

            fn machine(&self) -> bevy::prelude::Entity {
                self.machine
            }
        }

        bevy_gearbox::inventory::submit! {
            bevy_gearbox::registration::TransitionInstaller {
                install: bevy_gearbox::registration::register_transition::<#name>
            }
        }
    };

    TokenStream::from(expanded)
}

/// Attribute macro to auto-register a transition message type via inventory.
///
/// Use this on types that already implement `GearboxMessage` manually
/// (e.g. with a custom validator). The macro just submits the inventory installer.
///
/// # Example
///
/// ```rust
/// #[transition_message]
/// #[derive(Message, Clone)]
/// struct Attack {
///     machine: Entity,
///     damage: f32,
/// }
///
/// impl GearboxMessage for Attack {
///     type Validator = MyCustomValidator;
///     fn machine(&self) -> Entity { self.machine }
/// }
/// ```
#[proc_macro_attribute]
pub fn transition_message(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let parsed: Item = syn::parse(item).expect("#[transition_message] must be applied to a type item");
    let name = match &parsed {
        Item::Struct(s) => &s.ident,
        Item::Enum(e) => &e.ident,
        _ => panic!("#[transition_message] supports only structs or enums"),
    };

    let expanded = quote! {
        #parsed

        bevy_gearbox::inventory::submit! {
            bevy_gearbox::registration::TransitionInstaller {
                install: bevy_gearbox::registration::register_transition::<#name>
            }
        }
    };
    TokenStream::from(expanded)
}

/// Attribute macro to auto-register a state component type via inventory.
///
/// # Example
///
/// ```rust
/// #[state_component]
/// #[derive(Component, Reflect, Clone)]
/// struct MyFlag;
/// ```
#[proc_macro_attribute]
pub fn state_component(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let parsed: Item = syn::parse(item).expect("#[state_component] must be applied to a type item");
    let name = match &parsed {
        Item::Struct(s) => &s.ident,
        Item::Enum(e) => &e.ident,
        _ => panic!("#[state_component] supports only structs or enums"),
    };

    let expanded = quote! {
        #parsed

        bevy_gearbox::inventory::submit! {
            bevy_gearbox::registration::StateInstaller {
                install: bevy_gearbox::registration::register_state_component::<#name>
            }
        }
    };
    TokenStream::from(expanded)
}

/// Attribute macro to auto-register a Bevy `States` bridge via inventory.
///
/// # Example
///
/// ```rust
/// #[state_bridge]
/// #[derive(States, Component, Default, Clone, Hash, PartialEq, Eq, Debug)]
/// enum GameState { #[default] Menu, Playing }
/// ```
#[proc_macro_attribute]
pub fn state_bridge(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let parsed: Item = syn::parse(item).expect("#[state_bridge] must be applied to a type item");
    let name = match &parsed {
        Item::Struct(s) => &s.ident,
        Item::Enum(e) => &e.ident,
        _ => panic!("#[state_bridge] supports only structs or enums"),
    };

    let expanded = quote! {
        #parsed

        bevy_gearbox::inventory::submit! {
            bevy_gearbox::registration::StateBridgeInstaller {
                install: bevy_gearbox::registration::register_state_bridge::<#name>
            }
        }
    };
    TokenStream::from(expanded)
}

