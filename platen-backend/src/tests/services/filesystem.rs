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
