use figment::{
    Figment,
    providers::{Env, Format, Toml},
};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Config {
    pub(crate) database_url: String,
    pub(crate) bind_address: String,
    pub(crate) antra_password: String,
    pub(crate) antra_username: String,
    pub(crate) tidal_client_id: String,
    pub(crate) tidal_client_secret: String,
    #[serde(deserialize_with = "deserialize_tidal_country_code")]
    pub(crate) tidal_country_code: String,
    pub(crate) music_dir: String,
}

fn deserialize_tidal_country_code<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let country_code = String::deserialize(deserializer)?;
    if country_code.len() != 2 || !country_code.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(serde::de::Error::custom(
            "tidal_country_code must contain exactly 2 uppercase ASCII letters",
        ));
    }
    Ok(country_code)
}

impl Config {
    pub(crate) fn load() -> Result<Self, figment::Error> {
        Figment::new()
            .merge(Toml::file("platen.toml"))
            .merge(Env::prefixed("PLATEN_"))
            .extract()
    }
}

#[cfg(test)]
#[path = "tests/config.rs"]
mod tests;
