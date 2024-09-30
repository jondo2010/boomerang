//! The Recorder reactor records PhysicalActions and serializes them to a file for later analysis or replay.
//!
//! Use the `inject_recorder` function to add a Recorder to an environment builder.

use ::std::convert::TryInto;
use std::{path::Path, sync::Mutex};

use boomerang::prelude::*;

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
