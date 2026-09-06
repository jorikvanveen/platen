use figment::{
    Figment,
    providers::{Format, Serialized, Toml},
};

use super::Config;

fn base_config() -> Figment {
    Figment::new().merge(Toml::string(
        r#"
        database_url = "sqlite::memory:"
        bind_address = "127.0.0.1:0"
        antra_password = "test-password"
        antra_username = "test-user"
        tidal_client_id = "test-client"
        tidal_client_secret = "test-secret"
        music_dir = "test-music"
        "#,
    ))
}

#[test]
fn tidal_country_code_is_required() {
    let error = base_config().extract::<Config>().err().unwrap();

    assert!(error.to_string().contains("tidal_country_code"));
    assert!(error.to_string().contains("missing field"));
}

#[test]
fn tidal_country_code_accepts_exactly_two_uppercase_ascii_letters() {
    for country_code in ["NL", "US", "GB", "ZZ"] {
        let config = base_config()
            .merge(Toml::string(&format!(
                "tidal_country_code = {country_code:?}"
            )))
            .extract::<Config>()
            .unwrap();

        assert_eq!(config.tidal_country_code, country_code);
    }
}

#[test]
fn tidal_country_code_rejects_invalid_strings() {
    for country_code in [
        "", "N", "NLD", "nl", "Nl", "nL", " NL", "NL ", "N L", "N\n", "N\t", "N1", "12", "N-", "É",
        "ÉL", "ＮＬ",
    ] {
        let error = base_config()
            .merge(Serialized::default("tidal_country_code", country_code))
            .extract::<Config>()
            .err()
            .unwrap();

        assert!(
            error
                .to_string()
                .contains("exactly 2 uppercase ASCII letters"),
            "{country_code:?}: {error}"
        );
    }
}

#[test]
fn tidal_country_code_rejects_non_string_values() {
    for value in ["12", "true", "[\"NL\"]"] {
        assert!(
            base_config()
                .merge(Toml::string(&format!("tidal_country_code = {value}")))
                .extract::<Config>()
                .is_err(),
            "{value}"
        );
    }
}

#[test]
fn later_provider_overrides_toml_country_code() {
    let config = base_config()
        .merge(Toml::string("tidal_country_code = \"NL\""))
        .merge(Serialized::default("tidal_country_code", "US"))
        .extract::<Config>()
        .unwrap();

    assert_eq!(config.tidal_country_code, "US");
    assert_eq!(config.bind_address, "127.0.0.1:0");
}

#[test]
fn invalid_provider_override_does_not_fall_back_to_toml() {
    let error = base_config()
        .merge(Toml::string("tidal_country_code = \"NL\""))
        .merge(Serialized::default("tidal_country_code", "us"))
        .extract::<Config>()
        .err()
        .unwrap();

    assert!(
        error
            .to_string()
            .contains("exactly 2 uppercase ASCII letters")
    );
}

#[test]
fn valid_provider_override_replaces_invalid_toml_country_code() {
    let config = base_config()
        .merge(Toml::string("tidal_country_code = \"nl\""))
        .merge(Serialized::default("tidal_country_code", "NL"))
        .extract::<Config>()
        .unwrap();

    assert_eq!(config.tidal_country_code, "NL");
}
