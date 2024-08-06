//! The Replayer reactor replays a previously serialized recording.
//!
//! Use the `inject_replayer` function to inject the Replayer into an environment.

use std::path::Path;

use boomerang::{
    builder::{
        BuilderActionKey, BuilderError, BuilderFqn, BuilderReactionKey, BuilderReactorKey,
        EnvBuilder, Reactor,
    },
    reaction, runtime,
};

use super::RecordingHeader;

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

    #[reaction(reactor = "ReplayerBuilder<Des>", triggers(startup))]
    fn reaction_startup(&mut self, ctx: &mut runtime::Context) {
        let header: RecordingHeader = serde::de::Deserialize::deserialize(&mut self.deserializer)
            .expect("Failed to deserialize recording header");
        println!("Replaying recording: {}", header.name);
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
