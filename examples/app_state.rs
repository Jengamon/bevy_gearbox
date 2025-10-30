// Ported from bevy_gearbox_editor/examples/app_state.rs
// Uses protocol server to enable optional remote editor connection
use bevy::prelude::*;
use bevy_gearbox::prelude::*;
use bevy_gearbox::GearboxPlugin;
use bevy_gearbox_editor::ServerPlugin;

#[derive(States, Component, Debug, Clone, Copy, Eq, PartialEq, Hash, Default)]
enum ExampleState {
    #[default]
    Menu,
    Playing,
    Paused,
}

#[derive(Reflect, Debug, Clone, Copy, Eq, PartialEq, Hash, Default)]
#[reflect(Debug, PartialEq, Default)]
enum AppSignal { 
    #[default]
    Start, 
    Pause, 
    Resume 
}

#[derive(EntityEvent, Clone, Reflect)]
#[reflect(Default)]
#[bevy_gearbox::register_transition]
struct AppEvent {
    #[event_target]
    pub target: Entity,
    pub signal: AppSignal,
}

impl Default for AppEvent {
    fn default() -> Self { Self { target: Entity::PLACEHOLDER, signal: AppSignal::Start } }
}

#[derive(Reflect, Default, Clone)]
#[reflect(Default)]
struct AppEventValidator { expected: AppSignal }

impl bevy_gearbox::transitions::EventValidator<AppEvent> for AppEventValidator {
    fn matches(&self, ev: &AppEvent) -> bool { ev.signal == self.expected }
}

impl bevy_gearbox::TransitionEvent for AppEvent {
    type ExitEvent = bevy_gearbox::NoEvent;
    type EffectEvent = bevy_gearbox::NoEvent;
    type EntryEvent = bevy_gearbox::NoEvent;
    type Validator = AppEventValidator;

    fn to_exit_event(&self) -> Option<Self::ExitEvent> { None }
    fn to_effect_event(&self) -> Option<Self::EffectEvent> { None }
    fn to_entry_event(&self) -> Option<Self::EntryEvent> { None }
}

#[derive(Component)]
struct ChartRoot;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(GearboxPlugin)
        // Egui/Inspector not required for this example; keep the protocol server only
        .add_plugins(ServerPlugin::default())
        .init_state::<ExampleState>()
        .add_state_bridge::<ExampleState>()
        .add_systems(Startup, setup_machine)
        .add_systems(OnEnter(ExampleState::Menu), || println!("ExampleState::Menu"))
        .add_systems(OnEnter(ExampleState::Playing), || println!("ExampleState::Playing"))
        .add_systems(OnEnter(ExampleState::Paused), || println!("ExampleState::Paused"))
        .add_systems(Update, demo_input)
        .add_observer(on_enter_state)
        .register_type::<AppSignal>()
        .register_type::<AppEvent>()
        .register_type::<AppEventValidator>()
        .run();
}

fn setup_machine(mut commands: Commands) {
    // root -> { menu, playing, paused }
    let root = commands.spawn((ChartRoot, Name::new("AppStateMachine"))).id();

    let menu = commands.spawn((SubstateOf(root), ExampleState::Menu, Name::new("Menu"))).id();
    let playing = commands.spawn((SubstateOf(root), ExampleState::Playing, Name::new("Playing"))).id();
    let paused = commands.spawn((SubstateOf(root), ExampleState::Paused, Name::new("Paused"))).id();

    // Initial state is Menu
    commands.entity(root).insert((StateMachine::new(), InitialState(menu)));

    // Edges (one event; per-edge validator selects the signal)
    {
        let edge = commands.spawn((
            Name::new("Start"),
            Source(menu),
            Target(playing),
            EdgeKind::External,
        )).id();
        commands.entity(edge).insert(EventEdge::<AppEvent>::new(Some(AppEventValidator { expected: AppSignal::Start })));
    }
    {
        let edge = commands.spawn((
            Name::new("Pause"),
            Source(playing),
            Target(paused),
            EdgeKind::External,
        )).id();
        commands.entity(edge).insert(EventEdge::<AppEvent>::new(Some(AppEventValidator { expected: AppSignal::Pause })));
    }
    {
        let edge = commands.spawn((
            Name::new("Resume"),
            Source(paused),
            Target(playing),
            EdgeKind::External,
        )).id();
        commands.entity(edge).insert(EventEdge::<AppEvent>::new(Some(AppEventValidator { expected: AppSignal::Resume })));
    }
}

fn demo_input(
    kb: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
) {
    use bevy_gearbox::prelude::GearboxCommandsExt;
    if kb.just_pressed(KeyCode::Digit1) {
        println!("Event: Start (Menu -> Playing)");
        commands.emit_to_chart::<ChartRoot>(move |root| AppEvent { target: root, signal: AppSignal::Start });
    }
    if kb.just_pressed(KeyCode::Digit2) {
        println!("Event: Pause (Playing -> Paused)");
        commands.emit_to_chart::<ChartRoot>(move |root| AppEvent { target: root, signal: AppSignal::Pause });
    }
    if kb.just_pressed(KeyCode::Digit3) {
        println!("Event: Resume (Paused -> Playing)");
        commands.emit_to_chart::<ChartRoot>(move |root| AppEvent { target: root, signal: AppSignal::Resume });
    }
}

fn on_enter_state(
    enter_state: On<EnterState>,
    q_state: Query<&ExampleState>,
) {
    let entity = enter_state.target;

    let Ok(state) = q_state.get(entity) else {
        return;
    };
    println!("Enter gearbox state: {:?}", state);
}


