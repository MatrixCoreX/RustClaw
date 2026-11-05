//! One selected main/split document snapshot, shared by API, teaching and task validation.
use std::io;
use std::path::Path;

use claw_core::model_catalog::{ModelCatalog, ModelCatalogDocuments, ModelCatalogError};

use super::{with_locked_files, Target};
use crate::AppState;

pub(crate) fn read_model_catalog(state: &AppState) -> Result<ModelCatalog, ModelCatalogError> {
    let root = &state.skill_rt.workspace_root;
    let main = crate::runtime::active_runtime_config_path(state);
    let parent = main.parent().unwrap_or(root);
    let paths = [
        main.clone(),
        parent.join("image.toml"),
        parent.join("audio.toml"),
        parent.join("video.toml"),
        parent.join("music.toml"),
    ];
    with_locked_files(&paths, || {
        let [main, image, audio, video, music] = &paths;
        Ok(ModelCatalogDocuments {
            config: read(root, main, true)?,
            image: read(root, image, false)?,
            audio: read(root, audio, false)?,
            video: read(root, video, false)?,
            music: read(root, music, false)?,
        }
        .build(root))
    })
    .map_err(|source| read_error(&main, source))?
}

fn read(root: &Path, path: &Path, required: bool) -> Result<toml::Value, ModelCatalogError> {
    let bytes = Target::configured(root, path.into())
        .and_then(|target| target.read())
        .map_err(|source| read_error(path, source))?;
    let Some(bytes) = bytes else {
        if required {
            return Err(read_error(path, io::Error::from(io::ErrorKind::NotFound)));
        }
        return Ok(toml::Value::Table(toml::map::Map::new()));
    };
    let text = String::from_utf8(bytes)
        .map_err(|error| read_error(path, io::Error::new(io::ErrorKind::InvalidData, error)))?;
    toml::from_str(&text).map_err(|source| ModelCatalogError::Parse {
        path: path.to_string_lossy().into_owned(),
        source,
    })
}

fn read_error(path: &Path, source: io::Error) -> ModelCatalogError {
    ModelCatalogError::Read {
        path: path.to_string_lossy().into_owned(),
        source,
    }
}
