use std::fmt;

use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, Visitor};

use super::*;

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "snake_case")]
enum Field {
    SchemaVersion,
    ExportedAtTs,
    ScopeKind,
    Items,
    Checksum,
    #[serde(other)]
    Ignore,
}

pub(super) struct Header {
    pub(super) schema_version: u32,
    pub(super) exported_at_ts: i64,
    pub(super) scope_kind: String,
    pub(super) checksum: String,
}
pub(super) struct ExportSeed<'a, 'b>(pub(super) &'a mut Context<'b>);
impl<'de> DeserializeSeed<'de> for ExportSeed<'_, '_> {
    type Value = Header;
    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<Header, D::Error> {
        decoder.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for ExportSeed<'_, '_> {
    type Value = Header;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("memory_export_v1")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Header, M::Error> {
        let mut schema_version = None;
        let mut exported_at_ts = None;
        let mut scope_kind = None;
        let mut checksum = None;
        let mut items_seen = false;
        loop {
            self.0.budget.begin_value();
            let Some(field) = map.next_key::<Field>()? else {
                break;
            };
            self.0.budget.begin_value();
            match field {
                Field::SchemaVersion => {
                    if schema_version.is_some() {
                        return Err(de::Error::duplicate_field("schema_version"));
                    }
                    let version = map.next_value::<u32>()?;
                    if version != 1 {
                        self.0.fault = Some(UploadError::ExportSchemaUnsupported);
                        return Err(de::Error::custom("memory_import_schema_unsupported"));
                    }
                    schema_version = Some(version);
                }
                Field::ExportedAtTs => {
                    if exported_at_ts.is_some() {
                        return Err(de::Error::duplicate_field("exported_at_ts"));
                    }
                    exported_at_ts = Some(map.next_value()?);
                }
                Field::ScopeKind => {
                    if scope_kind.is_some() {
                        return Err(de::Error::duplicate_field("scope_kind"));
                    }
                    scope_kind = Some(map.next_value()?);
                }
                Field::Checksum => {
                    if checksum.is_some() {
                        return Err(de::Error::duplicate_field("checksum"));
                    }
                    checksum = Some(map.next_value()?);
                }
                Field::Items => {
                    if items_seen {
                        return Err(de::Error::duplicate_field("items"));
                    }
                    items_seen = true;
                    map.next_value_seed(items::ItemsSeed(&mut *self.0))?;
                }
                Field::Ignore => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if !items_seen {
            return Err(de::Error::missing_field("items"));
        }
        Ok(Header {
            schema_version: schema_version
                .ok_or_else(|| de::Error::missing_field("schema_version"))?,
            exported_at_ts: exported_at_ts
                .ok_or_else(|| de::Error::missing_field("exported_at_ts"))?,
            scope_kind: scope_kind.ok_or_else(|| de::Error::missing_field("scope_kind"))?,
            checksum: checksum.ok_or_else(|| de::Error::missing_field("checksum"))?,
        })
    }
}
