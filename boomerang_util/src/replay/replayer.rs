//! The Replayer reactor replays a previously serialized recording.
//!
//! Use the `inject_replayer` function to inject the Replayer into an environment.

use std::path::Path;

use boomerang::builder::{BuilderError, ReactionBuilderState, ReactorBuilderState};
use boomerang::{
    builder::{
        BuilderActionKey, BuilderFqn, BuilderReactionKey, BuilderReactorKey, EnvBuilder, Reactor,
    },
    runtime,
};
use boomerang_tinymap::{TinyMap, TinySecondaryMap};

pub struct ReplayerBuilder<Des>
where
    Des: Send + Sync + 'static,
    for<'a, 'de> &'a mut Des: serde::Deserializer<'de>,
{
    startup: BuilderReactionKey,
    actions: Vec<BuilderActionKey>,
    _phantom: std::marker::PhantomData<Des>,
}

impl<Des> Reactor for ReplayerBuilder<Des>
where
    Des: Send + Sync + 'static,
    for<'a, 'de> &'a mut Des: serde::Deserializer<'de>,
{
    type State = Replayer<Des>;

    fn build(
        name: &str,
        state: Self::State,
        parent: Option<BuilderReactorKey>,
        env: &mut EnvBuilder,
    ) -> Result<Self, BuilderError> {
        // Gather all action keys that were specified by FQNs in Recorder
        let actions = state
            .action_fqns
            .iter()
            .map(|fqn| env.find_physical_action_by_fqn(fqn.clone()))
            .collect::<Result<Vec<_>, _>>()?;

        let mut builder = env.add_reactor(name, parent, state);

        let mut reactor = Self {
            startup: Default::default(),
            actions,
            _phantom: Default::default(),
        };
        reactor.startup =
            Replayer::__build_reaction_startup(stringify!(startup), &reactor, &mut builder)
                .and_then(|b| b.finish())?;

        _ = builder.finish();

        Ok(reactor)
    }
}

pub struct Replayer<Des>
where
    Des: Send + Sync + 'static,
    for<'a, 'de> &'a mut Des: serde::Deserializer<'de>,
{
    /// List of actions to replay.
    action_fqns: Vec<BuilderFqn>,
    /// The deserializer that reads the recording.
    deserializer: Des,
}

impl<Des> Replayer<Des>
where
    Des: Send + Sync + 'static,
    for<'a, 'de> &'a mut Des: serde::Deserializer<'de>,
{
    /// Create a new Replayer with the given action FQNs and deserializer.
    ///
    /// # Arguments
    /// - `action_fqns`: The fully qualified names of the actions to replay.
    /// - `deserializer`: The deserializer that reads the recording.
    pub fn new<N, I>(action_fqns: I, deserializer: Des) -> Result<Self, BuilderError>
    where
        N: TryInto<BuilderFqn>,
        N::Error: Into<BuilderError>,
        I: IntoIterator<Item = N>,
    {
        let action_fqns = action_fqns
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)?;
        Ok(Self {
            action_fqns,
            deserializer,
        })
    }

    pub fn __build_reaction_startup<'builder>(
        name: &str,
        reactor: &ReplayerBuilder<Des>,
        builder: &'builder mut ReactorBuilderState,
    ) -> Result<ReactionBuilderState<'builder>, BuilderError> {
        let __wrapper: runtime::ReactionFn = Box::new(
            move |ctx: &mut runtime::Context,
                  state: &mut dyn runtime::ReactorState,
                  _inputs,
                  _outputs,
                  actions: &mut [&mut runtime::Action]| {
                let state: &mut Self = state
                    .downcast_mut()
                    .expect("Unable to downcast reactor state");

                // First build a Map out of the actions
                let action_map: TinySecondaryMap<_, _> = actions
                    .into_iter()
                    .map(|action| {
                        let act = action.as_physical().expect("Action is not physical");
                        (act.key, &act.store)
                    })
                    .collect();

                serde::Deserializer::deserialize_struct(
                    &mut state.deserializer,
                    "Record",
                    &[&"name", &"key", &"tag", &"value"],
                    visitor,
                );
            },
        );
        let __startup_action = builder.get_startup_action();
        let __shutdown_action = builder.get_shutdown_action();
        let mut reaction = builder
            .add_reaction(&name, __wrapper)
            .with_trigger_action(__startup_action, 0);

        for action_key in reactor.actions.iter() {
            reaction = reaction.with_schedulable_action(*action_key, 0);
        }

        Ok(reaction)
    }
}

/// Inject the Replayer into an environment to replay a recording.
pub fn inject_replayer<'a, P: AsRef<Path>>(
    env_builder: &mut EnvBuilder,
    filename: P,
    name: &str,
    actions: impl Iterator<Item = &'a str>,
) -> Result<(), BuilderError> {
    let file = std::fs::File::open(filename).map_err(BuilderError::from)?;
    let reader = std::io::BufReader::new(file);
    let deserializer = serde_json::Deserializer::from_reader(reader);
    let reactor_key = env_builder.find_reactor_by_fqn(name)?;
    let replayer_state = Replayer::new(actions, deserializer)?;
    let _replayer_builder =
        ReplayerBuilder::build("replayer", replayer_state, Some(reactor_key), env_builder)?;
    Ok(())
}
