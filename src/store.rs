use std::{
    collections::HashMap,
    error::Error,
    fs::OpenOptions,
    io::{Read, Write},
};

use crate::GuildsConfig;

pub fn store_config(config: GuildsConfig) -> Result<(), Box<dyn Error>> {
    let to_store = serde_json::to_string(&config)?;
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("store.json")?;
    f.write(&to_store.into_bytes())?;
    Ok(())
}

pub fn retrieve_config() -> Result<GuildsConfig, Box<dyn Error>> {
    let Ok(mut f) = OpenOptions::new().read(true).open("store.json") else {
        return Ok(HashMap::new());
    };
    let mut result = Vec::new();
    f.read_to_end(&mut result)?;
    Ok(
        serde_json::from_slice::<GuildsConfig>(&result[..]).unwrap_or_else(|_| {
            eprintln!("Could not read json from file, falling back to empty vec");
            HashMap::new()
        }),
    )
}
