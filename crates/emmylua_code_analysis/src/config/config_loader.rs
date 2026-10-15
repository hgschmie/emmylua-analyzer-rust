use std::{collections::HashSet, path::PathBuf};

use serde_json::Value;

use crate::{config::lua_loader::load_lua_config, read_file_with_encoding};

use super::{Emmyrc, flatten_config::FlattenConfigObject};

pub fn load_configs_raw(config_files: Vec<PathBuf>, partial_emmyrcs: Option<Vec<Value>>) -> Value {
    let mut config_jsons = Vec::new();

    for config_file in config_files {
        log::info!("Loading config file: {:?}", config_file);
        let config_content = match read_file_with_encoding(&config_file, "utf-8") {
            Some(content) => content,
            None => {
                log::error!(
                    "Failed to read config file: {:?}, error: File not found or unreadable",
                    config_file
                );
                continue;
            }
        };

        let config_value = if config_file.extension().and_then(|s| s.to_str()) == Some("lua") {
            match load_lua_config(&config_content) {
                Ok(value) => value,
                Err(e) => {
                    log::error!(
                        "Failed to parse lua config file: {:?}, error: {:?}",
                        config_file,
                        e
                    );
                    continue;
                }
            }
        } else {
            match serde_json::from_str(&config_content) {
                Ok(json) => json,
                Err(e) => {
                    log::error!(
                        "Failed to parse config file: {:?}, error: {:?}",
                        config_file,
                        e
                    );
                    continue;
                }
            }
        };

        config_jsons.push(config_value);
    }

    if let Some(partial_emmyrcs) = partial_emmyrcs {
        for partial_emmyrc in partial_emmyrcs {
            config_jsons.push(partial_emmyrc);
        }
    }

    if config_jsons.is_empty() {
        log::info!("No valid config file found.");
        Value::Object(Default::default())
    } else if config_jsons.len() == 1 {
        let first_config = config_jsons.into_iter().next().unwrap_or_else(|| {
            log::error!("No valid config file found.");
            Value::Object(Default::default())
        });

        let flatten_config = FlattenConfigObject::parse(first_config);
        flatten_config.to_emmyrc()
    } else {
        let merge_config = config_jsons
            .into_iter()
            // Normalize each layer before merging so dotted and nested keys
            // share the same precedence and array-append behavior.
            .map(|item| FlattenConfigObject::parse(item).to_emmyrc())
            .fold(Value::Object(Default::default()), |mut acc, item| {
                merge_values(&mut acc, item);
                acc
            });
        let flatten_config = FlattenConfigObject::parse(merge_config.clone());
        flatten_config.to_emmyrc()
    }
}

pub fn load_configs(config_files: Vec<PathBuf>, partial_emmyrcs: Option<Vec<Value>>) -> Emmyrc {
    let emmyrc_json_value = load_configs_raw(config_files, partial_emmyrcs);
    serde_json::from_value(emmyrc_json_value).unwrap_or_else(|err| {
        log::error!("Failed to parse config: error: {:?}", err);
        Emmyrc::default()
    })
}

fn merge_values(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, overlay_value) in overlay_map {
                match base_map.get_mut(&key) {
                    Some(base_value) => {
                        merge_values(base_value, overlay_value);
                    }
                    None => {
                        base_map.insert(key, overlay_value);
                    }
                }
            }
        }
        (Value::Array(base_array), Value::Array(overlay_array)) => {
            let mut seen = HashSet::new();
            base_array.extend(
                overlay_array
                    .into_iter()
                    .filter(|item| seen.insert(item.clone())),
            );
        }
        (base_slot, overlay_value) => {
            *base_slot = overlay_value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::load_configs_raw;
    use serde_json::{Value, json};

    fn merge(layers: Vec<Value>) -> Value {
        load_configs_raw(Vec::new(), Some(layers))
    }

    #[test]
    fn mixed_key_notation_preserves_array_layers() {
        let dotted = json!({"workspace.ignoreDir": ["vendor"]});
        let nested = json!({"workspace": {"ignoreDir": ["managed-mod"]}});
        assert_eq!(
            merge(vec![dotted.clone(), nested.clone()])["workspace"]["ignoreDir"],
            json!(["vendor", "managed-mod"])
        );
        assert_eq!(
            merge(vec![nested, dotted])["workspace"]["ignoreDir"],
            json!(["managed-mod", "vendor"])
        );
    }

    #[test]
    fn mixed_key_notation_preserves_scalar_precedence() {
        assert_eq!(
            merge(vec![
                json!({"runtime.version": "Lua5.1"}),
                json!({"runtime": {"version": "Lua5.2"}}),
            ])["runtime"]["version"],
            "Lua5.2"
        );
        assert_eq!(
            merge(vec![
                json!({"runtime": {"version": "Lua5.2"}}),
                json!({"runtime.version": "Lua5.3"}),
            ])["runtime"]["version"],
            "Lua5.3"
        );
    }

    #[test]
    fn normalization_retains_other_settings_and_array_entry_objects() {
        let library = json!({"path": "library", "ignoreDir": ["test"]});
        let result = merge(vec![
            json!({"workspace.library": [library.clone()], "diagnostics.disable": ["unused"]}),
            json!({"workspace": {"ignoreDir": ["managed-mod"]}}),
        ]);
        assert_eq!(result["workspace"]["library"], json!([library]));
        assert_eq!(result["diagnostics"]["disable"], json!(["unused"]));
        assert_eq!(result["workspace"]["ignoreDir"], json!(["managed-mod"]));
    }

    #[test]
    fn same_notation_retains_existing_merge_behavior() {
        assert_eq!(
            merge(vec![
                json!({"workspace": {"ignoreDir": ["first"]}}),
                json!({"workspace": {"ignoreDir": ["second", "second"]}}),
            ])["workspace"]["ignoreDir"],
            json!(["first", "second"])
        );
        assert_eq!(merge(vec![]), json!({}));
        assert_eq!(
            merge(vec![json!({"runtime.version": "Lua5.2"})]),
            json!({"runtime": {"version": "Lua5.2"}})
        );
    }
}
