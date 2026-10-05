//! Reject duplicate metadata keys, including nested source/policy objects.
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};
use std::fmt;

pub struct UniqueJson(pub Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueJson;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("JSON with unique keys")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some((key, value)) = entries.next_entry::<String, UniqueJson>()? {
                    if values.insert(key, value.0).is_some() {
                        return Err(de::Error::custom("duplicate metadata key"));
                    }
                }
                Ok(UniqueJson(Value::Object(values)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut entries: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = entries.next_element::<UniqueJson>()? {
                    values.push(value.0);
                }
                Ok(UniqueJson(Value::Array(values)))
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Number(value.into())))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Number(value.into())))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Number::from_f64(value)
                    .map(|number| UniqueJson(Value::Number(number)))
                    .ok_or_else(|| E::custom("nonfinite metadata"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::String(value.to_owned())))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Null))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::UniqueJson;
    #[test]
    fn repeated_nested_source_fields_cannot_hide_policy() {
        for data in [
            r#"{"channels":1,"channels":2}"#,
            r#"{"pcm":{"kind":"source","kind":"mic"}}"#,
        ] {
            assert!(serde_json::from_str::<UniqueJson>(data).is_err());
        }
        assert_eq!(
            serde_json::from_str::<UniqueJson>(r#"{"pcm":{"kind":"mic"},"list":[0,true,null]}"#)
                .unwrap()
                .0["pcm"]["kind"],
            "mic"
        );
    }
}
