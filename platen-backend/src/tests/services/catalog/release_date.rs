use super::*;

#[test]
fn parses_release_date_at_each_supported_precision() {
    assert_eq!(
        parse_release_date("2020"),
        Ok(ReleaseDate {
            year: 2020,
            month: None,
            day: None
        })
    );
    assert_eq!(
        parse_release_date("2020-05"),
        Ok(ReleaseDate {
            year: 2020,
            month: Some(5),
            day: None
        })
    );
    assert_eq!(
        parse_release_date("2020-05-17"),
        Ok(ReleaseDate {
            year: 2020,
            month: Some(5),
            day: Some(17)
        })
    );
}

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
