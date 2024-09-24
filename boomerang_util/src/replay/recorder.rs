//! The Recorder reactor records PhysicalActions and serializes them to a file for later analysis or replay.
//!
//! Use the `inject_recorder` function to add a Recorder to an environment builder.

use ::std::convert::TryInto;
use std::{array, path::Path, sync::Mutex};

use boomerang::{
    builder::{prelude::*, BuilderActionKey, BuilderReactorKey, PhysicalActionKey, ReactorField},
    runtime, Reaction,
};

#[cfg(feature = "disable")]
mod old {
    impl<Ser> Reactor for RecorderBuilder<Ser>
    where
        Ser: Send + Sync + 'static,
        for<'a> &'a mut Ser: serde::Serializer,
    {
        type State = RecState<Ser>;

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
                RecState::__build_reaction_startup(stringify!(startup), &reactor, &mut builder)
                    .and_then(|b| b.finish())?;

            reactor.record = RecState::__build_record(stringify!(record), &reactor, &mut builder)
                .and_then(|b| b.finish())?;

            _ = builder.finish();

            Ok(reactor)
        }
    }

    impl Foo {
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
            let __wrapper: runtime::ReactionFn = Box::new(
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
                            store: values,
                            ..
                        } = act.as_physical().expect("Action is not physical");

                        let action_store = &values.lock().expect("lock");

                        runtime::SerializableActionStore::serialize_value(
                            &mut *action_store,
                            tag,
                            &mut state.serializer,
                        );

                        let tagged_value = action_store.serialize_value(tag);

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
}

struct ArrayBuilder(serde_arrow::ArrayBuilder);
#[allow(unsafe_code)]
unsafe impl Send for ArrayBuilder {}

struct RecState {
    /// Label for the recording header
    recording_name: String,
    /// Name of the action to record
    action_fqn: BuilderFqn,
    /// Array builder
    array_builder: Option<Mutex<ArrayBuilder>>,
    record_count: usize,
}

impl RecState {
    /// Create a new Recorder with the given name and action FQN.
    ///
    /// # Arguments
    /// - recording_name: The name of the recording.
    /// - action_fqn: The fully-qualified name of the action to record.
    pub fn new(recording_name: &str, action_fqn: BuilderFqn) -> Result<Self, BuilderError> {
        Ok(Self {
            recording_name: recording_name.to_owned(),
            action_fqn,
            array_builder: None,
            record_count: 0,
        })
    }
}

/// Injects a recorder `Reaction` into the `Reactor` next to the given `Action`.
fn inject_recorder_reaction(
    env_builder: &mut EnvBuilder,
    action_fqn: BuilderFqn,
) -> Result<(), BuilderError> {
    tracing::info!("Tracing Physical Action: {:?}", action_fqn);
    let action_key = env_builder.find_physical_action_by_fqn(action_fqn.clone())?;
    let action = env_builder.get_action(action_key)?;
    let mut parent_builder = env_builder.get_reactor_builder(action.get_reactor_key())?;

    let __trigger_inner = {
        let mut rec_state = RecState::new("recorder", action_fqn)?;
        Box::new(
            move |ctx: &mut runtime::Context,
                  _state: &mut dyn runtime::ReactorState,
                  _ports: &[runtime::PortRef],
                  _ports_mut: &mut [runtime::PortRefMut],
                  actions: &mut [&mut runtime::Action]| {
                let [action]: &mut [&mut runtime::Action; 1usize] = actions
                    .try_into()
                    .expect("Unable to destructure actions for reaction");

                let action = action.as_physical().expect("Action is not physical");

                let builder = rec_state.array_builder.get_or_insert_with(|| {
                    Mutex::new(ArrayBuilder(
                        action.new_builder().expect("Failed to create builder"),
                    ))
                });

                let lock = builder.get_mut().expect("Failed to lock builder");
                action
                    .build_value_at(&mut lock.0, ctx.get_tag())
                    .expect("Failed to build value");

                rec_state.record_count += 1;

                if rec_state.record_count % 10 == 0 {
                    let batch = lock.0.to_record_batch().unwrap();
                    arrow::util::pretty::print_batches(&[batch]).unwrap();
                }
            },
        )
    };

    parent_builder
        .add_reaction("recorder", __trigger_inner)
        .with_action(
            action_key,
            0,
            boomerang::builder::TriggerMode::TriggersAndUses,
        )?
        .finish()?;
    parent_builder.finish()?;
    Ok(())
}

/// Injects a recorder into the environment builder to serialize actions to a file.
pub fn inject_recorder<'a, P: AsRef<Path>>(
    env_builder: &mut EnvBuilder,
    filename: P,
    name: &str,
    actions: impl Iterator<Item = &'a str>,
) -> Result<(), anyhow::Error> {
    //let file = std::fs::File::open(filename).map_err(BuilderError::from)?;
    //let writer = std::io::BufWriter::new(file);
    //let serializer = serde_json::Serializer::new(writer);

    for action in actions.into_iter() {
        inject_recorder_reaction(env_builder, action.try_into()?)?;
    }

    //let reactor_key = env_builder.find_reactor_by_fqn(name)?;
    Ok(())
}
