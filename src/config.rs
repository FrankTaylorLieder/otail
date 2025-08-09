use std::env;
use std::fs::read_to_string;
use std::path::Path;

use anyhow::Result;
use log::{info, trace};
use serde::{Deserialize, Serialize};

use crate::colour_spec::ColouringSpec;

const CONFIG_FILENAME: &str = "otail.yaml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtailConfig {
    pub colouring: ColouringSpec,
}

pub struct LocatedConfig {
    pub path: String,
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

pub fn load_config() -> Result<Option<LocatedConfig>> {
    let path = find_config();

    if let Some(path) = path {
        let config_yaml = read_to_string(&path)?;

        let config = if config_yaml.is_empty() {
            info!("Empty config found, initialising: {}", path);
            OtailConfig {
                colouring: ColouringSpec::new(),
            }
        } else {
            info!("Loading config from: {}", path);
            serde_yaml::from_str(&config_yaml)?
        };

        let config = LocatedConfig {
            path: path.clone(),
            config,
        };

        return Ok(Some(config));
    }

    return Ok(None);
}

pub fn save_config(located_config: &LocatedConfig) -> Result<()> {
    trace!("Saving config: {}", located_config.path);
    let config_yaml = serde_yaml::to_string(&located_config.config)?;
    std::fs::write(&located_config.path, config_yaml)?;

    Ok(())
}
