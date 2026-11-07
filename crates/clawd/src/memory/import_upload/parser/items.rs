use std::fmt;

use serde::de::{self, DeserializeSeed, SeqAccess, Visitor};

use super::*;

pub(super) struct ItemsSeed<'a, 'b>(pub(super) &'a mut Context<'b>);
impl<'de> DeserializeSeed<'de> for ItemsSeed<'_, '_> {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for ItemsSeed<'_, '_> {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("memory_export_items")
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut sequence: S) -> Result<(), S::Error> {
        self.0.item_hash.update(b"[");
        loop {
            self.0.budget.begin_value();
            let Some(item) = sequence.next_element::<MemoryListItem>()? else {
                break;
            };
            if self.0.count > 0 {
                self.0.item_hash.update(b",");
            }
            serde_json::to_writer(HashWriter(&mut self.0.item_hash), &item)
                .map_err(de::Error::custom)?;
            self.0.count = self
                .0
                .count
                .checked_add(1)
                .ok_or_else(|| de::Error::custom("memory_import_upload_item_count_overflow"))?;
            if let Err(error) = (self.0.consumer)(item) {
                self.0.consumer_error = Some(error);
                return Err(de::Error::custom("memory_import_upload_consumer_failed"));
            }
        }
        self.0.item_hash.update(b"]");
        Ok(())
    }
}
