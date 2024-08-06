//! The Recorder reactor records PhysicalActions and serializes them to a file for later analysis or replay.
//!
//! Use the `inject_recorder` function to add a Recorder to an environment builder.

use std::path::Path;

use boomerang::builder::{BuilderError, EnvBuilder};
use boomerang::builder::{ReactionBuilderState, ReactorBuilderState};
use boomerang::{
    builder::{BuilderActionKey, BuilderFqn, BuilderReactionKey, BuilderReactorKey, Reactor},
    reaction,
    runtime::{self},
};
use serde::Serialize;

use super::{Record, RecordingHeader};

pub struct RecorderBuilder<Ser>
where
    Ser: Send + Sync + 'static,
    for<'a> &'a mut Ser: serde::Serializer,
{
    startup: BuilderReactionKey,
    record: BuilderReactionKey,
    actions: Vec<BuilderActionKey>,
    _phantom: std::marker::PhantomData<Ser>,
}

impl<Ser> Reactor for RecorderBuilder<Ser>
where
    Ser: Send + Sync + 'static,
    for<'a> &'a mut Ser: serde::Serializer,
{
    type State = Recorder<Ser>;

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
            record: Default::default(),
            actions,
            _phantom: Default::default(),
        };
        reactor.startup =
            Recorder::__build_reaction_startup(stringify!(startup), &reactor, &mut builder)
                .and_then(|b| b.finish())?;

        reactor.record = Recorder::__build_record(stringify!(record), &reactor, &mut builder)
            .and_then(|b| b.finish())?;

        _ = builder.finish();

        Ok(reactor)
    }
}

pub struct Recorder<Ser>
where
    Ser: Send + Sync + 'static,
    for<'a> &'a mut Ser: serde::Serializer,
{
    /// Label for the recording header
    recording_name: String,
    /// List of actions to record
    action_fqns: Vec<BuilderFqn>,
    /// Serializer to use for recording
    serializer: Ser,
}

impl<Ser> Recorder<Ser>
where
    Ser: Send + Sync + 'static,
    for<'a> &'a mut Ser: serde::Serializer,
{
    /// Create a new Recorder with the given name and action FQNs.
    ///
    /// # Arguments
    /// - recording_name: The name of the recording.
    /// - action_fqns: The fully-qualified names of the actions to record.
    /// - serializer: The serializer that writes the recording.
    pub fn new<N, I>(
        recording_name: &str,
        action_fqns: I,
        serializer: Ser,
    ) -> Result<Self, BuilderError>
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
            recording_name: recording_name.to_owned(),
            action_fqns,
            serializer,
        })
    }

    #[reaction(reactor = "RecorderBuilder<Ser>", triggers(startup))]
    fn reaction_startup(&mut self, ctx: &mut runtime::Context) {
        let header = RecordingHeader {
            name: &self.recording_name,
            start_tag: ctx.get_tag(),
        };
        header.serialize(&mut self.serializer).unwrap();
    }

    #[reaction(reactor = "RecorderBuilder<Ser>", triggers(shutdown))]
    fn reaction_shutdown(&mut self, ctx: &mut runtime::Context) {
        //TODO: Serialize shutdown
    }

    pub fn __build_record<'builder>(
        name: &str,
        reactor: &RecorderBuilder<Ser>,
        builder: &'builder mut ReactorBuilderState,
    ) -> Result<ReactionBuilderState<'builder>, BuilderError> {
        let __wrapper: Box<dyn runtime::ReactionFn> = Box::new(
            move |ctx: &mut runtime::Context,
                  state: &mut dyn runtime::ReactorState,
                  _inputs,
                  _outputs,
                  actions: &mut [&mut runtime::Action]| {
                let state: &mut Self = state
                    .downcast_mut()
                    .expect("Unable to downcast reactor state");

                let tag = ctx.get_tag();

                for act in actions.iter() {
                    let runtime::PhysicalAction {
                        name: action_name,
                        key,
                        values,
                        ..
                    } = act.as_physical().expect("Action is not physical");

                    let base_action_values = &values.lock().expect("lock");
                    let tagged_value = base_action_values.get_serializable_value(tag);

                    let r = Record {
                        name: action_name,
                        key: *key,
                        tag,
                        value: tagged_value,
                    };
                    r.serialize(&mut state.serializer).unwrap();
                }
            },
        );

        let mut reaction = builder.add_reaction(name, __wrapper);
        for action_key in reactor.actions.iter() {
            reaction = reaction.with_trigger_action(*action_key, 0);
        }
        Ok(reaction)
    }
}

/// Injects a recorder into the environment builder to serialize actions to a file.
pub fn inject_recorder<'a, P: AsRef<Path>>(
    env_builder: &mut EnvBuilder,
    filename: P,
    name: &str,
    actions: impl Iterator<Item = &'a str>,
) -> Result<(), anyhow::Error> {
    let file = std::fs::File::open(filename).map_err(BuilderError::from)?;
    let writer = std::io::BufWriter::new(file);
    let serializer = serde_json::Serializer::new(writer);
    let reactor_key = env_builder.find_reactor_by_fqn(name)?;
    let recorder_state = Recorder::new(name, actions, serializer)?;
    let _recorder_builder =
        RecorderBuilder::build("recorder", recorder_state, Some(reactor_key), env_builder)?;
    Ok(())
}
