pub(crate) fn filesystem_safe_component(value: &str, fallback: &str) -> String {
    let mut sanitized = String::new();
    for character in value.chars() {
        match character {
            '/' | '\\' => {
                if !sanitized.ends_with(" - ") {
                    sanitized.push_str(" - ");
                }
            }
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => sanitized.push('_'),
            character if character.is_control() => sanitized.push('_'),
            character => sanitized.push(character),
        }
    }

    let sanitized = sanitized.trim().trim_end_matches([' ', '.']);
    if sanitized.is_empty() {
        fallback.to_owned()
    } else {
        sanitized.to_owned()
    }
}

pub(crate) fn album_location(primary_artist: &str, album_title: &str, release_year: i32) -> String {
    let artist_directory = filesystem_safe_component(primary_artist, "Unknown artist");
    let album_directory = format!(
        "{} ({release_year})",
        filesystem_safe_component(album_title, "Unknown album")
    );
    format!("{artist_directory}/{album_directory}")
}

#[cfg(test)]
mod tests {
    use super::{album_location, filesystem_safe_component};

    #[test]
    fn sanitizes_path_separators_without_creating_directories() {
        assert_eq!(
            filesystem_safe_component("Speakerboxxx/The Love Below", "Unknown album"),
            "Speakerboxxx - The Love Below"
        );
        assert_eq!(
            filesystem_safe_component("A\\B/C", "Unknown album"),
            "A - B - C"
        );

        let location = album_location("AC/DC", "A\\B/C", 2026);
        assert_eq!(location, "AC - DC/A - B - C (2026)");
        assert_eq!(location.split('/').count(), 2);
    }

    #[test]
    fn replaces_invalid_characters_and_uses_the_requested_fallback() {
        assert_eq!(
            filesystem_safe_component("A:B* C? D\" E< F> G|", "Unknown album"),
            "A_B_ C_ D_ E_ F_ G_"
        );
        assert_eq!(
            filesystem_safe_component("...   ", "Unknown album"),
            "Unknown album"
        );
        assert_eq!(
            filesystem_safe_component("   ", "Unknown artist"),
            "Unknown artist"
        );
    }
}
