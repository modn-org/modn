use super::{accepted_range, accepts};

#[test]
fn font_size_tokens_need_a_large_enough_length() {
    for value in ["0", "9px", "0.5rem", "0.5em", "50%", "small", "auto", ""] {
        assert!(
            !accepts("--modn-font-size-body-s", value),
            "--modn-font-size-body-s: {value} should be refused"
        );
    }
    for value in ["10px", "13px", "1rem", "0.625rem", "0.625em", "2em"] {
        assert!(
            accepts("--modn-font-size-body-s", value),
            "--modn-font-size-body-s: {value} should be accepted"
        );
    }
}

#[test]
fn font_line_tokens_need_a_large_enough_number_or_length() {
    for value in [
        "0",
        "0.9",
        "9px",
        "1rem",
        "normal",
        "var(--modn-font-line-body-m)",
    ] {
        assert!(
            !accepts("--modn-font-line-body-s", value),
            "--modn-font-line-body-s: {value} should be refused"
        );
    }
    for value in ["1", "1.5", "10px", "18px"] {
        assert!(
            accepts("--modn-font-line-body-s", value),
            "--modn-font-line-body-s: {value} should be accepted"
        );
    }
}

#[test]
fn space_tokens_stay_within_zero_and_the_upper_bound() {
    for value in [
        "-1px",
        "65px",
        "4.01rem",
        "5em",
        "12",
        "auto",
        "var(--modn-space-m)",
    ] {
        assert!(
            !accepts("--modn-space-4xl", value),
            "--modn-space-4xl: {value} should be refused"
        );
    }
    for value in ["0", "0px", "12px", "64px", "1.5rem", "4rem"] {
        assert!(
            accepts("--modn-space-4xl", value),
            "--modn-space-4xl: {value} should be accepted"
        );
    }
}

#[test]
fn disabled_opacity_stays_between_zero_point_four_and_one() {
    for value in [
        "0",
        "0.39",
        "1.01",
        "40%",
        "disabled",
        "var(--modn-opacity-disabled)",
    ] {
        assert!(
            !accepts("--modn-opacity-disabled", value),
            "--modn-opacity-disabled: {value} should be refused"
        );
    }
    for value in ["0.4", "0.75", "1"] {
        assert!(
            accepts("--modn-opacity-disabled", value),
            "--modn-opacity-disabled: {value} should be accepted"
        );
    }
}

#[test]
fn unrestricted_tokens_and_unrelated_custom_properties_are_ignored() {
    for property in ["--modn-color-text", "--modn-radius-m", "--modn-font-body"] {
        assert_eq!(accepted_range(property), None, "{property} is unrestricted");
        assert!(
            accepts(property, "anything"),
            "{property} accepts any value"
        );
    }
}

#[test]
fn the_range_description_names_the_token_group() {
    assert!(
        accepted_range("--modn-font-size-heading-l").is_some_and(|range| range.contains("10px"))
    );
    assert!(accepted_range("--modn-font-line-heading-l")
        .is_some_and(|range| range.contains("unitless")));
    assert!(accepted_range("--modn-space-2xl").is_some_and(|range| range.contains("64px")));
    assert!(accepted_range("--modn-opacity-disabled").is_some_and(|range| range.contains("0.4")));
}
