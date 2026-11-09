//! Application Fluent catalogs and process-locale display text.

use enzui_core::localization::{CatalogBuilder, Localizer};
use std::collections::BTreeMap;
use std::sync::LazyLock;

static SOURCES: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/locales/sources.json")).unwrap_or_default()
});
pub(crate) fn for_locales(locales: Vec<enzui_core::localization::Locale>) -> Localizer {
    let mut builder = CatalogBuilder::default();
    for (locale, resource) in [
        ("en-US", include_str!("../assets/locales/en-US.ftl")),
        ("de", include_str!("../assets/locales/de.ftl")),
    ] {
        if let Err(error) = builder.add_resource(locale, resource) {
            log::error!("application catalog {locale}: {error}");
        }
    }
    Localizer::new(builder.build(), locales, Default::default())
}

// Domain errors retain canonical source values across persistence and worker boundaries.
pub(crate) fn domain_text(source: &str) -> String {
    SOURCES.get(source).map_or_else(
        || source.to_owned(),
        |key| enzui_core::localization::text(key),
    )
}

pub(crate) fn number(value: impl ToString) -> String {
    let value = value.to_string();
    value
        .parse::<enzui_core::localization::Decimal>()
        .ok()
        .and_then(|decimal| {
            enzui_core::localization::current()
                .locale()
                .format_decimal(&decimal)
                .ok()
        })
        .unwrap_or(value)
}
