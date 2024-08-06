//! Recording and replaying of Boomerang actions.

mod recorder;
mod replayer;

use std::sync::{Arc, Mutex};

use boomerang_tinymap::TinySecondaryMap;
pub use recorder::{inject_recorder, Recorder, RecorderBuilder};

use boomerang::runtime;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct RecordingHeader<'a> {
    name: &'a str,
    start_tag: runtime::Tag,
}

#[derive(serde::Serialize)]
struct Record<'a> {
    name: &'a str,
    key: runtime::ActionKey,
    tag: runtime::Tag,
    value: Option<&'a dyn erased_serde::Serialize>,
}

#[derive(serde::Deserialize)]
struct ReplayRecord<'a>(&'a str, runtime::ActionKey, runtime::Tag);

fn foo<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
    action_map: TinySecondaryMap<runtime::ActionKey, &Arc<Mutex<dyn runtime::BaseActionValues>>>,
) -> Result<(), D::Error> {
    struct Visitor<'a> {
        action_map:
            &'a TinySecondaryMap<runtime::ActionKey, &'a Arc<Mutex<dyn runtime::BaseActionValues>>>,
    }

    impl<'de: 'a, 'a> serde::de::Visitor<'de> for Visitor<'a> {
        type Value = ();
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("tuple struct Record")
        }

        #[inline]
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let name = seq
                .next_element::<&str>()?
                .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
            let key = seq
                .next_element::<runtime::ActionKey>()?
                .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
            let tag = seq
                .next_element::<runtime::Tag>()?
                .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?;

            let action = self
                .action_map
                .get(key)
                .ok_or_else(|| serde::de::Error::custom("ActionKey not found"))?;

            Ok(())
        }
    }

    serde::Deserializer::deserialize_tuple_struct(
        deserializer,
        "Record",
        4usize,
        Visitor {
            action_map: &action_map,
        },
    )
}
