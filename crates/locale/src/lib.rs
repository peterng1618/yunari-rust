//! The words and separators a theme's language spells.
//!
//! Ported from `renderer-core/scene/datetime/names.ts`. A table of English names
//! cannot be localized, and its short forms cannot be truncated from the long
//! ones: German's short weekday is `Do`, not `Don`, and Traditional Chinese's
//! `週四` is no prefix of `星期四`. The source asks the platform (`Intl`, which
//! is CLDR); a native surface reads the same CLDR data through ICU4X, so both
//! surfaces spell a language the same way. A language ICU4X cannot render reads
//! English rather than failing a paint.

use icu_calendar::{Date, Gregorian};
use icu_datetime::fieldsets::enums::{DateFieldSet, TimeFieldSet};
use icu_datetime::input::Time;
use icu_datetime::pattern::{
    DateTimePattern, DayPeriodNameLength, FixedCalendarDateTimeNames, MonthNameLength,
    WeekdayNameLength,
};
use icu_decimal::DecimalFormatter;
use icu_decimal::input::Decimal;
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu_locale_core::Locale as IcuLocale;
use writeable::TryWriteable;

/// What the English fallback spells when ICU4X is refused a language mid-paint.
const MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const WEEKDAYS_LONG: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

const WEEKDAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// A language, and the words it spells.
#[derive(Debug, Clone)]
pub struct Locale {
    locale: Option<IcuLocale>,
}

impl Locale {
    /// The language a theme declares. An absent or unusable tag reads English,
    /// as the source's default does; validation is what refuses a bad tag.
    pub fn of(language: Option<&str>) -> Self {
        let tag = language.filter(|tag| !tag.is_empty()).unwrap_or("en");
        Self {
            locale: tag.parse::<IcuLocale>().ok().filter(has_data),
        }
    }

    /// The month's name, long or abbreviated, as the language spells it.
    pub fn month(&self, year: i32, month: u32, day: u32, long: bool) -> String {
        let index = month.saturating_sub(1).min(11) as usize;
        let fallback = if long {
            MONTHS_LONG[index]
        } else {
            MONTHS_SHORT[index]
        };
        self.locale
            .as_ref()
            .and_then(|locale| month_name(locale, year, month, day, long))
            .unwrap_or_else(|| fallback.to_owned())
    }

    /// The weekday's name, long or abbreviated.
    pub fn weekday(&self, year: i32, month: u32, day: u32, long: bool) -> String {
        let index = weekday_of(year, month, day).unwrap_or(0) as usize;
        let fallback = if long {
            WEEKDAYS_LONG[index]
        } else {
            WEEKDAYS_SHORT[index]
        };
        self.locale
            .as_ref()
            .and_then(|locale| weekday_name(locale, year, month, day, long))
            .unwrap_or_else(|| fallback.to_owned())
    }

    /// The language's own half of the day: `AM`/`PM` where CLDR gives them,
    /// `午前`/`午後` where it does not.
    pub fn day_period(&self, hour: u32) -> String {
        let fallback = if hour < 12 { "AM" } else { "PM" };
        self.locale
            .as_ref()
            .and_then(|locale| day_period_name(locale, hour))
            .unwrap_or_else(|| fallback.to_owned())
    }

    /// The reading in the theme's own digits and separator. `ascii` is the
    /// figure already rounded and written with `.` decimals.
    pub fn number(&self, ascii: &str) -> String {
        let Some(locale) = &self.locale else {
            return ascii.to_owned();
        };
        let mut options = DecimalFormatterOptions::default();
        options.grouping_strategy = Some(GroupingStrategy::Never);
        let Ok(formatter) = DecimalFormatter::try_new(locale.clone().into(), options) else {
            return ascii.to_owned();
        };
        let Ok(decimal) = ascii.parse::<Decimal>() else {
            return ascii.to_owned();
        };
        formatter.format_to_string(&decimal)
    }
}

/// Whether this build has CLDR data for the language. A well-formed tag with
/// no data resolves to root, whose month names are `M09`-style placeholders.
fn has_data(locale: &IcuLocale) -> bool {
    month_name(locale, 2026, 9, 24, true).is_some_and(|name| name != PLACEHOLDER_MONTH)
}

/// Root's spelled-out month, which no language uses as a real name.
const PLACEHOLDER_MONTH: &str = "M09";

fn month_name(locale: &IcuLocale, year: i32, month: u32, day: u32, long: bool) -> Option<String> {
    let mut names = date_names(locale)?;
    let length = if long {
        MonthNameLength::StandaloneWide
    } else {
        MonthNameLength::StandaloneAbbreviated
    };
    names.include_month_names(length).ok()?;
    date_name(&names, year, month, day, if long { "LLLL" } else { "LLL" })
}

fn weekday_name(locale: &IcuLocale, year: i32, month: u32, day: u32, long: bool) -> Option<String> {
    let mut names = date_names(locale)?;
    let length = if long {
        WeekdayNameLength::StandaloneWide
    } else {
        WeekdayNameLength::StandaloneAbbreviated
    };
    names.include_weekday_names(length).ok()?;
    date_name(&names, year, month, day, if long { "cccc" } else { "ccc" })
}

fn day_period_name(locale: &IcuLocale, hour: u32) -> Option<String> {
    let mut names: FixedCalendarDateTimeNames<Gregorian, TimeFieldSet> =
        FixedCalendarDateTimeNames::try_new(locale.clone().into()).ok()?;
    names
        .include_day_period_names(DayPeriodNameLength::Abbreviated)
        .ok()?;
    let time = Time::try_new(u8::try_from(hour).ok()?, 0, 0, 0).ok()?;
    let pattern = "a".parse::<DateTimePattern>().ok()?;
    names
        .with_pattern_unchecked(&pattern)
        .format(&time)
        .try_write_to_string()
        .ok()
        .map(|text| text.into_owned())
}

fn date_names(locale: &IcuLocale) -> Option<FixedCalendarDateTimeNames<Gregorian, DateFieldSet>> {
    FixedCalendarDateTimeNames::try_new(locale.clone().into()).ok()
}

fn date_name(
    names: &FixedCalendarDateTimeNames<Gregorian, DateFieldSet>,
    year: i32,
    month: u32,
    day: u32,
    format: &str,
) -> Option<String> {
    let date =
        Date::try_new_gregorian(year, u8::try_from(month).ok()?, u8::try_from(day).ok()?).ok()?;
    let pattern = format.parse::<DateTimePattern>().ok()?;
    names
        .with_pattern_unchecked(&pattern)
        .format(&date)
        .try_write_to_string()
        .ok()
        .map(|text| text.into_owned())
}

/// Whether this build renders the tag. A well-formed tag with no CLDR data
/// resolves to root, whose month names are placeholders — a setting that would
/// appear to do nothing, so it is refused rather than accepted.
pub fn is_language(language: &str) -> bool {
    let shaped = !language.is_empty()
        && language.len() <= 64
        && language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    shaped
        && language
            .parse::<IcuLocale>()
            .ok()
            .is_some_and(|locale| has_data(&locale))
}

/// Sunday is 0, from the date itself.
fn weekday_of(year: i32, month: u32, day: u32) -> Option<u32> {
    let date =
        Date::try_new_gregorian(year, u8::try_from(month).ok()?, u8::try_from(day).ok()?).ok()?;
    // ICU numbers Monday 1 through Sunday 7.
    Some(u32::from(date.weekday() as u8 % 7))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Thursday: 2026-09-24.
    const DATE: (i32, u32, u32) = (2026, 9, 24);

    fn month(language: Option<&str>, long: bool) -> String {
        let (year, month, day) = DATE;
        Locale::of(language).month(year, month, day, long)
    }

    fn weekday(language: &str, long: bool) -> String {
        let (year, month, day) = DATE;
        Locale::of(Some(language)).weekday(year, month, day, long)
    }

    #[test]
    fn without_a_language_every_word_is_english() {
        assert_eq!(month(None, true), "September");
        assert_eq!(month(None, false), "Sep");
        assert_eq!(weekday("en", true), "Thursday");
        assert_eq!(weekday("en", false), "Thu");
        assert_eq!(Locale::of(None).day_period(14), "PM");
        assert_eq!(Locale::of(None).day_period(0), "AM");
        assert_eq!(Locale::of(None).number("17.6"), "17.6");
    }

    #[test]
    fn a_tag_this_build_cannot_render_reads_english_rather_than_blank() {
        assert_eq!(month(Some("xx-YY"), true), "September");
        assert_eq!(month(Some("en_US"), true), "September");
        assert_eq!(Locale::of(Some("en_US")).number("17.6"), "17.6");
    }

    #[test]
    fn a_malformed_or_dataless_tag_is_refused_as_a_language() {
        for tag in ["", "en_US", "en-", "123", "xx-YY", "zz"] {
            assert!(!is_language(tag), "{tag:?}");
        }
        for tag in ["en", "fr", "ja-JP", "zh-Hant-TW", "vi"] {
            assert!(is_language(tag), "{tag:?}");
        }
    }

    #[test]
    fn resolves_long_and_short_forms_from_cldr_not_a_truncation() {
        // German's short weekday is `Do`; the long form's first three letters
        // are `Don`. A truncating table gets this wrong, so this pins the data.
        assert_eq!(weekday("de", true), "Donnerstag");
        assert_eq!(weekday("de", false), "Do");

        // Traditional Chinese shortens the other way: `週四` is not a prefix of
        // `星期四` at all.
        assert_eq!(weekday("zh-Hant", true), "星期四");
        assert_eq!(weekday("zh-Hant", false), "週四");
    }

    #[test]
    fn spells_a_month_in_the_chosen_language() {
        assert_eq!(month(Some("en"), true), "September");
        assert_eq!(month(Some("fr"), true), "septembre");
        assert_eq!(month(Some("vi"), true), "Tháng 9");
        assert_eq!(month(Some("pt"), false), "set.");
    }

    #[test]
    fn reads_a_day_period_the_language_actually_has() {
        // Japanese has no AM/PM; it has 午前/午後.
        assert_eq!(Locale::of(Some("ja")).day_period(12), "午後");
        assert_eq!(Locale::of(Some("ja")).day_period(0), "午前");
        assert_eq!(Locale::of(Some("fr")).day_period(12), "PM");
    }

    #[test]
    fn the_reading_takes_the_languages_own_separator() {
        assert_eq!(Locale::of(Some("de")).number("17.6"), "17,6");
        assert_eq!(Locale::of(Some("fr")).number("42.00"), "42,00");
        assert_eq!(Locale::of(Some("en")).number("1.000000"), "1.000000");
    }
}
