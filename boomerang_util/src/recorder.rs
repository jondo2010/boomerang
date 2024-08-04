//! The Recorder reactor records PhysicalActions and serializes them to a file for later analysis or replay.

use serde::{ser::SerializeStruct, Serializer};

use boomerang::{
    builder::{self, BuilderActionKey, BuilderFqn, BuilderReactionKey},
    reaction, runtime,
};

pub struct RecorderBuilder {
    startup: BuilderReactionKey,
    record: BuilderReactionKey,
    actions: Vec<BuilderActionKey>,
}

impl builder::Reactor for RecorderBuilder {
    type State = Recorder;
    fn build<'__builder>(
        name: &str,
        state: Self::State,
        parent: Option<::boomerang::builder::BuilderReactorKey>,
        env: &'__builder mut ::boomerang::builder::EnvBuilder,
    ) -> Result<Self, ::boomerang::builder::BuilderError> {
        // Gather all action keys that were specified by FQNs in Recorder
        let actions = state
            .action_fqns
            .iter()
            .map(|fqn| env.find_physical_action_by_fqn(fqn.clone()))
            .collect::<Result<Vec<_>, _>>()?;

        let mut __builder = env.add_reactor(name, parent, state);

        let mut reactor = Self {
            startup: Default::default(),
            record: Default::default(),
            actions,
        };
        reactor.startup =
            Recorder::__build_reaction_startup(stringify!(startup), &reactor, &mut __builder)
                .and_then(|b| b.finish())?;

        reactor.record = Recorder::__build_record(stringify!(record), &reactor, &mut __builder)
            .and_then(|b| b.finish())?;

        Ok(reactor)
    }
}

pub struct Recorder {
    action_fqns: Vec<BuilderFqn>,
    //serializer: Arc<Mutex<dyn erased_serde::Serializer>>,
    serializer: serde_json::Serializer<std::io::BufWriter<std::fs::File>>,
}

impl Recorder {
    pub fn new<N: Into<BuilderFqn>>(action_fqns: impl IntoIterator<Item = N>) -> Recorder {
        let file = std::fs::File::create("recording.json").unwrap();
        let writer = std::io::BufWriter::new(file);

        let serializer = serde_json::Serializer::new(writer);

        let action_fqns = action_fqns.into_iter().map(|n| n.into()).collect();
        Self {
            action_fqns,
            serializer,
        }
    }

    #[reaction(reactor = "RecorderBuilder", triggers(startup))]
    fn reaction_startup(&mut self, ctx: &mut runtime::Context) {}

    pub fn __build_record<'builder>(
        name: &str,
        reactor: &RecorderBuilder,
        builder: &'builder mut ::boomerang::builder::ReactorBuilderState,
    ) -> Result<
        ::boomerang::builder::ReactionBuilderState<'builder>,
        ::boomerang::builder::BuilderError,
    > {
        let __wrapper: Box<dyn ::boomerang::runtime::ReactionFn> = Box::new(
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

                    let mut struct_se = state
                        .serializer
                        .serialize_struct("PhysicalAction", 3)
                        .unwrap();
                    struct_se.serialize_field("name", action_name).unwrap();
                    struct_se.serialize_field("key", &key).unwrap();
                    struct_se.serialize_field("tag", &tag).unwrap();
                    struct_se.serialize_field("value", &tagged_value).unwrap();
                    struct_se.end().unwrap();
                }
            },
        );

        let mut reaction = builder.add_reaction(&name, __wrapper);
        for action_key in reactor.actions.iter() {
            reaction = reaction.with_trigger_action(*action_key, 0);
        }
        Ok(reaction)
    }
}
