use std::env;
use std::fs::read_to_string;
use std::path::Path;

use anyhow::Result;
use log::{info, trace, warn};
use serde::{Deserialize, Serialize};

use crate::colour_spec::ColouringSpec;

const CONFIG_FILENAME: &str = "otail.yaml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtailConfig {
    #[serde(default)]
    pub readonly: bool,
    pub colouring: ColouringSpec,
}

pub struct LocatedConfig {
    pub path: Option<String>,
    pub config: OtailConfig,
}

fn find_config() -> Option<String> {
    if Path::new(CONFIG_FILENAME).exists() {
        return Some(CONFIG_FILENAME.to_owned());
    }

    let path = format!("./{file}", file = CONFIG_FILENAME);
    if Path::new(&path).exists() {
        return Some(path);
    }

    if let Ok(home) = env::var("HOME") {
        let path = format!("{home}/.config/{file}", home = home, file = CONFIG_FILENAME);
        if Path::new(&path).exists() {
            return Some(path);
        }
    }

    None
}

// Get the config. Handle any problems and return a temporary readonly config so otail can
// continue.
//
// TODO Maybe return a message to display if there is a problem.
pub fn load_config() -> LocatedConfig {
    load_config_from(None).unwrap_or_else(|_| LocatedConfig {
        path: None,
        config: OtailConfig {
            readonly: true,
            colouring: ColouringSpec::default(),
        },
    })
}

pub fn load_config_from(config_path: Option<String>) -> Result<LocatedConfig> {
    let path = if let Some(config_path) = config_path {
        if Path::new(&config_path).exists() {
            Some(config_path)
        } else {
            return Err(anyhow::anyhow!(
                "Specified config file does not exist: {}",
                config_path
            ));
        }
    } else {
        find_config()
    };

    let otail_config = if let Some(ref path) = path {
        let config_yaml = match read_to_string(&path) {
            Ok(config_yaml) => config_yaml,
            // TODO Make the resulting config readonly so we don't overwrite the real config
            Err(e) => {
                warn!("Failed to load config from {}: {}", path, e);
                String::new()
            }
        };

        if config_yaml.is_empty() {
            info!("Empty config found, initialising: {}", path);
            OtailConfig {
                readonly: false,
                colouring: ColouringSpec::default(),
            }
        } else {
            info!("Loading config from: {}", path);
            match serde_yaml::from_str(&config_yaml) {
                Ok(otail_config) => otail_config,
                Err(e) => {
                    warn!("Failed to parse config from {}: {}", path, e);
                    OtailConfig {
                        readonly: true,
                        colouring: ColouringSpec::default(),
                    }
                }
            }
        }
    } else {
        OtailConfig {
            readonly: true,
            colouring: ColouringSpec::default(),
        }
    };

    let config = LocatedConfig {
        path,
        config: otail_config,
    };

    Ok(config)
}

// Save the config as best we can.
pub fn maybe_save_config(located_config: &LocatedConfig) {
    if located_config.config.readonly {
        trace!("Not saved readonly config.");
        return;
    }

    if let Some(ref path) = located_config.path {
        if let Err(e) = (|| -> Result<()> {
            trace!("Saving config: {}", path);
            let config_yaml = serde_yaml::to_string(&located_config.config)?;
            std::fs::write(path, config_yaml)?;
            trace!("Config saved.");
            Ok(())
        })() {
            warn!("Failed to save config {}: {}", path, e);
        }
    } else {
        trace!("No file to save config.");
    }
}
