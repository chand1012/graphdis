//! Collection limits are enforced during deserialization, before model validation.

use alloc::{collections::BTreeMap, string::String, vec::Vec};
use core::{fmt, marker::PhantomData};
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};

pub(super) struct List<T, const N: usize>(pub Vec<T>);

impl<T, const N: usize> Default for List<T, N> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for List<T, N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Items<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Items<T, N> {
            type Value = List<T, N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "at most {N} entries")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while values.len() < N {
                    let Some(value) = seq.next_element()? else {
                        return Ok(List(values));
                    };
                    values.push(value);
                }
                if seq.next_element::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("too many entries"));
                }
                Ok(List(values))
            }
        }
        deserializer.deserialize_seq(Items::<T, N>(PhantomData))
    }
}

pub(super) struct Map<T, const N: usize>(pub BTreeMap<String, T>);

impl<T, const N: usize> Default for Map<T, N> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}

impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for Map<T, N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Entries<T, N> {
            type Value = Map<T, N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "at most {N} named entries")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.len() == N {
                        return Err(de::Error::custom("too many entries"));
                    }
                    let value = map.next_value()?;
                    if values.insert(key, value).is_some() {
                        return Err(de::Error::custom("duplicate key"));
                    }
                }
                Ok(Map(values))
            }
        }
        deserializer.deserialize_map(Entries::<T, N>(PhantomData))
    }
}
