use super::*;

#[test]
fn rejects_invalid_release_dates() {
    for value in [
        "20",
        "2020-5",
        "2020-13",
        "2020-02-30",
        "2020-05-1",
        "2020-05-17T00:00:00Z",
    ] {
        assert!(
            parse_release_date(value).is_err(),
            "{value} should be invalid"
        );
    }
}
