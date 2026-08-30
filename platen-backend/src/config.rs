use figment::{
    Figment,
    providers::{Env, Format, Toml},
};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct Config {
    pub(crate) database_url: String,
    pub(crate) bind_address: String,
    pub(crate) antra_password: String,
    pub(crate) antra_username: String,
    pub(crate) tidal_client_id: String,
    pub(crate) tidal_client_secret: String,
    pub(crate) music_dir: String,
}

impl Config {
    pub(crate) fn load() -> Result<Self, figment::Error> {
        Figment::new()
            .merge(Toml::file("platen.toml"))
            .merge(Env::prefixed("PLATEN_").split("__"))
            .extract()
    }
}
