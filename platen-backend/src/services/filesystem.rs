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
#[path = "../tests/services/filesystem.rs"]
mod tests;
