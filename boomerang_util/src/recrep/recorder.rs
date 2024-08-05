//! The Recorder reactor records PhysicalActions and serializes them to a file for later analysis or replay.

use boomerang::{
    builder::{self, BuilderActionKey, BuilderError, BuilderFqn, BuilderReactionKey},
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

impl<Ser> builder::Reactor for RecorderBuilder<Ser>
where
    Ser: Send + Sync + 'static,
    for<'a> &'a mut Ser: serde::Serializer,
{
    type State = Recorder<Ser>;

    fn build(
        name: &str,
        state: Self::State,
        parent: Option<::boomerang::builder::BuilderReactorKey>,
        env: &mut ::boomerang::builder::EnvBuilder,
    ) -> Result<Self, ::boomerang::builder::BuilderError> {
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

        //builder.finish()

        Ok(reactor)
    }
}

pub struct Recorder<Ser>
where
    Ser: Send + Sync + 'static,
    for<'a> &'a mut Ser: serde::Serializer,
{
    recording_name: String,
    action_fqns: Vec<BuilderFqn>,
    //serializer: Arc<Mutex<dyn erased_serde::Serializer>>,
    //serializer: serde_json::Serializer<std::io::BufWriter<std::fs::File>>,
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
            .map(|n| n.try_into().map_err(Into::into))
            .collect::<Result<Vec<_>, _>>()?;
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

    //#[reaction(reactor = "RecorderBuilder", triggers(shutdown))]
    //fn reaction_shutdown(&mut self, ctx: &mut runtime::Context) {}

    pub fn __build_record<'builder>(
        name: &str,
        reactor: &RecorderBuilder<Ser>,
        builder: &'builder mut ::boomerang::builder::ReactorBuilderState,
    ) -> Result<
        ::boomerang::builder::ReactionBuilderState<'builder>,
        ::boomerang::builder::BuilderError,
    > {
        let __wrapper: Box<dyn::boomerang::runtime::ReactionFn> = Box::new(
            move |ctx: &mut ::boomerang::runtime::Context,
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
                    } = act.as_physical().unwrap();

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
